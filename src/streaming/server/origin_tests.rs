use super::http::CONTENT_SECURITY_POLICY;
use super::{
    build_cors_layer, check_ws_origin, is_local_origin, sessions_handler, StreamingConfig,
    StreamingServer,
};
use crate::terminal::Terminal;
use parking_lot::RwLock;
use std::sync::Arc;
use tower::ServiceExt; // .oneshot for in-process router tests

#[test]
fn no_origin_is_allowed_non_browser() {
    assert!(check_ws_origin(None, None));
    assert!(check_ws_origin(
        None,
        Some(&["https://app.example.com".to_string()][..])
    ));
}

#[test]
fn default_rejects_remote_browser_origin() {
    // CSRF-via-WebSocket: a remote page must be blocked when no allowlist is set.
    assert!(!check_ws_origin(Some("https://evil.com"), None));
    assert!(!check_ws_origin(Some("http://attacker.example:8080"), None));
}

#[test]
fn default_allows_local_browser_origin() {
    assert!(check_ws_origin(Some("http://localhost:8099"), None));
    assert!(check_ws_origin(Some("https://127.0.0.1:8099"), None));
    assert!(check_ws_origin(Some("http://[::1]:8099"), None));
}

#[test]
fn allowlist_enforced_when_configured() {
    let list = vec!["https://app.example.com".to_string()];
    let allowed: &[String] = &list;
    assert!(check_ws_origin(
        Some("https://app.example.com"),
        Some(allowed)
    ));
    assert!(!check_ws_origin(Some("https://evil.com"), Some(allowed)));
    // Even a local origin is rejected if it's not in the explicit allowlist.
    assert!(!check_ws_origin(
        Some("http://localhost:8099"),
        Some(allowed)
    ));
}

#[test]
fn is_local_origin_host_extraction() {
    assert!(is_local_origin("http://localhost:8099"));
    assert!(is_local_origin("https://127.0.0.1:443"));
    assert!(is_local_origin("http://[::1]:8099"));
    assert!(!is_local_origin("http://localhost.evil.com:8099"));
    assert!(!is_local_origin("https://example.com"));
    // A look-alike host must not match.
    assert!(!is_local_origin("http://127.0.0.1.evil.com"));
}

fn origin_request(
    method: &str,
    uri: &str,
    origin: Option<&str>,
) -> axum::http::Request<axum::body::Body> {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    if let Some(origin) = origin {
        builder = builder.header("origin", origin);
    }
    builder.body(axum::body::Body::empty()).unwrap()
}

#[tokio::test]
async fn cors_without_allowlist_mirrors_ws_local_origin_default() {
    use axum::{routing::get, Router};

    let app = Router::new()
        .route("/ok", get(|| async { "ok" }))
        .layer(build_cors_layer(&None));

    // Remote origin: no Access-Control-Allow-Origin header, so a browser
    // blocks the cross-origin read.
    let res = app
        .clone()
        .oneshot(origin_request("GET", "/ok", Some("https://evil.example")))
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    assert!(!res.headers().contains_key("access-control-allow-origin"));

    // Local origin: allowed and echoed back.
    let res = app
        .clone()
        .oneshot(origin_request("GET", "/ok", Some("http://127.0.0.1:8099")))
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    assert_eq!(
        res.headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some("http://127.0.0.1:8099")
    );
}

