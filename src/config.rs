//! The resolved runtime configuration, built from parsed CLI args and shared through
//! [`AppState`](crate::server::AppState) behind an `Arc`.

use std::path::PathBuf;

// Fields are retained for introspection and future handler use; not all are read today.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Config {
    pub file: PathBuf,
    pub lat_col: Option<String>,
    pub lon_col: Option<String>,
    pub table: Option<String>,
    pub port: u16,
    pub bind: String,
    pub watch: bool,
    pub disk: bool,
    /// Which browser origins may call the API.
    pub cors: CorsPolicy,
    /// When set, every endpoint but `/health` needs `Authorization: Bearer <token>`.
    pub token: Option<Token>,
}

/// Which browser origins may call the API.
#[derive(Debug, Clone, PartialEq)]
pub enum CorsPolicy {
    /// Any origin (`Access-Control-Allow-Origin: *`): the default.
    Any,
    /// Only these origins, e.g. `https://maps.example.org`.
    Origins(Vec<String>),
    /// No CORS headers: browsers on other origins can't read responses.
    Disabled,
}

impl CorsPolicy {
    /// Build a policy from `--cors-origin` values: none given means [`CorsPolicy::Any`],
    /// `*` means any, `none` disables CORS, anything else is an origin
    /// (`scheme://host[:port]`, no path), normalized as browsers send it.
    pub fn from_args(values: &[String]) -> Result<Self, String> {
        let values: Vec<&str> = values.iter().map(|v| v.trim()).collect();
        match values.as_slice() {
            [] | ["*"] => Ok(CorsPolicy::Any),
            ["none"] => Ok(CorsPolicy::Disabled),
            _ => {
                let mut origins = Vec::new();
                for v in values {
                    if v == "*" || v == "none" {
                        return Err(format!(
                            "--cors-origin {v} can't be combined with other origins"
                        ));
                    }
                    origins.push(normalize_origin(v).ok_or_else(|| {
                        format!(
                            "invalid --cors-origin '{v}': use scheme://host[:port], e.g. https://maps.example.org"
                        )
                    })?);
                }
                Ok(CorsPolicy::Origins(origins))
            }
        }
    }
}

/// An origin as a browser sends it in `Origin`: scheme and host lowercased, a
/// default port (443 for https, 80 for http) dropped. `None` if it isn't
/// `scheme://host[:port]` in visible ASCII.
fn normalize_origin(v: &str) -> Option<String> {
    let (scheme, rest) = v.split_once("://")?;
    let visible = |s: &str| s.bytes().all(|b| (0x21..=0x7e).contains(&b));
    if scheme.is_empty() || rest.is_empty() || rest.contains('/') || !visible(v) {
        return None;
    }
    let scheme = scheme.to_ascii_lowercase();
    let mut host = rest.to_ascii_lowercase();
    let default_port = match scheme.as_str() {
        "https" => Some(":443"),
        "http" => Some(":80"),
        _ => None,
    };
    if let Some(port) = default_port {
        if let Some(stripped) = host.strip_suffix(port) {
            host = stripped.to_string();
        }
    }
    Some(format!("{scheme}://{host}"))
}

/// A bearer token. Its `Debug` output is redacted, so printing a [`Config`] can't
/// leak it into logs.
#[derive(Clone, PartialEq)]
pub struct Token(String);

impl Token {
    /// `None` for an empty (or all-whitespace) token.
    pub fn new(token: &str) -> Option<Self> {
        let t = token.trim();
        (!t.is_empty()).then(|| Token(t.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn cors_policy_from_args() {
        assert_eq!(CorsPolicy::from_args(&[]), Ok(CorsPolicy::Any));
        assert_eq!(CorsPolicy::from_args(&args(&["*"])), Ok(CorsPolicy::Any));
        assert_eq!(
            CorsPolicy::from_args(&args(&["none"])),
            Ok(CorsPolicy::Disabled)
        );
        assert_eq!(
            CorsPolicy::from_args(&args(&["https://a.example", "http://localhost:3000"])),
            Ok(CorsPolicy::Origins(args(&[
                "https://a.example",
                "http://localhost:3000"
            ])))
        );
    }

    #[test]
    fn cors_policy_rejects_bad_origins() {
        for bad in [
            &["a.example"][..],
            &["https://a.example/path"],
            &["https://"],
            &["*", "https://a.example"],
            &["none", "https://a.example"],
        ] {
            assert!(CorsPolicy::from_args(&args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn origins_are_normalized_as_browsers_send_them() {
        assert_eq!(
            CorsPolicy::from_args(&args(&["HTTPS://Maps.Example.org:443", "http://h:8080"])),
            Ok(CorsPolicy::Origins(args(&[
                "https://maps.example.org",
                "http://h:8080"
            ])))
        );
        assert_eq!(normalize_origin("http://h:80").as_deref(), Some("http://h"));
    }

    #[test]
    fn token_is_redacted_and_never_empty() {
        let t = Token::new(" s3cret ").unwrap();
        assert_eq!(t.as_str(), "s3cret");
        assert_eq!(format!("{t:?}"), "Token(<redacted>)");
        assert!(Token::new("  ").is_none());
    }
}
