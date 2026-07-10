//! A normalized credential and the JSON template used to create it in 1Password.

use crate::chrome::CsvRow;
use serde_json::json;

/// A credential ready to be matched against 1Password and, if missing, created.
#[derive(Debug, Clone)]
pub struct Credential {
    pub title: String,
    pub url: String,
    pub username: String,
    pub password: String,
    pub note: String,
}

impl From<CsvRow> for Credential {
    fn from(r: CsvRow) -> Self {
        let url = r.url.trim().to_string();
        let title = if !r.name.trim().is_empty() {
            r.name.trim().to_string()
        } else {
            host_of(&url).unwrap_or_else(|| "Untitled Login".to_string())
        };
        Credential {
            title,
            url,
            username: r.username.trim().to_string(),
            password: r.password,
            note: r.note.trim().to_string(),
        }
    }
}

impl Credential {
    /// The normalized host used for matching (lowercased, `www.` stripped).
    pub fn host(&self) -> Option<String> {
        host_of(&self.url)
    }

    /// Stable identity for de-duplicating CSV rows: (host, lowercased username).
    pub fn identity(&self) -> (String, String) {
        (
            self.host().unwrap_or_default(),
            self.username.to_lowercase(),
        )
    }

    /// The op JSON template to create this login (piped to `op item create -`).
    pub fn create_template(&self) -> String {
        let mut fields = vec![
            json!({
                "id": "username",
                "type": "STRING",
                "purpose": "USERNAME",
                "label": "username",
                "value": self.username,
            }),
            json!({
                "id": "password",
                "type": "CONCEALED",
                "purpose": "PASSWORD",
                "label": "password",
                "value": self.password,
            }),
        ];
        if !self.note.is_empty() {
            fields.push(json!({
                "id": "notesPlain",
                "type": "STRING",
                "purpose": "NOTES",
                "label": "notesPlain",
                "value": self.note,
            }));
        }

        let mut item = json!({
            "title": self.title,
            "category": "LOGIN",
            "fields": fields,
        });
        if !self.url.is_empty() {
            item["urls"] = json!([{ "label": "website", "primary": true, "href": self.url }]);
        }
        item.to_string()
    }
}

/// Extract a normalized host from a URL string: lowercase, `www.` stripped.
/// Falls back to `None` for values without a recognizable host.
pub fn host_of(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    // url::Url needs a scheme; add one if missing (Chrome sometimes exports
    // bare hosts or android:// URIs).
    let parsed = url::Url::parse(raw)
        .or_else(|_| url::Url::parse(&format!("https://{raw}")))
        .ok()?;
    let host = parsed.host_str()?.to_lowercase();
    Some(host.strip_prefix("www.").unwrap_or(&host).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_hosts() {
        assert_eq!(
            host_of("https://www.Example.com/login").as_deref(),
            Some("example.com")
        );
        assert_eq!(host_of("http://github.com").as_deref(), Some("github.com"));
        assert_eq!(host_of("example.org").as_deref(), Some("example.org"));
        assert_eq!(host_of("").as_deref(), None);
    }

    #[test]
    fn template_includes_url_and_omits_empty_note() {
        let c = Credential {
            title: "Example".into(),
            url: "https://example.com".into(),
            username: "jane".into(),
            password: "pw".into(),
            note: "".into(),
        };
        let t = c.create_template();
        assert!(t.contains("\"category\":\"LOGIN\""));
        assert!(t.contains("example.com"));
        assert!(!t.contains("notesPlain"));
    }

    #[test]
    fn identity_is_host_and_lower_username() {
        let c = Credential {
            title: "X".into(),
            url: "https://WWW.Site.com".into(),
            username: "Jane@Example.com".into(),
            password: "p".into(),
            note: "".into(),
        };
        assert_eq!(
            c.identity(),
            ("site.com".to_string(), "jane@example.com".to_string())
        );
    }
}
