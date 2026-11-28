//! Parsing of password-manager CSV exports: Google / Chrome Password Manager
//! and Apple Passwords / Safari.
//!
//! The format is decided by the header alone:
//!
//! - Chrome / Google: `name,url,username,password,note` (`note` is sometimes
//!   absent).
//! - Apple Passwords and Safari: `Title,URL,Username,Password,Notes,OTPAuth`.
//!
//! Columns are located by header name (case-insensitive), so column order and
//! short rows are tolerated. A header that is neither is refused before any
//! row is read, and the error never repeats the header: a file without one
//! has a credential in its first line.

use crate::error::{Error, Result};
use std::collections::HashMap;
use std::path::Path;

/// Which password manager wrote an export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Format {
    Chrome,
    Apple,
}

impl Format {
    /// The value in `--json` output.
    pub fn id(self) -> &'static str {
        match self {
            Format::Chrome => "chrome",
            Format::Apple => "apple",
        }
    }

    /// The name in human output.
    pub fn label(self) -> &'static str {
        match self {
            Format::Chrome => "Chrome / Google Password Manager",
            Format::Apple => "Apple Passwords / Safari",
        }
    }

    /// The password manager that still holds every row once the export is
    /// deleted.
    pub fn keeper(self) -> &'static str {
        match self {
            Format::Chrome => "Google Password Manager",
            Format::Apple => "Apple Passwords",
        }
    }

    /// Decide the format from a header's lowercased, trimmed column names.
    fn detect(columns: &[String]) -> Option<Format> {
        let has = |name: &str| columns.iter().any(|c| c == name);
        let common = has("url") && has("username") && has("password");
        match (common && has("name"), common && has("title")) {
            (true, false) => Some(Format::Chrome),
            (false, true) => Some(Format::Apple),
            _ => None,
        }
    }
}

/// One row of an export.
#[derive(Debug, Clone, Default)]
pub struct CsvRow {
    /// The row's number as a spreadsheet shows it: the header is row 1.
    pub row: usize,
    pub name: String,
    pub url: String,
    pub username: String,
    pub password: String,
    pub note: String,
    /// Apple's `OTPAuth` column: an `otpauth://` URI, or empty.
    pub otp: String,
}

impl CsvRow {
    /// A row is worth importing if it has at least a URL or a username.
    pub fn is_meaningful(&self) -> bool {
        !self.url.trim().is_empty() || !self.username.trim().is_empty()
    }
}

/// A parsed export: the rows worth importing, and the rows dropped.
#[derive(Debug)]
pub struct Export {
    pub format: Format,
    pub rows: Vec<CsvRow>,
    /// Row numbers of rows with neither a URL nor a username. They are not
    /// synced, though one may still carry a password.
    pub unreadable: Vec<usize>,
}

/// Read and parse an export, deciding its format from the header.
pub fn read_csv(path: &Path) -> Result<Export> {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .has_headers(true)
        .from_path(path)
        .map_err(|e| Error::Csv(format!("{}: {e}", path.display())))?;

    let headers = reader
        .headers()
        .map_err(|e| Error::Csv(format!("{}: reading the header: {e}", path.display())))?
        .clone();
    let columns: Vec<String> = headers
        .iter()
        .map(|h| h.trim_start_matches('\u{feff}').trim().to_lowercase())
        .collect();
    let format = Format::detect(&columns).ok_or_else(|| {
        Error::Csv(format!(
            "{}: not a password export gpm2op reads. Its first line ({} column(s)) is neither \
             Chrome / Google Password Manager's header (name,url,username,password,note) nor \
             Apple Passwords / Safari's (Title,URL,Username,Password,Notes,OTPAuth).",
            path.display(),
            columns.len()
        ))
    })?;
    let idx: HashMap<&str, usize> = columns
        .iter()
        .enumerate()
        .map(|(i, c)| (c.as_str(), i))
        .collect();
    let col = |rec: &csv::StringRecord, keys: &[&str]| -> String {
        keys.iter()
            .find_map(|k| idx.get(k).and_then(|&i| rec.get(i)))
            .unwrap_or("")
            .to_string()
    };

    let mut rows = Vec::new();
    let mut unreadable = Vec::new();
    for (i, rec) in reader.records().enumerate() {
        let row = i + 2;
        let rec = rec.map_err(|e| Error::Csv(format!("{}: row {row}: {e}", path.display())))?;
        let row = CsvRow {
            row,
            name: col(&rec, &["name", "title"]),
            url: col(&rec, &["url"]),
            username: col(&rec, &["username"]),
            password: col(&rec, &["password"]),
            note: col(&rec, &["note", "notes"]),
            otp: col(&rec, &["otpauth"]),
        };
        if row.is_meaningful() {
            rows.push(row);
        } else {
            unreadable.push(row.row);
        }
    }
    Ok(Export {
        format,
        rows,
        unreadable,
    })
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
        assert_eq!(export.format, Format::Chrome);
        // The row with no URL and no username is dropped, and its row named.
        assert_eq!(export.unreadable, vec![3]);
        let rows = export.rows;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].row, 2);
        assert_eq!(rows[0].name, "Example");
        assert_eq!(rows[0].username, "jane@example.com");
        assert_eq!(rows[0].note, "my note");
        // Missing note column on the last row is fine.
        assert_eq!(rows[1].note, "");
        assert_eq!(rows[1].row, 4);
    }

    #[test]
    fn tolerates_missing_note_column() {
        let f = write_tmp("name,url,username,password\nX,https://x.com,u,p\n");
        let rows = read_csv(f.path()).unwrap().rows;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].password, "p");
    }

    #[test]
    fn parses_an_apple_passwords_export() {
        let f = write_tmp(
            "\u{feff}Title,URL,Username,Password,Notes,OTPAuth\n\
             example.com (jane),https://example.com/,jane,pw,a note,otpauth://totp/x?secret=AAAA\n\
             example.org,https://example.org/,jane,,,\n",
        );
        let export = read_csv(f.path()).unwrap();
        assert_eq!(export.format, Format::Apple);
        let rows = export.rows;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "example.com (jane)");
        assert_eq!(rows[0].note, "a note");
        assert!(rows[0].otp.starts_with("otpauth://"));
        // An empty password is kept, not dropped.
        assert_eq!(rows[1].password, "");
        assert_eq!(rows[1].row, 3);
    }

    #[test]
    fn a_header_that_is_neither_is_refused_without_repeating_it() {
        let f = write_tmp("date,amount,memo\n2026-01-01,12.00,coffee\n");
        let err = read_csv(f.path()).unwrap_err().to_string();
        assert!(err.contains("not a password export"), "{err}");
        assert!(err.contains("3 column(s)"), "{err}");
        assert!(!err.contains("memo"), "{err}");

        // A file with no header has a credential where the header should be.
        let f = write_tmp("Example,https://example.com/,jane,hunter2,\n");
        let err = read_csv(f.path()).unwrap_err().to_string();
        assert!(!err.contains("hunter2"), "{err}");
        assert!(!err.contains("example.com/"), "{err}");
    }

    #[test]
    fn a_header_with_both_name_and_title_is_ambiguous() {
        let cols: Vec<String> = ["name", "title", "url", "username", "password"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(Format::detect(&cols), None);
    }
}
