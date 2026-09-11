//! The SPA, served two ways.

use std::borrow::Cow;
use std::path::{Component, Path, PathBuf};

use axum::extract::Path as UrlPath;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

/// Where the SPA is read from under `GB_WEB_DEV=1`, relative to the process's working directory.
const DEV_ROOT: &str = "web/dist";

/// Vite emits content-hashed filenames under `assets/`, so those may be cached forever;
/// `index.html` is the mutable pointer to them and must not be.
const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const REVALIDATE: &str = "no-cache";

#[derive(rust_embed::Embed)]
#[folder = "web/dist"]
struct Dist;

/// `GET /` — always `index.html`.
pub async fn index() -> Response {
    serve("index.html")
}

/// `GET /{*path}`.
pub async fn asset(UrlPath(path): UrlPath<String>) -> Response {
    serve(&path)
}

fn serve(path: &str) -> Response {
    let Some(path) = sanitise(path) else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    let Some(body) = read(&path) else {
        // The one case worth explaining rather than 404ing: the binary was built before the SPA
        // was.
        if path == "index.html" {
            return (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/html; charset=utf-8"), (header::CACHE_CONTROL, REVALIDATE)],
                NOT_BUILT,
            )
                .into_response();
        }
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };

    let caching = if path.starts_with("assets/") { IMMUTABLE } else { REVALIDATE };
    ([(header::CONTENT_TYPE, content_type(&path)), (header::CACHE_CONTROL, caching)], body).into_response()
}

/// Read one asset, from the binary or — under `GB_WEB_DEV` — from disk.
fn read(path: &str) -> Option<Cow<'static, [u8]>> {
    if dev_mode() {
        return std::fs::read(Path::new(DEV_ROOT).join(path)).ok().map(Cow::Owned);
    }
    Dist::get(path).map(|file| file.data)
}

fn dev_mode() -> bool {
    std::env::var_os("GB_WEB_DEV").is_some_and(|value| value != "0" && value != "")
}

fn sanitise(path: &str) -> Option<String> {
    let trimmed = path.trim_start_matches('/');
    if trimmed.is_empty() {
        return Some("index.html".to_string());
    }
    let mut clean = PathBuf::new();
    for component in Path::new(trimmed).components() {
        match component {
            Component::Normal(part) => clean.push(part),
            _ => return None,
        }
    }
    // `rust-embed` keys on forward slashes on every platform, and so does the URL.
    Some(clean.components().filter_map(|c| c.as_os_str().to_str()).collect::<Vec<_>>().join("/"))
}

/// Only what Vite actually emits, plus the handful a favicon or a font would need.
fn content_type(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, extension)| extension) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json" | "map") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Served at `/` when `web/dist` held no `index.html` at compile time.
const NOT_BUILT: &str = r#"<!doctype html>
<meta charset="utf-8">
<title>gb · the UI is not built</title>
<link rel="icon" type="image/png" href="/favicon.png">
<body style="background:#0f1115;color:#d7dae0;font:14px/1.6 ui-monospace,monospace;padding:40px">
<h1 style="font-size:16px">The web UI was not built into this binary.</h1>
<p>The SPA is compiled into <code>gb</code> from <code>web/dist</code>, which was empty when the
binary was built. Build it and rebuild:</p>
<pre style="background:#161920;border:1px solid #262b34;border-radius:4px;padding:12px">cd web &amp;&amp; pnpm install &amp;&amp; pnpm run build
cargo build --release</pre>
<p>Or run <code>GB_WEB_DEV=1 gb serve</code> to read <code>web/dist</code> from disk instead.
The API is up either way — <code>/api/events</code>, <code>/api/video</code>,
<code>/api/healthz</code>.</p>
</body>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    /// The traversal defence, from both directions: nothing that could leave the asset root
    /// survives, and everything the SPA actually asks for does.
    #[test]
    fn sanitise_keeps_asset_paths_and_rejects_everything_else() {
        assert_eq!(sanitise("index.html").as_deref(), Some("index.html"));
        assert_eq!(sanitise("/assets/index-D8_AXMtz.js").as_deref(), Some("assets/index-D8_AXMtz.js"));
        assert_eq!(sanitise("").as_deref(), Some("index.html"));
        assert_eq!(sanitise("/").as_deref(), Some("index.html"));

        for hostile in [
            "../Cargo.toml",
            "assets/../../Cargo.toml",
            "/../../etc/passwd",
            "..",
            "./assets/x.js", // a `.` component: harmless, but nothing legitimate sends one
        ] {
            assert_eq!(sanitise(hostile), None, "{hostile} was not rejected");
        }
    }

    /// A path that survives sanitising can still only name a file *inside* the root, which is the
    /// property the disk read depends on.
    #[test]
    fn a_sanitised_path_stays_under_the_root() {
        for path in ["index.html", "assets/deep/nested/file.js", "favicon.ico"] {
            let joined = Path::new(DEV_ROOT).join(sanitise(path).expect("legitimate path"));
            assert!(joined.starts_with(DEV_ROOT), "{path} resolved outside the root: {joined:?}");
            assert!(!joined.components().any(|c| c == Component::ParentDir));
        }
    }

    #[test]
    fn content_types_cover_what_vite_emits() {
        assert_eq!(content_type("index.html"), "text/html; charset=utf-8");
        assert_eq!(content_type("assets/index-abc123.js"), "text/javascript; charset=utf-8");
        assert_eq!(content_type("assets/index-abc123.css"), "text/css; charset=utf-8");
        assert_eq!(content_type("assets/logo.svg"), "image/svg+xml");
        assert_eq!(content_type("LICENSE"), "application/octet-stream");
    }

    /// `/` answers with a page in both worlds — the built SPA, or the message explaining that it
    /// is not built.
    #[test]
    fn index_is_always_a_page() {
        let response = serve("index.html");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()),
            Some("text/html; charset=utf-8"),
        );
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).and_then(|v| v.to_str().ok()),
            Some(REVALIDATE),
            "the entry point points at hashed assets, so it must never be cached",
        );

        let body = body_of(response);
        if Dist::get("index.html").is_some() {
            assert!(body.contains(r#"id="root""#), "the built SPA should mount into #root: {body}");
        } else {
            assert!(body.contains("not built"), "the placeholder should say what to run: {body}");
        }
    }

    #[test]
    fn a_missing_asset_is_a_404_rather_than_the_index() {
        assert_eq!(serve("assets/does-not-exist.js").status(), StatusCode::NOT_FOUND);
        assert_eq!(serve("../Cargo.toml").status(), StatusCode::NOT_FOUND);
    }

    /// Reading a body is the only async part of any of this, and it is not worth a `macros`
    /// feature on `tokio` to say so.
    fn body_of(response: Response) -> String {
        let runtime = tokio::runtime::Builder::new_current_thread().build().expect("a bare runtime");
        let bytes = runtime
            .block_on(axum::body::to_bytes(response.into_body(), usize::MAX))
            .expect("the body is a small page held in memory");
        String::from_utf8(bytes.to_vec()).expect("html is utf-8")
    }
}
