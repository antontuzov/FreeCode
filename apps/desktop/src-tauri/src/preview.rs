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
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::Mutex;
use std::time::Duration;

use freecode_tools::{PageHost, PageOp, SandboxPolicy};
use serde_json::{json, Value};
use tauri::http::{header, Method, Request, Response, StatusCode};
use tauri::{AppHandle, Emitter, Manager, State, UriSchemeContext, Wry};

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

/// The script every served page carries, so the run's browser calls can act on the *rendered*
/// document: its `innerText` is not in the source, and a button that only works after its own script
/// ran cannot be clicked by editing the HTML.
///
/// The frame's origin is its own (`preview://`), so it cannot be reached from the window's JavaScript
/// directly. Every operation therefore travels as a `postMessage` the window relays from the host's
/// event, and the answer comes back the same way, tagged with the id the host is waiting on. The
/// script is also where the console's errors and warnings are kept: `window.onerror` cannot be
/// installed from the parent either.
///
/// It is injected by [`with_script`] into the served HTML rather than served as a separate file, so a
/// page that names its own strict CSP still has the bridge it needs.
const PAGE_SCRIPT: &str = r##"
(function () {
  if (window.top === window) return;
  var TEXT_MAX = 20000, LOG_MAX = 80, LOG_TEXT = 1000;
  var probe = location.pathname.indexOf("__agent-check") >= 0;
  var logged = [];
  var replies = {};
  function receipt(op, ok) {
    if (!probe) return;
    var el = document.getElementById("freecode-ops");
    if (!el) { el = document.createElement("pre"); el.id = "freecode-ops"; document.body.appendChild(el); }
    el.textContent += (ok ? "ok    " : "FAIL  ") + op + "\n";
  }
  function fmt(value) {
    if (typeof value === "string") return value;
    if (value instanceof Error) return String(value);
    try { return JSON.stringify(value); } catch (e) { return String(value); }
  }
  function push(level, text) {
    if (logged.length >= LOG_MAX) return;
    logged.push({ level: level, text: String(text).slice(0, LOG_TEXT) });
  }
  var nativeError = console.error, nativeWarn = console.warn;
  console.error = function () { push("error", Array.prototype.map.call(arguments, fmt).join(" ")); return nativeError.apply(console, arguments); };
  console.warn = function () { push("warn", Array.prototype.map.call(arguments, fmt).join(" ")); return nativeWarn.apply(console, arguments); };
  window.addEventListener("error", function (e) { push("error", e.message || "script error"); });
  window.addEventListener("unhandledrejection", function (e) { push("error", "unhandled rejection: " + fmt(e.reason)); });

  function name(el) {
    var tag = el.tagName.toLowerCase();
    if (el.id) return tag + "#" + el.id;
    var cls = (el.className && typeof el.className === "string") ? "." + el.className.trim().split(/\s+/).join(".") : "";
    return tag + cls;
  }
  function find(selector) {
    var el = document.querySelector(selector);
    if (!el) throw new Error("no element matches \"" + selector + "\" on this page");
    return el;
  }
  function visibleText(el) {
    var raw = el ? (el.innerText || el.textContent || "") : "";
    return raw.replace(/\s+$/, "").slice(0, TEXT_MAX);
  }
  function click(a) {
    var el = find(a.selector);
    el.scrollIntoView({ block: "center", inline: "center" });
    var box = el.getBoundingClientRect();
    var x = box.left + box.width / 2, y = box.top + box.height / 2;
    if (typeof a.x === "number") x = box.left + a.x;
    if (typeof a.y === "number") y = box.top + a.y;
    var opts = { bubbles: true, cancelable: true, view: window, clientX: x, clientY: y };
    el.dispatchEvent(new MouseEvent("pointerdown", opts));
    el.dispatchEvent(new MouseEvent("mousedown", opts));
    el.dispatchEvent(new MouseEvent("pointerup", opts));
    el.dispatchEvent(new MouseEvent("mouseup", opts));
    el.dispatchEvent(new MouseEvent("click", opts));
    return { target: name(el) };
  }
  function code(key) {
    if (key === " " || key === "Space" || key === "Spacebar") return "Space";
    if (key.length === 1) {
      if (/[a-z]/i.test(key)) return "Key" + key.toUpperCase();
      if (/[0-9]/.test(key)) return "Digit" + key;
      return "";
    }
    return key;
  }
  function keydown(el, key) {
    var props = { key: key === "Space" ? " " : key, code: code(key), bubbles: true, cancelable: true, view: window };
    el.dispatchEvent(new KeyboardEvent("keydown", props));
    el.dispatchEvent(new KeyboardEvent("keypress", props));
    el.dispatchEvent(new KeyboardEvent("keyup", props));
    if (key === "Enter" && el.form && el.form.requestSubmit) el.form.requestSubmit();
  }
  function type(a) {
    var el = a.selector ? find(a.selector) : document.activeElement;
    if (!el || el === document.body) throw new Error("nothing is focused; pass a selector");
    el.focus();
    var proto = (el instanceof HTMLTextAreaElement) ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    var setter = Object.getOwnPropertyDescriptor(proto, "value").set;
    setter.call(el, a.text);
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
    if (a.submit) keydown(el, "Enter");
    return { target: name(el) };
  }
  function press(a) {
    var el = document.activeElement || document.body;
    keydown(el, a.key);
    return { key: a.key };
  }
  function wait(a, done) {
    var started = Date.now();
    var limit = typeof a.timeout_ms === "number" ? a.timeout_ms : 3000;
    try { document.querySelector(a.selector); } catch (e) {
      return done({ error: "\"" + a.selector + "\" is not a selector this page can match: " + e.message });
    }
    (function look() {
      var el = null;
      try { el = document.querySelector(a.selector); } catch (e) { el = null; }
      if (el) return done({ found: true, elapsed_ms: Date.now() - started, text: visibleText(document.body) });
      if (Date.now() - started >= limit) return done({ found: false, elapsed_ms: Date.now() - started, text: visibleText(document.body) });
      setTimeout(look, 50);
    })();
  }
  function perform(op, a, done) {
    try {
      if (op === "read") return done({ title: document.title, url: location.href, text: visibleText(a.selector ? find(a.selector) : document.body) });
      if (op === "click") return done(click(a));
      if (op === "type") return done(type(a));
      if (op === "press") return done(press(a));
      if (op === "console") return done({ lines: logged.slice(0, LOG_MAX) });
      if (op === "wait") return wait(a, done);
      throw new Error("unknown page operation: " + op);
    } catch (e) {
      done({ error: String((e && e.message) || e) });
    }
  }
  window.addEventListener("message", function (e) {
    var d = e.data;
    if (!d || d.source !== "freecode-agent" || typeof d.id !== "number") return;
    // A second post for an id already answered is a retried delivery, not a second click: answering
    // from the cache keeps the page from being made to act twice.
    if (replies[d.id]) { parent.postMessage({ source: "freecode-page", id: d.id, reply: replies[d.id] }, "*"); return; }
    perform(d.op, d.fields || {}, function (body) {
      replies[d.id] = body;
      receipt(d.op, !body.error);
      parent.postMessage({ source: "freecode-page", id: d.id, reply: body }, "*");
    });
  });
  parent.postMessage({ source: "freecode-page", ready: true }, "*");
})();
"##;

