//! A normalized credential and the JSON template used to create it in 1Password.

use crate::source::CsvRow;
use serde_json::json;

/// Where a credential came from: the export's position among the files of the
/// run (0-based) and the row number in it (the header is row 1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Origin {
    pub file: usize,
    pub row: usize,
}

/// A credential ready to be matched against 1Password and, if missing, created.
#[derive(Debug, Clone, Default)]
pub struct Credential {
    pub origin: Origin,
    pub title: String,
    pub url: String,
    pub username: String,
    /// Empty when the export had none, such as a Chrome row for a sign-in
    /// through another site. The login is created without a password field.
    pub password: String,
    pub note: String,
    /// An `otpauth://` URI from Apple's `OTPAuth` column.
    pub otp: Option<String>,
}

/// Whether an `OTPAuth` value is one 1Password can store as a one-time
/// password.
pub fn is_otpauth(raw: &str) -> bool {
    raw.trim()
        .get(..10)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("otpauth://"))
}

impl Credential {
    /// Build a credential from a row of the export at position `file`.
    pub fn from_row(file: usize, r: CsvRow) -> Self {
        let url = r.url.trim().to_string();
        let title = if !r.name.trim().is_empty() {
            r.name.trim().to_string()
        } else {
            host_of(&url).unwrap_or_else(|| "Untitled Login".to_string())
        };
        let otp = r.otp.trim();
        Credential {
            origin: Origin { file, row: r.row },
            title,
            url,
            username: r.username.trim().to_string(),
            password: r.password,
            note: r.note.trim().to_string(),
            otp: is_otpauth(otp).then(|| otp.to_string()),
        }
    }

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

    /// Identity for keeping every password: the site, the lowercased
    /// username, the password and the one-time-password secret. Two rows
    /// with the same identity are exact repeats.
    pub fn secret_identity(&self) -> (String, String, String, String) {
        let (host, username) = self.identity();
        (
            host,
            username,
            self.password.clone(),
            self.otp.clone().unwrap_or_default(),
        )
    }

    /// The op JSON template to create this login (piped to `op item create -`).
    pub fn create_template(&self) -> String {
        let mut fields = vec![json!({
            "id": "username",
            "type": "STRING",
            "purpose": "USERNAME",
            "label": "username",
            "value": self.username,
        })];
        if !self.password.is_empty() {
            fields.push(json!({
                "id": "password",
                "type": "CONCEALED",
                "purpose": "PASSWORD",
                "label": "password",
                "value": self.password,
            }));
        }
        if let Some(otp) = &self.otp {
            fields.push(json!({
                "type": "OTP",
                "label": "one-time password",
                "value": otp,
            }));
        }
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
            ..Credential::default()
        };
        let t = c.create_template();
        assert!(t.contains("\"category\":\"LOGIN\""));
        assert!(t.contains("example.com"));
        assert!(!t.contains("notesPlain"));
        assert!(!t.contains("\"OTP\""));
    }

    #[test]
    fn an_empty_password_gets_no_password_field() {
        let c = Credential {
            title: "Example".into(),
            url: "https://example.com".into(),
            username: "jane".into(),
            ..Credential::default()
        };
        let t: serde_json::Value = serde_json::from_str(&c.create_template()).unwrap();
        let purposes: Vec<_> = t["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["purpose"].as_str().unwrap_or(""))
            .collect();
        assert_eq!(purposes, vec!["USERNAME"]);
    }

    #[test]
    fn an_otpauth_value_becomes_an_otp_field_and_anything_else_is_dropped() {
        let row = |otp: &str| CsvRow {
            row: 2,
            url: "https://example.com".into(),
            username: "jane".into(),
            otp: otp.into(),
            ..CsvRow::default()
        };
        let c = Credential::from_row(0, row(" OTPAUTH://totp/x?secret=AAAA "));
        let t: serde_json::Value = serde_json::from_str(&c.create_template()).unwrap();
        let otp = t["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["type"] == "OTP")
            .unwrap();
        assert_eq!(otp["value"], "OTPAUTH://totp/x?secret=AAAA");
        assert!(Credential::from_row(0, row("AAAA")).otp.is_none());
        assert!(Credential::from_row(0, row("")).otp.is_none());
        assert_eq!(c.origin, Origin { file: 0, row: 2 });
    }

    #[test]
    fn identity_is_host_and_lower_username() {
        let c = Credential {
            title: "X".into(),
            url: "https://WWW.Site.com".into(),
            username: "Jane@Example.com".into(),
            password: "p".into(),
            ..Credential::default()
        };
        assert_eq!(
            c.identity(),
            ("site.com".to_string(), "jane@example.com".to_string())
        );
    }
}
