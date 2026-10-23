use axum::http::{HeaderMap, HeaderValue, header};
use cookie::{Cookie, SameSite};

pub const SESSION_COOKIE: &str = "pf_session";
pub const CSRF_COOKIE: &str = "pf_csrf";

pub fn read(headers: &HeaderMap, name: &str) -> Option<String> {
    for header in headers.get_all(header::COOKIE) {
        let Ok(value) = header.to_str() else {
            continue;
        };
        for part in value.split(';') {
            let Ok(cookie) = Cookie::parse(part.trim().to_owned()) else {
                continue;
            };
            if cookie.name() == name {
                return Some(cookie.value().to_owned());
            }
        }
    }
    None
}

pub fn session_cookie(token: &str, secure: bool, ttl_seconds: i64) -> String {
    build_cookie(SESSION_COOKIE, token, secure, true, ttl_seconds)
}

pub fn csrf_cookie(token: &str, secure: bool, ttl_seconds: i64) -> String {
    // The CSRF token is intentionally readable by the SPA so it can echo it in
    // the X-CSRF-Token header. It is not an authentication credential.
    build_cookie(CSRF_COOKIE, token, secure, false, ttl_seconds)
}

pub fn clear_session_cookie(secure: bool) -> String {
    clear_cookie(SESSION_COOKIE, secure, true)
}

pub fn clear_csrf_cookie(secure: bool) -> String {
    clear_cookie(CSRF_COOKIE, secure, false)
}

pub fn append(headers: &mut HeaderMap, cookie: String) {
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        headers.append(header::SET_COOKIE, value);
    }
}

fn build_cookie(
    name: &'static str,
    value: &str,
    secure: bool,
    http_only: bool,
    ttl: i64,
) -> String {
    let mut cookie = Cookie::new(name, value.to_owned());
    cookie.set_path("/");
    cookie.set_secure(secure);
    cookie.set_http_only(http_only);
    cookie.set_same_site(SameSite::Strict);
    cookie.set_max_age(cookie::time::Duration::seconds(ttl.max(1)));
    cookie.to_string()
}

fn clear_cookie(name: &'static str, secure: bool, http_only: bool) -> String {
    let mut cookie = Cookie::new(name, String::new());
    cookie.set_path("/");
    cookie.set_secure(secure);
    cookie.set_http_only(http_only);
    cookie.set_same_site(SameSite::Strict);
    cookie.set_max_age(cookie::time::Duration::ZERO);
    cookie.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_cookie_is_hardened() {
        let cookie = session_cookie("token", true, 60);
        assert!(cookie.contains("pf_session=token"));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("Secure"));
        assert!(cookie.contains("SameSite=Strict"));
        assert!(cookie.contains("Path=/"));
    }

    #[test]
    fn csrf_cookie_is_readable_but_hardened() {
        let cookie = csrf_cookie("token", true, 60);
        assert!(cookie.contains("pf_csrf=token"));
        assert!(!cookie.contains("HttpOnly"));
        assert!(cookie.contains("Secure"));
        assert!(cookie.contains("SameSite=Strict"));
    }

    #[test]
    fn reads_named_cookie_from_multiple_headers() {
        let mut headers = HeaderMap::new();
        headers.append(header::COOKIE, HeaderValue::from_static("a=1; pf_csrf=abc"));
        headers.append(header::COOKIE, HeaderValue::from_static("pf_session=xyz"));
        assert_eq!(read(&headers, CSRF_COOKIE).as_deref(), Some("abc"));
        assert_eq!(read(&headers, SESSION_COOKIE).as_deref(), Some("xyz"));
        assert_eq!(read(&headers, "missing"), None);
    }
}