/// Put the bridge script into a served page: after `<head>` when there is one, at the very start
/// otherwise. The search is case-insensitive on the opening tag and every byte of the file is kept —
/// this runs on the way to the webview, not on the file.
fn with_script(bytes: Vec<u8>) -> Vec<u8> {
    let tag = format!("<script data-freecode-page>{PAGE_SCRIPT}</script>");
    let lower = bytes.to_ascii_lowercase();
    let after_head = lower
        .windows(5)
        .position(|window| window == b"<head")
        .and_then(|at| {
            lower[at..]
                .iter()
                .position(|byte| *byte == b'>')
                .map(|close| at + close + 1)
        });
    let at = after_head.unwrap_or(0);
    let mut out = Vec::with_capacity(bytes.len() + tag.len());
    out.extend_from_slice(&bytes[..at]);
    out.extend_from_slice(tag.as_bytes());
    out.extend_from_slice(&bytes[at..]);
    out
}

/// The page the development probe drives. The probe writes it into the project folder under
/// `FREECODE_PAGE_PROBE`, so it is opened, confined, injected and framed by the same path every page
/// a run builds goes through; a released build writes nothing. Its visible state — the counter, the
/// entered name, the receipt list — is what a screenshot of the probe shows, which is why every
/// control writes something a reader can see.
const PROBE_PATH: &str = "__agent-check.html";

