//! CORS and bearer-token authentication, applied as layers in
//! [`build_router`](crate::server::build_router). Request tracing is a tower-http
//! layer there too.

use crate::config::CorsPolicy;
use crate::error::AppError;
use crate::server::AppState;
use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, Method};
use axum::middleware::Next;
use axum::response::Response;
use tower_http::cors::{AllowHeaders, AllowOrigin, Any, CorsLayer};

/// The CORS layer for `policy`, or `None` to add no layer at all. Preflight
/// `OPTIONS` requests are answered here, outside the token check: browsers send
/// them without credentials.
pub fn cors_layer(policy: &CorsPolicy) -> Option<CorsLayer> {
    match policy {
        // Like `CorsLayer::permissive()`, but the allowed request headers mirror the
        // preflight's: browsers don't let `Access-Control-Allow-Headers: *` cover
        // `Authorization`, which a token needs.
        CorsPolicy::Any => Some(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(AllowHeaders::mirror_request())
                .expose_headers(Any),
        ),
        CorsPolicy::Disabled => None,
        CorsPolicy::Origins(origins) => Some(
            CorsLayer::new()
                .allow_origin(AllowOrigin::list(
                    // Validated in `CorsPolicy::from_args`.
                    origins.iter().filter_map(|o| HeaderValue::from_str(o).ok()),
                ))
                .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
                .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
                .expose_headers([header::LINK]),
        ),
    }
}

/// Require `Authorization: Bearer <token>` when the server has a token. `/health`
/// stays open for load balancers and container health checks.
pub async fn require_token(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let Some(token) = state.config.token.as_ref() else {
        return Ok(next.run(req).await);
    };
    if req.uri().path() == "/health" {
        return Ok(next.run(req).await);
    }
    let presented = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(bearer_credentials);
    match presented {
        Some(p) if constant_time_eq(p.as_bytes(), token.as_str().as_bytes()) => {
            Ok(next.run(req).await)
        }
        _ => Err(AppError::Unauthorized),
    }
}

/// The credentials in an `Authorization: Bearer <token>` value. The scheme is
/// case-insensitive (RFC 7235), with any whitespace after it.
fn bearer_credentials(value: &str) -> Option<&str> {
    let (scheme, rest) = value.trim().split_once(char::is_whitespace)?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| rest.trim())
        .filter(|t| !t.is_empty())
}

/// Compare without stopping at the first differing byte, so the time taken doesn't
/// reveal how much of a guess was right. (The length can still show.)
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The warning for serving on `bind` without a token, or `None` when that's safe
/// (a token is set, or `bind` is loopback). Shared by the CLI and the Python
/// bindings so both say the same thing.
pub fn public_bind_warning(bind: &str, has_token: bool) -> Option<String> {
    (!has_token && !is_loopback(bind)).then(|| {
        format!(
            "serving on {bind} without a token: anyone who can reach this port can read the data"
        )
    })
}

/// Whether `bind` only reaches this machine: every address it resolves to is a
/// loopback address. Unresolvable counts as not loopback.
pub fn is_loopback(bind: &str) -> bool {
    use std::net::ToSocketAddrs;
    match (bind, 0).to_socket_addrs() {
        Ok(addrs) => {
            let addrs: Vec<_> = addrs.collect();
            !addrs.is_empty() && addrs.iter().all(|a| a.ip().is_loopback())
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_matches_only_equal_bytes() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secreT"));
        assert!(!constant_time_eq(b"secret", b"secret2"));
        assert!(!constant_time_eq(b"", b"x"));
    }

    #[test]
    fn bearer_scheme_is_case_insensitive() {
        assert_eq!(bearer_credentials("Bearer abc"), Some("abc"));
        assert_eq!(bearer_credentials("bearer\tabc "), Some("abc"));
        assert_eq!(bearer_credentials("BEARER  abc"), Some("abc"));
        assert_eq!(bearer_credentials("Basic abc"), None);
        assert_eq!(bearer_credentials("Bearer"), None);
        assert_eq!(bearer_credentials("Bearer   "), None);
    }

    #[test]
    fn public_bind_warning_only_without_a_token_off_loopback() {
        assert!(public_bind_warning("0.0.0.0", false).is_some());
        assert!(public_bind_warning("0.0.0.0", true).is_none());
        assert!(public_bind_warning("127.0.0.1", false).is_none());
    }

    #[test]
    fn loopback_addresses() {
        assert!(is_loopback("127.0.0.1"));
        assert!(is_loopback("::1"));
        assert!(is_loopback("localhost"));
        assert!(is_loopback("127.0.0.2"));
        assert!(!is_loopback("0.0.0.0"));
        assert!(!is_loopback("192.0.2.10"));
    }
}
