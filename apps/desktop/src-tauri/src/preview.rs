//! Showing the page a run wrote inside the app, instead of only handing it to the desktop.
//!
//! A webview cannot frame a `file://` address — the page's own origin is blocked from asking for
//! one — so the review's open control used to leave the app entirely. This module registers the
//! `preview` scheme, which serves the project folder the run was confined to: the control for an
//! HTML file asks [`preview_url`] and frames the address it gets back, and the game runs inside the
//! app window.
//!
//! The scheme is a read of the workspace by a second door, so every request goes through the same
//! [`SandboxPolicy`] a `read_file` call does: a `..` that leaves the folder, an absolute path from
//! somewhere else, or a protected name like `.env` is answered with the same refusal.

use std::borrow::Cow;

use smollm_tools::SandboxPolicy;
use tauri::http::{header, Method, Request, Response, StatusCode};
use tauri::{AppHandle, UriSchemeContext, Wry};

/// The scheme the in-app browser reads the project folder through.
pub const SCHEME: &str = "preview";

/// The content type a served name answers with, so the webview renders the page instead of
/// downloading it. Only the kinds a page a run builds is made of; anything else is served as
/// bytes, which the webview will show or refuse as it sees fit.
fn mime(name: &str) -> &'static str {
    match name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "txt" | "md" => "text/plain; charset=utf-8",
        "wasm" => "application/wasm",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// The bytes a confined name holds, and the type to answer with. Split from the handler so the
/// reading can be tested without an [`AppHandle`].
fn file_for(
    policy: &SandboxPolicy,
    raw: &str,
) -> Result<(Vec<u8>, &'static str), (StatusCode, String)> {
    let file = crate::reveal::confined(policy, raw).map_err(|why| (StatusCode::NOT_FOUND, why))?;
    let bytes = std::fs::read(&file).map_err(|why| {
        (
            StatusCode::NOT_FOUND,
            format!("{} cannot be read: {why}", file.display()),
        )
    })?;
    let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
    Ok((bytes, mime(name)))
}

fn text(status: StatusCode, body: String) -> Response<Cow<'static, [u8]>> {
    let mut response = Response::new(Cow::Owned(body.into_bytes()));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
}

/// The `preview` protocol handler: one file from the project folder, or the reason there is none.
/// Only the host the scheme itself spells — `localhost` — may ask, and only for reads.
pub fn handle(
    ctx: UriSchemeContext<'_, Wry>,
    request: Request<Vec<u8>>,
) -> Response<Cow<'static, [u8]>> {
    let app = ctx.app_handle();
    if request.method() != Method::GET {
        return text(
            StatusCode::METHOD_NOT_ALLOWED,
            "the preview scheme only serves reads".into(),
        );
    }
    if request.uri().host() != Some("localhost") {
        return text(
            StatusCode::BAD_REQUEST,
            "the preview scheme only answers the app's own localhost".into(),
        );
    }
    let raw = decode_component(request.uri().path().trim_start_matches('/'));
    let policy = match crate::reveal::policy(app) {
        Ok(policy) => policy,
        Err(why) => return text(StatusCode::NOT_FOUND, why),
    };
    match file_for(&policy, &raw) {
        Ok((bytes, mime)) => {
            let mut response = Response::new(Cow::Owned(bytes));
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                header::HeaderValue::from_str(mime)
                    .unwrap_or(header::HeaderValue::from_static("application/octet-stream")),
            );
            // The page is the run's live answer: an edit after the preview opened should show on
            // the next open, not hide behind a cache the scheme has no invalidation for.
            response.headers_mut().insert(
                header::CACHE_CONTROL,
                header::HeaderValue::from_static("no-store"),
            );
            response
        }
        Err((status, why)) => text(status, why),
    }
}

/// The in-app address of one of the run's pages, or the reason there is none: the review's open
/// control for an HTML file asks this and frames the answer, so a refused path is a sentence in the
/// panel rather than a blank page. Same confinement as [`crate::reveal::open_file`].
fn url_for(policy: &SandboxPolicy, path: &str) -> Result<String, String> {
    let file = crate::reveal::confined(policy, path)?;
    let root = policy
        .workspace_root
        .canonicalize()
        .map_err(|why| format!("the project folder cannot be read: {why}"))?;
    let rel = file
        .strip_prefix(&root)
        .map_err(|_| "the file is not inside the project folder".to_string())?;
    let mut url = String::from(SCHEME);
    url.push_str("://localhost/");
    for (i, part) in rel.iter().enumerate() {
        if i > 0 {
            url.push('/');
        }
        url.push_str(&encode_component(&part.to_string_lossy()));
    }
    Ok(url)
}

#[tauri::command]
pub async fn preview_url(app: AppHandle, path: String) -> Result<String, String> {
    let policy = crate::reveal::policy(&app)?;
    url_for(&policy, &path)
}