const PROBE_HTML: &str = r##"<!doctype html>
<html>
<head><meta charset="utf-8"><title>Page probe</title></head>
<body style="font:14px -apple-system,system-ui,sans-serif;padding:28px;color:#182018;background:#fff">
<h1 style="margin:0 0 6px;font-size:20px">Page probe</h1>
<p style="margin:0 0 18px;color:#5a665a">The run's browser calls drive this page. Each one leaves a receipt below.</p>
<button id="bump" style="font:inherit;padding:6px 12px;border:1px solid #b9c4b9;border-radius:8px;background:#f4f7f4;cursor:pointer">Bump the counter</button>
<p style="margin:14px 0 6px">Count: <b id="count">0</b></p>
<p style="margin:0 0 14px"><label>Name <input id="name" style="font:inherit;padding:4px 8px;border:1px solid #b9c4b9;border-radius:6px;width:12em"></label></p>
<p id="entered" style="margin:0 0 14px;color:#2f6b3f"></p>
<textarea id="result" rows="3" style="font:12px ui-monospace,monospace;width:100%;box-sizing:border-box;padding:8px;border:1px solid #d5ddd5;border-radius:8px;color:#3b463b"></textarea>
<pre id="freecode-ops" style="font:12px ui-monospace,monospace;margin-top:14px;color:#3b463b"></pre>
<script>
  var n = 0;
  document.getElementById("bump").addEventListener("click", function () {
    n += 1;
    document.getElementById("count").textContent = String(n);
  });
  document.getElementById("name").addEventListener("keydown", function (e) {
    if (e.key !== "Enter") return;
    document.getElementById("entered").textContent = "Entered: " + e.target.value;
  });
</script>
</body>
</html>
"##;

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
            let bytes = if mime.starts_with("text/html") {
                with_script(bytes)
            } else {
                bytes
            };
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

/// The page a project's browser starts on: `index.html`, or `index.htm`, which are the names a
/// static server answers a folder with.
///
/// A blank tab's Start browser control asks this rather than guessing an address, because the panel
/// is handed paths one at a time by the review and knows no folder to look in on its own. A project
/// with neither page gets a sentence instead of a dead frame; the address field above the button is
/// still the way to any other page.
fn entry_for(policy: &SandboxPolicy) -> Result<String, String> {
    for name in ["index.html", "index.htm"] {
        if policy.workspace_root.join(name).is_file() {
            return url_for(policy, name);
        }
    }
    Err(format!(
        "{} has no index.html to start from; enter a page's path above.",
        policy.workspace_root.display()
    ))
}

#[tauri::command]
pub async fn preview_entry(app: AppHandle) -> Result<String, String> {
    let policy = crate::reveal::policy(&app)?;
    entry_for(&policy)
}

/// The channel a page operation travels on, host to window. The window answers by calling
/// [`page_reply`] with the id it was given.
pub const PAGE_EVENT: &str = "page-request";

/// One waiting operation's inbox: the window's answer, or the reason there is none.
type Waiter = std::sync::mpsc::Sender<Result<Value, String>>;
type Waiters = HashMap<u64, Waiter>;

/// The operations waiting for an answer from the window's page, plus the id source. A tool call that
/// drives the panel blocks on its own receiver — it runs on the blocking pool — and the window's
/// reply is what wakes it.
#[derive(Default)]
pub struct PageBridge {
    waiters: Mutex<Waiters>,
    next: AtomicU64,
}

impl PageBridge {
    fn waiters(&self) -> Result<std::sync::MutexGuard<'_, Waiters>, String> {
        self.waiters
            .lock()
            .map_err(|_| "the page bridge is held by a thread that panicked".to_string())
    }
}

