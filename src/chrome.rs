//! Parsing of Google/Chrome Password Manager CSV exports.
//!
//! The export has the header `name,url,username,password,note` (the `note`
//! column is sometimes absent). We tolerate missing columns and blank rows.

use crate::error::{Error, Result};
use std::collections::HashMap;
use std::path::Path;

/// One row of the Chrome/Google export.
#[derive(Debug, Clone, Default)]
pub struct CsvRow {
    pub name: String,
    pub url: String,
    pub username: String,
    pub password: String,
    pub note: String,
}

impl CsvRow {
    /// A row is worth importing if it has at least a URL or a username.
    pub fn is_meaningful(&self) -> bool {
        !self.url.trim().is_empty() || !self.username.trim().is_empty()
    }
}

/// A parsed export: the rows worth importing, and how many were dropped.
#[derive(Debug)]
pub struct Export {
    pub rows: Vec<CsvRow>,
    /// Rows with neither a URL nor a username. They are not synced, though
    /// one may still carry a password.
    pub unreadable: usize,
}

/// Read and parse a Chrome/Google CSV export.
///
/// Columns are located by header name (case-insensitive), so column order and
/// short rows are both tolerated. The expected headers are
/// `name,url,username,password,note`.
pub fn read_csv(path: &Path) -> Result<Export> {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .has_headers(true)
        .from_path(path)
        .map_err(|e| Error::Csv(format!("{}: {e}", path.display())))?;

    let headers = reader
        .headers()
        .map_err(|e| Error::Csv(format!("reading header: {e}")))?
        .clone();
    let mut idx: HashMap<String, usize> = HashMap::new();
    for (i, h) in headers.iter().enumerate() {
        idx.insert(h.trim().to_lowercase(), i);
    }
    let col = |rec: &csv::StringRecord, keys: &[&str]| -> String {
        for k in keys {
            if let Some(&i) = idx.get(*k) {
                if let Some(v) = rec.get(i) {
                    return v.to_string();
                }
            }
        }
        String::new()
    };

    let mut rows = Vec::new();
    let mut unreadable = 0;
    for (i, rec) in reader.records().enumerate() {
        let rec = rec.map_err(|e| Error::Csv(format!("row {}: {e}", i + 1)))?;
        let row = CsvRow {
            name: col(&rec, &["name"]),
            url: col(&rec, &["url"]),
            username: col(&rec, &["username"]),
            password: col(&rec, &["password"]),
            note: col(&rec, &["note", "notes"]),
        };
        if row.is_meaningful() {
            rows.push(row);
        } else {
            unreadable += 1;
        }
    }
    Ok(Export { rows, unreadable })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_tmp(contents: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        f.flush().unwrap();
        f
    }

    #[test]
    fn parses_standard_export() {
        let f = write_tmp(
            "name,url,username,password,note\n\
             Example,https://example.com/login,jane@example.com,hunter2,my note\n\
             Empty,,,,\n\
             GitHub,https://github.com,jane,pw\n",
        );
        let export = read_csv(f.path()).unwrap();
        // The row with no URL and no username is dropped, and counted.
        assert_eq!(export.unreadable, 1);
        let rows = export.rows;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "Example");
        assert_eq!(rows[0].username, "jane@example.com");
        assert_eq!(rows[0].note, "my note");
        // Missing note column on the last row is fine.
        assert_eq!(rows[1].note, "");
    }

    #[test]
    fn tolerates_missing_note_column() {
        let f = write_tmp("name,url,username,password\nX,https://x.com,u,p\n");
        let rows = read_csv(f.path()).unwrap().rows;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].password, "p");
    }
}