/// The unreserved set is the only text a URL component may hold unencoded; everything else — a
/// space in "my game.html", a non-ASCII letter — travels as `%XX` and is decoded on the way back
/// in [`decode_component`].
fn encode_component(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// The reverse of [`encode_component`]: a lone `%` or a broken pair stays literal rather than
/// vanishing, so a name that was never encoded still resolves to itself.
fn decode_component(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smollm_tools::Access;
    use std::path::Path;

    fn fixture() -> (tempfile::TempDir, SandboxPolicy) {
        let root = tempfile::tempdir().unwrap();
        let inside = root.path().join("my-app");
        std::fs::create_dir_all(&inside).unwrap();
        std::fs::write(inside.join("index.html"), b"<html></html>").unwrap();
        std::fs::write(inside.join("my game.html"), b"<html></html>").unwrap();
        std::fs::write(inside.join("style.css"), b"body{}").unwrap();
        std::fs::write(root.path().join("elsewhere.txt"), b"outside the task").unwrap();
        let policy = SandboxPolicy {
            workspace_root: root.path().to_path_buf(),
            ..SandboxPolicy::default()
        };
        (root, policy)
    }

    #[test]
    fn a_page_inside_the_workspace_gets_a_preview_address() {
        let (_root, policy) = fixture();
        let url = url_for(&policy, "my-app/index.html").unwrap();
        assert_eq!(url, "preview://localhost/my-app/index.html");
    }

    #[test]
    fn a_name_with_a_space_round_trips_through_the_address() {
        let (_root, policy) = fixture();
        let url = url_for(&policy, "my-app/my game.html").unwrap();
        assert_eq!(url, "preview://localhost/my-app/my%20game.html");
        let raw = decode_component(url.strip_prefix("preview://localhost/").unwrap());
        assert_eq!(raw, "my-app/my game.html");
    }

    #[test]
    fn a_path_that_walks_out_is_refused_not_served() {
        let (_root, policy) = fixture();
        let why = url_for(&policy, "../elsewhere.txt").unwrap_err();
        assert!(
            why.contains("outside") || why.contains("workspace"),
            "{why}"
        );
    }

    #[test]
    fn an_absolute_path_is_refused_too() {
        let (_root, policy) = fixture();
        assert!(url_for(&policy, "/etc/passwd").is_err());
    }

    #[test]
    fn a_protected_name_stays_protected_under_the_scheme() {
        let (root, policy) = fixture();
        std::fs::write(root.path().join(".env"), b"TOKEN=1").unwrap();
        let why = url_for(&policy, ".env").unwrap_err();
        assert!(why.contains("protected") || why.contains("denied"), "{why}");
    }

    #[test]
    fn the_served_bytes_are_the_files_and_the_type_follows_the_name() {
        let (_root, policy) = fixture();
        let (bytes, mime) = file_for(&policy, "my-app/index.html").unwrap();
        assert_eq!(bytes, b"<html></html>");
        assert_eq!(mime, "text/html; charset=utf-8");
        let (_, css) = file_for(&policy, "my-app/style.css").unwrap();
        assert_eq!(css, "text/css; charset=utf-8");
    }

    #[test]
    fn a_missing_file_is_a_not_found() {
        let (_root, policy) = fixture();
        let (status, why) = file_for(&policy, "my-app/nope.html").unwrap_err();
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(why.contains("nothing to open"), "{why}");
    }

    #[test]
    fn encoding_keeps_unreserved_and_escapes_the_rest() {
        assert_eq!(encode_component("index.html"), "index.html");
        assert_eq!(encode_component("my game.html"), "my%20game.html");
        assert_eq!(
            encode_component("главная.html"),
            "%D0%B3%D0%BB%D0%B0%D0%B2%D0%BD%D0%B0%D1%8F.html"
        );
    }

    #[test]
    fn decoding_keeps_a_broken_escape_literal() {
        assert_eq!(decode_component("my%20game.html"), "my game.html");
        assert_eq!(decode_component("50%25"), "50%");
        assert_eq!(decode_component("50%"), "50%");
        assert_eq!(decode_component("50%2"), "50%2");
    }

    #[test]
    fn a_directory_is_not_a_page_to_serve() {
        let (_root, policy) = fixture();
        let (_, why) = file_for(&policy, "my-app").unwrap_err();
        assert!(why.contains("nothing to open"), "{why}");
    }

    #[test]
    fn mime_names_the_kinds_a_page_is_made_of() {
        assert_eq!(mime("a.html"), "text/html; charset=utf-8");
        assert_eq!(mime("B.HTM"), "text/html; charset=utf-8");
        assert_eq!(mime("x.js"), "text/javascript; charset=utf-8");
        assert_eq!(mime("x.woff2"), "font/woff2");
        assert_eq!(mime("noext"), "application/octet-stream");
    }

    #[test]
    fn the_read_access_is_the_same_the_loop_has() {
        // The scheme must not become a wider door than the loop's own reads: the policy used here
        // is the one reveal shares, and this pins the access level the resolution runs at.
        let (_root, policy) = fixture();
        let got = policy
            .resolve_path(Path::new("my-app/index.html"), Access::Read)
            .unwrap();
        assert!(got.is_file());
    }
}