/// The window's answer to one page operation. `reply` is either `{ "value": … }` or `{ "error": … }`.
///
/// An id nobody is waiting on is not an error: the call timed out and its waiter is gone, and the
/// row that call belongs to has already said what happened. The window has no reason to hear about
/// a race it cannot act on.
#[tauri::command]
pub fn page_reply(bridge: State<'_, PageBridge>, id: u64, reply: Value) -> Result<(), String> {
    let sender = bridge.waiters()?.remove(&id);
    if let Some(sender) = sender {
        let answer = match reply.get("error").and_then(Value::as_str) {
            Some(why) => Err(why.to_string()),
            None => Ok(reply.get("value").cloned().unwrap_or(Value::Null)),
        };
        let _ = sender.send(answer);
    }
    Ok(())
}

/// The window's page, as a tool executor sees it: one operation becomes one event, and the answer
/// arrives on [`page_reply`]. This is the only implementation of [`PageHost`] in the build.
pub struct Page<'a> {
    app: &'a AppHandle,
}

impl<'a> Page<'a> {
    pub fn new(app: &'a AppHandle) -> Self {
        Self { app }
    }
}

impl PageHost for Page<'_> {
    fn ask(&self, op: PageOp, arguments: &Value, patience: Duration) -> Result<Value, String> {
        let mut fields = arguments.clone();
        // The panel frames a `preview://` address, and the executor's own addition to an open is the
        // workspace-relative path. Turning that into the address here keeps the window from needing a
        // second round trip — and keeps the sandbox check in the host, where the path was resolved.
        if op == PageOp::Open && fields.get("url").is_none() {
            let path = fields
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let policy = crate::reveal::policy(self.app)?;
            fields["url"] = json!(url_for(&policy, path)?);
        }
        let bridge = self.app.state::<PageBridge>();
        let id = bridge.next.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = std::sync::mpsc::channel();
        bridge.waiters()?.insert(id, sender);
        let sent = self.app.emit(
            PAGE_EVENT,
            json!({ "id": id, "op": op.as_str(), "arguments": fields }),
        );
        if let Err(why) = sent {
            let _ = bridge.waiters().map(|mut waiters| {
                waiters.remove(&id);
            });
            return Err(format!("the window could not be asked: {why}"));
        }
        let answer = receiver.recv_timeout(patience);
        if answer.is_err() {
            let _ = bridge.waiters().map(|mut waiters| {
                waiters.remove(&id);
            });
        }
        match answer {
            Ok(answer) => answer,
            Err(RecvTimeoutError::Timeout) => Err(format!(
                "the browser panel did not answer \"{}\" within {} s. The panel must be open on a \
                 page, and a page that is still loading holds its script back.",
                op.as_str(),
                patience.as_secs()
            )),
            Err(RecvTimeoutError::Disconnected) => {
                Err("the browser panel's answer channel closed before it answered".into())
            }
        }
    }
}

/// The development check behind `?pagecheck=1`: write the probe page into the project folder and drive
/// it with the same executors and the same [`Page`] host a real run uses, leaving the result on the
/// page where a screenshot can show it. Refused unless `FREECODE_PAGE_PROBE` is set, so a released
/// build can neither write the fixture nor run the check.
#[tauri::command]
pub async fn agent_page_probe(app: AppHandle) -> Result<Value, String> {
    if std::env::var_os("FREECODE_PAGE_PROBE").is_none() {
        return Err(
            "agent_page_probe is a development check: launch with FREECODE_PAGE_PROBE=1 and \
             ?pagecheck=1"
                .into(),
        );
    }
    tauri::async_runtime::spawn_blocking(move || probe(&app))
        .await
        .map_err(|why| format!("the probe stopped: {why}"))?
}