#[tokio::test]
async fn cors_with_allowlist_passes_listed_origin_only() {
    use axum::{routing::get, Router};

    let allowed = Some(vec!["https://app.example.com".to_string()]);
    let app = Router::new()
        .route("/ok", get(|| async { "ok" }))
        .layer(build_cors_layer(&allowed));

    let res = app
        .clone()
        .oneshot(origin_request(
            "GET",
            "/ok",
            Some("https://app.example.com"),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    assert_eq!(
        res.headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some("https://app.example.com")
    );

    // A local origin is not in the explicit allowlist → no ACAO, same
    // exact-match semantics as check_ws_origin.
    let res = app
        .oneshot(origin_request("GET", "/ok", Some("http://localhost:8099")))
        .await
        .unwrap();
    assert!(!res.headers().contains_key("access-control-allow-origin"));
}

#[tokio::test]
async fn sessions_endpoint_rejects_disallowed_origin() {
    use axum::{routing::get, Router};

    let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
    let server = Arc::new(StreamingServer::new(terminal, "127.0.0.1:0".to_string()));
    let app = Router::new()
        .route("/sessions", get(sessions_handler))
        .with_state(server);

    // Cross-origin browser request → 403.
    let res = app
        .clone()
        .oneshot(origin_request(
            "GET",
            "/sessions",
            Some("https://evil.example"),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), 403);

    // Local origin → 200 with the session list.
    let res = app
        .clone()
        .oneshot(origin_request(
            "GET",
            "/sessions",
            Some("http://localhost:8099"),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body = axum::body::to_bytes(res.into_body(), 1 << 16)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&body).contains("\"sessions\""));

    // No Origin header (non-browser client such as curl) → 200.
    let res = app
        .oneshot(origin_request("GET", "/sessions", None))
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn sessions_endpoint_allows_configured_origin() {
    use axum::{routing::get, Router};

    let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
    let config = StreamingConfig {
        allowed_origins: Some(vec!["https://app.example.com".to_string()]),
        ..StreamingConfig::default()
    };
    let server = Arc::new(StreamingServer::with_config(
        terminal,
        "127.0.0.1:0".to_string(),
        config,
    ));
    let app = Router::new()
        .route("/sessions", get(sessions_handler))
        .with_state(server);

    let res = app
        .clone()
        .oneshot(origin_request(
            "GET",
            "/sessions",
            Some("https://app.example.com"),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), 200);

    // An origin not on the allowlist is still rejected.
    let res = app
        .oneshot(origin_request(
            "GET",
            "/sessions",
            Some("https://evil.example"),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), 403);
}

/// SEC-010: every response from the HTTP app — static frontend files
/// (ServeDir fallback) and API routes alike — must carry the
/// anti-framing headers, or a transparent iframe plus cached Basic
/// credentials can clickjack keystrokes into the live shell.
#[tokio::test]
async fn http_app_responses_carry_anti_framing_headers() {
    // `TempDir` (OS-random name, `Drop`-based cleanup) replaces the
    // pid-derived path: a recycled pid could collide across `cargo
    // test` invocations, and the trailing `remove_dir_all` never ran on
    // a failed assertion.
    let web_root = tempfile::Builder::new()
        .prefix("par-term-webroot-headers-")
        .tempdir()
        .expect("create temp dir for web root fixture");
    std::fs::write(web_root.path().join("index.html"), "<html></html>").unwrap();

    let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
    let config = StreamingConfig {
        web_root: web_root.path().to_string_lossy().into_owned(),
        ..StreamingConfig::default()
    };
    let server = Arc::new(StreamingServer::with_config(
        terminal,
        "127.0.0.1:0".to_string(),
        config,
    ));
    let app = server.build_http_app();

    // Static frontend file via the ServeDir fallback.
    let res = app
        .clone()
        .oneshot(origin_request("GET", "/", None))
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    assert_eq!(
        res.headers()
            .get("x-frame-options")
            .and_then(|v| v.to_str().ok()),
        Some("DENY")
    );
    assert_eq!(
        res.headers()
            .get("content-security-policy")
            .and_then(|v| v.to_str().ok()),
        Some(CONTENT_SECURITY_POLICY)
    );
    assert!(
        res.headers()
            .get("content-security-policy")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|csp| csp.contains("frame-ancestors 'none'")),
        "SEC-010's anti-framing guarantee must survive the full policy"
    );

    // API route response too.
    let res = app
        .oneshot(origin_request("GET", "/sessions", None))
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    assert_eq!(
        res.headers()
            .get("x-frame-options")
            .and_then(|v| v.to_str().ok()),
        Some("DENY")
    );
    assert_eq!(
        res.headers()
            .get("content-security-policy")
            .and_then(|v| v.to_str().ok()),
        Some(CONTENT_SECURITY_POLICY)
    );
}
/// ENH-059: the full policy carries the directives the page needs, and
/// never the unscoped eval.
#[tokio::test]
async fn http_app_csp_restricts_scripts_connections_and_objects() {
    let web_root = tempfile::Builder::new()
        .prefix("par-term-webroot-csp-")
        .tempdir()
        .expect("create temp dir for web root fixture");
    std::fs::write(web_root.path().join("index.html"), "<html></html>").unwrap();

    let terminal = Arc::new(RwLock::new(Terminal::new(80, 24)));
    let config = StreamingConfig {
        web_root: web_root.path().to_string_lossy().into_owned(),
        ..StreamingConfig::default()
    };
    let server = Arc::new(StreamingServer::with_config(
        terminal,
        "127.0.0.1:0".to_string(),
        config,
    ));
    let app = server.build_http_app();

    let res = app.oneshot(origin_request("GET", "/", None)).await.unwrap();
    assert_eq!(res.status(), 200);
    let csp = res
        .headers()
        .get("content-security-policy")
        .and_then(|v| v.to_str().ok())
        .expect("the CSP header");
    for needle in [
        "default-src 'self'",
        "connect-src 'self' ws: wss:",
        "object-src 'none'",
        "base-uri 'self'",
        "script-src 'self'",
    ] {
        assert!(csp.contains(needle), "CSP must contain {needle}: {csp}");
    }
    assert!(
        !csp.contains("'unsafe-eval'"),
        "plain 'unsafe-eval' must not appear (the WASM carve-out is scoped): {csp}"
    );
}

/// ENH-059 drift guard: every external origin the shipped `web_term/`
/// export actually loads must appear in CONTENT_SECURITY_POLICY. A
/// frontend change that adds one fails here until the constant is
/// extended in the same change.
#[test]
fn csp_allows_every_external_origin_the_shipped_frontend_loads() {
    let web = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web_term");
    if !web.is_dir() {
        // The static export is a build artifact; the guard runs where it
        // exists.
        return;
    }
    // `href=`/`src=` attributes in the HTML and `url()` forms in the CSS.
    // Preconnect links are navigation hints CSP does not govern, so their
    // lines are skipped; `https://` tokens anywhere else are not scanned.
    let mut origins: Vec<String> = Vec::new();
    for html in glob_html(&web) {
        let text = std::fs::read_to_string(&html).unwrap_or_default();
        for line in text.lines() {
            if line.contains("rel=\"preconnect\"") {
                continue;
            }
            for needle in ["href=\"https://", "src=\"https://"] {
                collect_origins(line, needle, '"', &mut origins);
            }
        }
    }
    for css in glob_css(&web) {
        let text = std::fs::read_to_string(&css).unwrap_or_default();
        for line in text.lines() {
            for needle in ["url(https://", "url('https://", "url(\"https://"] {
                collect_origins(line, needle, ')', &mut origins);
            }
        }
    }
    assert!(
        !origins.is_empty(),
        "the shipped export must load at least one external origin for this guard to mean anything"
    );
    for origin in &origins {
        assert!(
            CONTENT_SECURITY_POLICY.contains(origin.as_str()),
            "the shipped frontend loads https://{origin}, which the CSP does not allow; extend CONTENT_SECURITY_POLICY in src/streaming/server/http.rs in the same change"
        );
    }
}

/// Every `https://<host>` origin after `needle` in `line`, cut at `terminator`.
fn collect_origins(line: &str, needle: &str, terminator: char, out: &mut Vec<String>) {
    let mut rest = line;
    while let Some(i) = rest.find(needle) {
        let after = &rest[i + needle.len()..];
        let end = after.find(terminator).unwrap_or(after.len());
        let host: String = after[..end]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-')
            .collect();
        if !host.is_empty() {
            out.push(host);
        }
        rest = &after[end.min(after.len())..];
    }
}

/// Every `*.html` under `web`, sorted for stable assertions.
fn glob_html(web: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out: Vec<std::path::PathBuf> = std::fs::read_dir(web)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "html"))
        .collect();
    out.sort();
    out
}

/// Every `*.css` under `web`'s `_next/static`, recursive.
fn glob_css(web: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![web.join("_next")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "css") {
                out.push(path);
            }
        }
    }
    out
}