fn probe(app: &AppHandle) -> Result<Value, String> {
    let policy = crate::reveal::policy(app)?;
    let host = Page::new(app);
    // The page is written into the project folder, so `browser_open` resolves it, the `preview` scheme
    // serves it and the bridge script is injected into it exactly as for a page a run wrote.
    let file = policy.workspace_root.join(PROBE_PATH);
    std::fs::write(&file, PROBE_HTML)
        .map_err(|why| format!("{} cannot be written: {why}", file.display()))?;
    let mut steps = Vec::new();
    run_page(
        &mut steps,
        "browser_open",
        json!({ "path": PROBE_PATH }),
        &policy,
        &host,
    )?;
    let read = run_page(&mut steps, "browser_read", json!({}), &policy, &host)?;
    if !read.contains("Page probe") {
        return Err(format!("the read did not see the page: {read:?}"));
    }
    run_page(
        &mut steps,
        "browser_click",
        json!({ "selector": "#bump" }),
        &policy,
        &host,
    )?;
    let count = run_page(
        &mut steps,
        "browser_read",
        json!({ "selector": "#count" }),
        &policy,
        &host,
    )?;
    if !count.contains('1') {
        return Err(format!("the click did not reach the button: {count:?}"));
    }
    run_page(
        &mut steps,
        "browser_type",
        json!({ "selector": "#name", "text": "Ada" }),
        &policy,
        &host,
    )?;
    run_page(
        &mut steps,
        "browser_press",
        json!({ "key": "Enter" }),
        &policy,
        &host,
    )?;
    let waited = run_page(
        &mut steps,
        "browser_wait",
        json!({ "selector": "#entered", "timeout_ms": 2000 }),
        &policy,
        &host,
    )?;
    if !waited.contains("Ada") {
        return Err(format!("the typed name did not reach the page: {waited:?}"));
    }
    run_page(&mut steps, "browser_console", json!({}), &policy, &host)?;
    let summary = format!("{} checks passed on this page", steps.len());
    host.ask(
        PageOp::Type,
        &json!({ "selector": "#result", "text": summary }),
        PageOp::Type.patience(),
    )?;
    Ok(json!({
        "ok": true,
        "url": format!("{SCHEME}://localhost/{PROBE_PATH}"),
        "steps": steps,
    }))
}

/// One probe step through the real executor, recorded for the report rather than answered to a model.
fn run_page(
    steps: &mut Vec<Value>,
    tool: &str,
    arguments: Value,
    policy: &SandboxPolicy,
    host: &Page,
) -> Result<String, String> {
    let done = freecode_tools::execute_watched(tool, &arguments, policy, None, Some(host))
        .map_err(|why| format!("{tool} did not run: {why}"))?;
    steps.push(json!({
        "tool": tool,
        "summary": done.summary.text,
        "key": done.summary.key.map(|key| key.i18n_key()),
        "failed": done.summary.failed(),
    }));
    Ok(done.content)
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
    use freecode_tools::Access;
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
    fn a_project_starts_on_its_index_page() {
        let (root, policy) = fixture();
        std::fs::write(root.path().join("index.html"), b"<html></html>").unwrap();
        assert_eq!(
            entry_for(&policy).unwrap(),
            "preview://localhost/index.html"
        );
    }

    #[test]
    fn index_htm_is_an_entry_page_too() {
        let (root, policy) = fixture();
        std::fs::write(root.path().join("index.htm"), b"<html></html>").unwrap();
        assert_eq!(entry_for(&policy).unwrap(), "preview://localhost/index.htm");
    }

    #[test]
    fn a_project_without_an_entry_page_says_so_instead_of_guessing_one() {
        // The fixture's only page is under `my-app/`: a start control that opened it would be
        // choosing among the run's files for the user, and one that opened nothing would be silent.
        let (_root, policy) = fixture();
        let why = entry_for(&policy).unwrap_err();
        assert!(why.contains("no index.html"), "{why}");
        assert!(why.contains("enter a page's path"), "{why}");
    }

    #[test]
    fn entry_is_resolved_through_the_same_sandbox_the_scheme_serves() {
        // The entry is answered as an address by the same function every served page goes through,
        // so it cannot become a door around the confinement that one keeps.
        let (root, policy) = fixture();
        std::fs::write(root.path().join("index.html"), b"<html></html>").unwrap();
        let url = entry_for(&policy).unwrap();
        let raw = decode_component(url.strip_prefix("preview://localhost/").unwrap());
        assert!(file_for(&policy, &raw).is_ok());
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
