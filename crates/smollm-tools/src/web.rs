//! The two calls that leave the machine: a search, and the read of one page it points at.
//!
//! [`crate::sandbox`] has already said yes before this module is reached — the network gate is the
//! Privacy switch, and nothing here runs with it off. What is left for this file to decide is the part
//! a permission class cannot: *which* addresses are allowed to be reached at all, and how much of what
//! comes back is worth the context window it will occupy on every later round of the run.
//!
//! The search has one provider, and its name is in the sentence the model reads. That is a deliberate
//! limit rather than an oversight: every key-free general web search API is either a paid product or a
//! page built for a browser, and a scraper aimed at the second kind stops working the week the site
//! notices it. Wikipedia's own search answers a large share of the questions a user asks a local model
//! about — who someone is, what a thing is, what a term means — and it says so in JSON with a
//! documented contract. `fetch_url` is the half that reaches the rest of the internet: the model reads
//! any public page it has been pointed at, including the ones a search could not have found.
//!
//! Three guards, none of which is about the provider:
//!   * **scheme**: http and https, so a call cannot read a `file://` path past the workspace sandbox
//!     that bounds every other read in this build;
//!   * **address**: loopback, private, link-local and similar ranges are refused, *after* the host name
//!     has been resolved — because `localhost` is how a model reaches the engine this app is
//!     supervising, and reading it is not what a web tool is for;
//!   * **size**: one page is capped on the wire and again in characters, because a tool result is
//!     replayed on every later round and one 2 MB article would spend the conversation's window.
//!
//! The address check happens before the request and again on every redirect, so a public page cannot
//! be used as a door into the local network. It is not a complete answer: a name that resolves to two
//! addresses, one public and one private, is refused rather than tried, and a DNS record that changes
//! between the check and the connection (rebinding, TTL 0) can still slip through. That is the
//! residual risk of a tool that reads arbitrary URLs at all, and it is why the whole module stays
//! behind the switch the user turns on.
//!
//! What comes back is reduced by [`html_to_text`], a scanner rather than a parser. It drops the page's
//! menus, scripts and notices before the character budget is spent — the first lines of a real article
//! are its chrome, and a 2 B model answers better from its prose than from "Main menu / Navigation" —
//! and the page's own `<title>` becomes the label the transcript row carries.

use std::io::Read;
use std::net::{IpAddr, ToSocketAddrs};
use std::time::Duration;

use reqwest::redirect::Policy;
use reqwest::Url;
use serde_json::Value;

/// How long one web call may take before the round says it did not. Long enough for a cold lookup on a
/// bad connection, short enough that a hung host costs one blocked row rather than the run.
const TIMEOUT: Duration = Duration::from_secs(12);

/// Bytes read off the wire for one page. The page is stripped to text before any of it reaches the
/// model, and no article's markup is smaller than this.
const MAX_BODY_BYTES: usize = 400 * 1024;

/// Hops one call will follow before it says the page will not settle.
const MAX_REDIRECTS: usize = 5;

/// The most results one search hands back.
pub const MAX_RESULTS: usize = 8;

/// Characters of page text a call returns when the model did not ask for a number.
pub const DEFAULT_PAGE_CHARS: usize = 6_000;

/// The same figure when the model does ask, capped: 12 kB is the byte budget every other tool result
/// obeys ([`crate::exec::MAX_RESULT_BYTES`]), and a page read is replayed on every later round.
pub const MAX_PAGE_CHARS: usize = 12_000;

/// The documented, key-free page search. `utm_source` is not required, but Wikimedia asks that a
/// client identify itself, and the user agent below is what does that.
const SEARCH_BASE: &str = "https://en.wikipedia.org/w/rest.php/v1/search/page";

/// What one search returned. The URL is the page itself, not a redirect through the provider, so
/// `fetch_url` can be handed it unchanged and a user can click the same link in the transcript.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// One page, read and reduced to the text a model can use.
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub url: String,
    pub text: String,
    /// What the page calls itself: its `<title>`, or the first line of its body when the document has
    /// no head element. This is the label the transcript row shows and the answer cites, so it is
    /// worked out here rather than left to whoever renders the row.
    pub title: String,
    /// The page was longer than the budget, either on the wire or in characters. The row says so
    /// rather than letting a cut read as the whole document.
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum WebError {
    /// The call was declined before anything was sent, and the sentence says which rule did it. A
    /// refusal is aimed at the model as much as at the user: an address it named is a thing it can
    /// change on the next round.
    #[error("{0}")]
    Refused(String),
    /// The request went out and the internet did not answer usefully.
    #[error("{0}")]
    Failed(String),
}

/// Search the provider for `query`. An empty result is `Ok(vec![])`, not an error: "Wikipedia has
/// nothing for that phrasing" is an answer the model can act on, and a failure is not.
pub fn search(query: &str, limit: usize) -> Result<Vec<Hit>, WebError> {
    let query = query.trim();
    if query.is_empty() {
        return Err(WebError::Refused(
            "the search was asked with no words in it".to_string(),
        ));
    }
    let url = search_url(query, limit)?;
    Ok(parse_hits(&get_json(&url)?))
}

/// The provider's own address for one query. Split out because the encoding is the part worth testing
/// without a network: a query with `&` or a non-Latin word must arrive as one argument.
fn search_url(query: &str, limit: usize) -> Result<Url, WebError> {
    let limit = limit.clamp(1, MAX_RESULTS).to_string();
    Url::parse_with_params(SEARCH_BASE, &[("q", query), ("limit", limit.as_str())])
        .map_err(|e| WebError::Failed(format!("the search address could not be built: {e}")))
}

/// Read one page and return its text. `max_chars` is the model's own number when it gives one.
pub fn read_page(raw: &str, max_chars: usize) -> Result<Page, WebError> {
    let url = Url::parse(raw.trim()).map_err(|e| {
        WebError::Refused(format!(
            "\"{raw}\" is not an address this tool can read: {e}. Give it a full http(s) URL."
        ))
    })?;
    guard(&url)?;
    let resp = client()?
        .get(url.clone())
        .send()
        .map_err(|e| WebError::Failed(format!("{url} could not be reached: {e}")))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(WebError::Failed(format!(
            "{url} answered {status} instead of the page"
        )));
    }
    let ctype = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if !reads_as_text(&ctype) {
        return Err(WebError::Failed(format!(
            "{url} is {} rather than a text page, and this build reads only html, plain text, \
             markdown, JSON and XML",
            if ctype.is_empty() {
                "an unlabelled type"
            } else {
                ctype.as_str()
            }
        )));
    }
    let (bytes, capped_on_the_wire) = read_capped(resp)?;
    let body = String::from_utf8_lossy(&bytes);
    // The document's own name, taken before the head is stripped away — that element is where it
    // lives, and it is a better label than whatever the first line of the body happens to be.
    let head = head_title(&body);
    // A JSON or XML answer needs no stripping, and stripping it would take the structure the model
    // asked for. HTML is the only format whose tags are noise in a sentence.
    let text = if looks_like_markup(&body) {
        html_to_text(&body)
    } else {
        body.into_owned()
    };
    let max = max_chars.clamp(1, MAX_PAGE_CHARS);
    let chars = text.chars().count();
    let text: String = text.chars().take(max).collect();
    let title = head.unwrap_or_else(|| line_title(&text, &url));
    Ok(Page {
        url: url.to_string(),
        title,
        truncated: capped_on_the_wire || chars > text.chars().count(),
        text,
    })
}

/// The page's `<title>` text, if it has one with something in it. Entities are decoded and the whole
/// thing is capped, because the row that shows it is one line wide.
fn head_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let after = start + "<title".len();
    let next = html.as_bytes().get(after).copied();
    if !matches!(
        next,
        Some(b'>') | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'/')
    ) {
        return None;
    }
    let open_end = html[after..].find('>')?;
    let inner = &html[after + open_end + 1..];
    let close = inner.to_ascii_lowercase().find("</title>")?;
    let text = html_to_text(&inner[..close])
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() {
        return None;
    }
    Some(text.chars().take(120).collect())
}

/// The first line with anything on it, which is the best label a document without a head can give.
fn line_title(text: &str, url: &Url) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(url.as_str())
        .chars()
        .take(120)
        .collect()
}

fn client() -> Result<reqwest::blocking::Client, WebError> {
    reqwest::blocking::Client::builder()
        .user_agent(concat!("smollm-studio/", env!("CARGO_PKG_VERSION")))
        .timeout(TIMEOUT)
        // The guard the first address passed, applied to every hop, so a public page cannot hand the
        // run a redirect into the local network. reqwest's `limited` and `custom` are one enum
        // variant rather than two settings, so a policy that checks destinations has to count hops
        // itself — and a page that will not settle is worth refusing the same as one that resolves
        // somewhere it should not.
        .redirect(Policy::custom(|attempt| {
            let hops = attempt.previous().len().saturating_sub(1);
            if hops >= MAX_REDIRECTS {
                return attempt.error("this page redirected too many times to settle on one");
            }
            match guard(attempt.url()) {
                Ok(()) => attempt.follow(),
                Err(refusal) => attempt.error(refusal.to_string()),
            }
        }))
        .build()
        .map_err(|e| WebError::Failed(format!("the web client could not start: {e}")))
}

fn get_json(url: &Url) -> Result<Value, WebError> {
    guard(url)?;
    let resp = client()?
        .get(url.clone())
        .send()
        .map_err(|e| WebError::Failed(format!("{url} could not be reached: {e}")))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(WebError::Failed(format!(
            "the search answered {status} instead of results"
        )));
    }
    let (bytes, _) = read_capped(resp)?;
    serde_json::from_slice(&bytes).map_err(|e| {
        WebError::Failed(format!(
            "the search answered in a shape not understood: {e}"
        ))
    })
}

/// Read at most [`MAX_BODY_BYTES`] off the response, and say whether there was more.
fn read_capped(mut resp: reqwest::blocking::Response) -> Result<(Vec<u8>, bool), WebError> {
    let mut buf = Vec::new();
    let mut scratch = [0u8; 64 * 1024];
    let mut over = false;
    loop {
        let got = resp
            .read(&mut scratch)
            .map_err(|e| WebError::Failed(format!("the page stopped arriving: {e}")))?;
        if got == 0 {
            break;
        }
        buf.extend_from_slice(&scratch[..got]);
        if buf.len() >= MAX_BODY_BYTES {
            over = true;
            break;
        }
    }
    buf.truncate(MAX_BODY_BYTES);
    Ok((buf, over))
}

/// Scheme and destination, checked before a single packet leaves.
fn guard(url: &Url) -> Result<(), WebError> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(WebError::Refused(format!(
            "{} is a {} address. This tool reads http and https only, and never a file on this \
             machine — the workspace tools are the way to those.",
            url,
            url.scheme()
        )));
    }
    let Some(host) = url.host_str() else {
        return Err(WebError::Refused(format!("{url} names no host")));
    };
    let port = url.port_or_known_default().unwrap_or(443);
    if let Ok(ip) = host.parse::<IpAddr>() {
        return refuse_ip(url, ip);
    }
    // A name is looked up rather than trusted: "localhost", "metadata" and an internal DNS name all
    // reach this machine's own services without an address that would have been refused.
    let addrs = (host, port)
        .to_socket_addrs()
        .map_err(|e| {
            WebError::Refused(format!(
                "{host} did not resolve, so it cannot be checked ({e})"
            ))
        })?
        .collect::<Vec<_>>();
    if addrs.is_empty() {
        return Err(WebError::Refused(format!("{host} resolved to nothing")));
    }
    for addr in &addrs {
        refuse_ip(url, addr.ip())?;
    }
    Ok(())
}

fn refuse_ip(url: &Url, ip: IpAddr) -> Result<(), WebError> {
    if is_private(ip) {
        return Err(WebError::Refused(format!(
            "{url} resolves to {ip}, an address on this machine's own network. The web tools read the \
             public internet; the engine this app runs is not something they look at."
        )));
    }
    Ok(())
}

/// Loopback, the private ranges, and the ones a cloud instance uses to read its own credentials.
///
/// Written against the stable address predicates plus two hand-checked ranges, because the equivalents
/// for IPv6 unique-local space and IPv4-mapped addresses are still nightly-only.
fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_multicast()
                // 100.64.0.0/10: carrier-grade NAT, and the range a container's own network sits on.
                || (o[0] == 100 && o[1] & 0xc0 == 0x40)
                // 0.0.0.0/8 means "this host" as often as it means nothing.
                || o[0] == 0
        }
        IpAddr::V6(v6) => {
            let s = v6.segments();
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || v6.is_unicast_link_local()
                // fc00::/7 — unique local.
                || (s[0] & 0xfe00) == 0xfc00
                // ::ffff:a.b.c.d and ::a.b.c.d — an IPv4 address wearing IPv6 syntax.
                || (s[0] == 0 && s[1] == 0 && s[2] == 0 && s[3] == 0 && s[4] == 0xffff
                    && is_private(IpAddr::from([
                        (s[5] >> 8) as u8,
                        s[5] as u8,
                        (s[6] >> 8) as u8,
                        s[6] as u8,
                    ])))
                || (s[0] == 0 && s[1] == 0 && s[2] == 0 && s[3] == 0 && s[4] == 0 && s[5] == 0
                    && (s[6] == 0 || s[6] == 0xffff))
        }
    }
}

/// The content types whose bytes are worth handing to a model as text.
fn reads_as_text(ctype: &str) -> bool {
    let base = ctype.split(';').next().unwrap_or("").trim().to_lowercase();
    base.is_empty()
        || base.starts_with("text/")
        || matches!(
            base.as_str(),
            "application/json"
                | "application/xml"
                | "application/xhtml+xml"
                | "application/rss+xml"
        )
        || base.ends_with("+json")
        || base.ends_with("+xml")
}

/// Whether a body is markup to be stripped rather than text to be passed through. Decided on the
/// content rather than the header, because a page served as `text/plain` with its tags still on it is
/// the common case and would otherwise reach the model as `<div class="…`.
fn looks_like_markup(body: &str) -> bool {
    let head = body.trim_start();
    let head = &head[..head.len().min(2048)];
    let lower = head.to_ascii_lowercase();
    lower.contains("<html") || lower.contains("<!doctype html") || lower.contains("<body")
}

/// The parts of an HTML document that are not the document.
///
/// The four chrome elements are here because a page's first hundred lines are otherwise its menus:
/// the probe against the live provider showed a Wikipedia article arriving as "Jump to content /
/// Main menu / Navigation / Main page", which is not an answer a model can read anything out of.
const DROP_BLOCKS: &[&str] = &[
    "script", "style", "noscript", "head", "svg", "template", "header", "footer", "nav", "aside",
];

/// Block-level tags whose end is worth a line break in the text that survives.
const BLOCKS: &[&str] = &[
    "p",
    "div",
    "li",
    "ul",
    "ol",
    "table",
    "tr",
    "td",
    "th",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "br",
    "hr",
    "section",
    "article",
    "header",
    "footer",
    "nav",
    "blockquote",
    "pre",
    "dd",
    "dt",
];

/// Reduce an HTML document to readable text: no markup, no scripts, no entity escapes, one blank line
/// at most between blocks.
///
/// Deliberately a scanner rather than a parser. The whole job is to get prose out of a page well enough
/// for a 2 B model to answer from it, and a real parser is a dependency this build does not need to
/// have an opinion about. Anything it mangles, it mangles into readable text rather than into an error.
pub fn html_to_text(html: &str) -> String {
    let doc = drop_blocks(html);
    let mut out = String::with_capacity(doc.len() / 2 + 16);
    let mut rest = doc.as_str();
    loop {
        let Some(start) = rest.find('<') else {
            out.push_str(&decode_entities(rest));
            break;
        };
        out.push_str(&decode_entities(&rest[..start]));
        // A comment ends at its own marker rather than at the first bracket, because a `>` inside one
        // — every conditional IE comment has several — otherwise lands in the prose as a stray
        // character, which is what the live provider showed.
        if rest[start..].starts_with("<!--") {
            let body = &rest[start + 4..];
            let close = body.find("-->").map(|rel| start + 4 + rel + 3);
            rest = &rest[close.unwrap_or(rest.len())..];
            continue;
        }
        let Some(end) = tag_end(&rest[start..]) else {
            // A stray `<` with no closing bracket is text, not a tag.
            out.push_str(&decode_entities(&rest[start..]));
            break;
        };
        let tag = &rest[start + 1..start + end];
        if !tag.starts_with('!') {
            let name: String = tag
                .trim_start_matches('/')
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            let name = name.to_ascii_lowercase();
            let closing = tag.starts_with('/');
            if BLOCKS.contains(&name.as_str()) && name != "br" && name != "hr" {
                if closing {
                    out.push('\n');
                }
            } else if name == "br" || name == "hr" {
                out.push('\n');
            }
        }
        rest = &rest[start + end + 1..];
    }
    collapse(&out)
}

/// Where a tag ends: the first bracket that is not inside a quoted attribute value.
///
/// Wikipedia serves Parsoid markup whose `data-mw` attributes are single-quoted JSON containing
/// brackets, so cutting at the first `>` in the middle of one leaves the rest of that attribute —
/// `class="…"` and all — sitting in the prose. The live provider probe is what found it.
fn tag_end(tail: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (at, ch) in tail.char_indices().skip(1) {
        match quote {
            Some(open) if ch == open => quote = None,
            Some(_) => {}
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if ch == '>' => return Some(at),
            None => {}
        }
    }
    None
}

/// Drop every `<name>…</name>` block for the names in [`DROP_BLOCKS`], case-insensitively.
fn drop_blocks(html: &str) -> String {
    let mut doc = html.to_string();
    for name in DROP_BLOCKS {
        doc = drop_one(&doc, name);
    }
    doc
}

fn drop_one(doc: &str, name: &str) -> String {
    let lower = doc.to_ascii_lowercase();
    let open = format!("<{name}");
    let close = format!("</{name}");
    let mut out = String::with_capacity(doc.len());
    let mut at = 0usize;
    while at < doc.len() {
        let Some(rel) = lower[at..].find(&open) else {
            out.push_str(&doc[at..]);
            break;
        };
        let start = at + rel;
        // Only a real opening tag: `<scriptx` is not `<script`.
        let after = start + open.len();
        let next = doc.as_bytes().get(after).copied();
        if !matches!(
            next,
            Some(b'>') | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'/')
        ) {
            out.push_str(&doc[at..after]);
            at = after;
            continue;
        }
        out.push_str(&doc[at..start]);
        let end = lower[after..]
            .find(&close)
            .map(|rel| after + rel + close.len())
            .unwrap_or(doc.len());
        // The block's own closing bracket is not part of its name, and leaving it behind put a bare
        // `>` into the text — one line of it for every script, style and menu on the page.
        at = past_bracket(doc, end);
    }
    out
}

/// Step over the `>` that ends a tag whose name has just been matched, so removing a block does not
/// leave its terminator in the prose.
fn past_bracket(doc: &str, mut at: usize) -> usize {
    while matches!(
        doc.as_bytes().get(at).copied(),
        Some(b' ') | Some(b'\t') | Some(b'\n')
    ) {
        at += 1;
    }
    if doc.as_bytes().get(at) == Some(&b'>') {
        at + 1
    } else {
        at
    }
}

/// The named escapes a page actually uses, plus numeric references. Anything else is left alone rather
/// than guessed at.
fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    const NAMED: &[(&str, &str)] = &[
        ("amp", "&"),
        ("lt", "<"),
        ("gt", ">"),
        ("quot", "\""),
        ("apos", "'"),
        ("nbsp", " "),
        ("mdash", "—"),
        ("ndash", "–"),
        ("hellip", "…"),
        ("middot", "·"),
        ("laquo", "«"),
        ("raquo", "»"),
        ("copy", "©"),
        ("reg", "®"),
        ("trade", "™"),
        ("deg", "°"),
        ("rsquo", "\u{2019}"),
        ("lsquo", "\u{2018}"),
        ("rdquo", "\u{201d}"),
        ("ldquo", "\u{201c}"),
    ];
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find(';') else {
            out.push('&');
            rest = &rest[start + 1..];
            continue;
        };
        let entity = &rest[start + 1..start + end];
        let decoded = if let Some(code) = entity.strip_prefix('#') {
            let (radix, digits) = match code.strip_prefix(['x', 'X']) {
                Some(rest) => (16, rest),
                None => (10, code),
            };
            u32::from_str_radix(digits, radix)
                .ok()
                .and_then(char::from_u32)
                .map(|c| c.to_string())
        } else {
            NAMED
                .iter()
                .find(|(name, _)| *name == entity)
                .map(|(_, value)| (*value).to_string())
        };
        match decoded {
            Some(text) => out.push_str(&text),
            // An unknown entity is its own text: dropping it would lose a word to save four characters.
            None => out.push_str(&rest[start..start + end + 1]),
        }
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    out
}

/// Trim each line, drop the empty ones, and keep at most one blank line between paragraphs.
fn collapse(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank = false;
    for line in text.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            blank = true;
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
            if blank {
                out.push('\n');
            }
        }
        out.push_str(&line);
        blank = false;
    }
    out
}

/// Turn one search response into hits. Split from the request so the shape of a provider's answer is
/// testable against a fixture rather than against the provider.
fn parse_hits(body: &Value) -> Vec<Hit> {
    let Some(pages) = body.get("pages").and_then(Value::as_array) else {
        return Vec::new();
    };
    pages
        .iter()
        .take(MAX_RESULTS)
        .filter_map(|page| {
            let title = page
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            if title.is_empty() {
                return None;
            }
            let key = page
                .get("key")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| title.replace(' ', "_"));
            let description = page
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            let excerpt = clean_snippet(page.get("excerpt").and_then(Value::as_str).unwrap_or(""));
            let snippet = match (description.is_empty(), excerpt.is_empty()) {
                (true, _) => excerpt,
                (false, true) => description.to_string(),
                (false, false) => format!("{description} {excerpt}"),
            };
            Some(Hit {
                title: title.to_string(),
                url: format!("https://en.wikipedia.org/wiki/{}", encode_key(&key)),
                snippet,
            })
        })
        .collect()
}

fn clean_snippet(raw: &str) -> String {
    let text = html_to_text(raw);
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The article path, with everything a URL may not carry percent-encoded.
fn encode_key(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    for byte in key.as_bytes() {
        match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'_'
            | b'-'
            | b'.'
            | b'~'
            | b'('
            | b')'
            | b'\'' => out.push(*byte as char),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_with_awkward_characters_arrives_as_one_argument() {
        let url = search_url("Ricoeur & Kant", 5).unwrap();
        assert_eq!(url.query_pairs().count(), 2);
        assert_eq!(
            url.query_pairs().find(|(k, _)| k == "q").unwrap().1,
            "Ricoeur & Kant"
        );
        let non_latin = search_url("マリリン・モンロー", 3).unwrap();
        assert!(non_latin.as_str().contains("%E3%83%9E"), "{non_latin}");
        // The limit is clamped rather than passed through, because the provider's own cap is smaller
        // than what a model will ask for when it asks at all.
        assert!(search_url("x", 500).unwrap().as_str().ends_with("limit=8"));
        assert!(search_url("x", 0).unwrap().as_str().ends_with("limit=1"));
    }

    #[test]
    fn a_search_response_becomes_hits_with_clickable_addresses() {
        let body = serde_json::json!({
            "pages": [
                {
                    "title": "Grace Hopper",
                    "key": "Grace_Hopper",
                    "description": "U.S. naval officer and computer scientist (1906–1992)",
                    "excerpt": "<span class=\"searchmatch\">Grace</span> Brewster Hopper (née Murray; &nbsp;1906)"
                },
                { "title": "  " },
                { "title": "Hopper (microarchitecture)", "excerpt": "Named for a rear admiral" }
            ]
        });
        let hits = parse_hits(&body);
        assert_eq!(hits.len(), 2, "the blank title is not a hit");
        assert_eq!(hits[0].title, "Grace Hopper");
        assert_eq!(hits[0].url, "https://en.wikipedia.org/wiki/Grace_Hopper");
        assert_eq!(
            hits[0].snippet,
            "U.S. naval officer and computer scientist (1906–1992) Grace Brewster Hopper (née Murray; 1906)"
        );
        // No `key` still lands on the right article, because the title is the same spelling with
        // spaces where the path has underscores.
        assert_eq!(
            hits[1].url,
            "https://en.wikipedia.org/wiki/Hopper_(microarchitecture)"
        );
    }

    #[test]
    fn an_answer_with_no_pages_is_nothing_to_show_and_not_a_failure() {
        assert!(parse_hits(&serde_json::json!({})).is_empty());
        assert!(parse_hits(&serde_json::json!({ "pages": [] })).is_empty());
        assert!(search("   ", 5).is_err(), "no words is not a query");
    }

    #[test]
    fn a_file_address_is_refused_before_anything_is_sent() {
        let err = read_page("file:///etc/passwd", 100).unwrap_err();
        assert!(matches!(err, WebError::Refused(_)), "{err:?}");
        assert!(err.to_string().contains("file"), "{err}");
        let err = read_page("not a url at all", 100).unwrap_err();
        assert!(matches!(err, WebError::Refused(_)), "{err:?}");
    }

    #[test]
    fn this_machines_own_addresses_are_refused() {
        for url in [
            "http://127.0.0.1:8080/health",
            "http://localhost:3000/",
            "http://10.0.0.5/",
            "http://192.168.1.1/admin",
            "http://169.254.169.254/latest/meta-data/",
            "http://[::1]/",
            "http://[fd00::1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://100.64.0.1/",
        ] {
            let err = guard(&Url::parse(url).unwrap())
                .expect_err("{url} points at this machine and must not be reachable");
            assert!(matches!(err, WebError::Refused(_)), "{url}: {err:?}");
        }
    }

    #[test]
    fn a_public_address_passes_the_same_check() {
        // A literal, so the assertion is about the guard and not about whether this machine can reach
        // a name server. What a public host *used to be* — example.com's old address — and no packet.
        assert!(guard(&Url::parse("https://93.184.216.34/example").unwrap()).is_ok());
        assert!(!is_private(IpAddr::from([8, 8, 8, 8])));
        assert!(!is_private(IpAddr::from([1, 1, 1, 1])));
    }

    #[test]
    fn only_text_is_handed_to_the_model() {
        assert!(reads_as_text("text/html; charset=utf-8"));
        assert!(reads_as_text("application/json"));
        assert!(reads_as_text("application/feed+json"));
        // A PDF or a picture is not a page this build can read, and saying so beats streaming bytes
        // that the stripper would turn into nothing.
        assert!(!reads_as_text("application/pdf"));
        assert!(!reads_as_text("image/png"));
    }

    #[test]
    fn a_page_becomes_prose_with_no_markup_left() {
        let html = r#"<!DOCTYPE html><html><head><title>Grace</title>
             <script>var tracking = "beacon";</script>
             <style>p { color: red }</style></head>
             <body><h1>Grace Hopper</h1><p>She invented <b>COBOL</b>&rsquo;s compiler.</p>
             <ul><li>1906 &ndash; 1992</li><li>Rear&nbsp;Admiral</li></ul>
             <!-- a comment nobody asked for --><p>Done.</p></body></html>"#;
        let text = html_to_text(html);
        assert!(!text.contains('<'), "{text}");
        // A dropped block's own closing bracket is the one character that can survive it, and a page
        // full of scripts and menus then reads as a column of `>` lines above the prose.
        assert!(!text.contains('>'), "{text}");
        assert!(!text.contains("tracking"), "the script survived");
        assert!(!text.contains("color: red"), "the style survived");
        assert!(!text.contains("nobody asked"), "the comment survived");
        assert!(text.contains("Grace Hopper\n"), "{text}");
        assert!(text.contains("She invented COBOL’s compiler."), "{text}");
        assert!(text.contains("1906 – 1992\n"), "{text}");
        assert!(text.contains("Rear Admiral"), "{text}");
        assert!(!text.contains("\n\n\n"), "{text}");
    }

    /// A comment's own terminator is `-->`, not the next bracket. Conditional IE comments carry
    /// several, and stopping at the first one left a bare `>` in the prose — the first thing the live
    /// provider probe showed.
    #[test]
    fn a_comment_holding_a_bracket_leaks_nothing_into_the_prose() {
        let html = "<body><p>Real text.</p><!--[if IE]><b>ancient</b><![endif]-->\
             <!-- a > b, said quietly --><p>More.</p></body>";
        let text = html_to_text(html);
        assert!(!text.contains('<') && !text.contains('>'), "{text}");
        assert!(
            !text.contains("ancient") && !text.contains("quietly"),
            "{text}"
        );
        assert!(
            text.contains("Real text.") && text.contains("More."),
            "{text}"
        );
    }

    /// A page's menus are not its content, and the character budget makes the difference matter: the
    /// first lines of a Wikipedia article used to be "Jump to content / Main menu / Navigation", all
    /// of which the model paid its context for and none of which it could answer from.
    #[test]
    fn a_pages_menus_are_dropped_before_the_prose_is_kept() {
        let html = "<body><header class=\"mw-header\"><nav><ul><li>Main page</li><li>Random article</li>\
             </ul></nav></header><aside>Hide</aside><div id=\"content\"><p>Rust is a multi-paradigm \
             language.</p></div><footer>a license notice</footer></body>";
        let text = html_to_text(html);
        assert!(
            text.contains("Rust is a multi-paradigm language."),
            "{text}"
        );
        assert!(!text.contains("Main page"), "{text}");
        assert!(!text.contains("Random article"), "{text}");
        assert!(!text.contains("Hide"), "{text}");
        assert!(!text.contains("license notice"), "{text}");
    }

    #[test]
    fn an_unknown_entity_keeps_its_text_and_a_numeric_one_becomes_a_character() {
        assert_eq!(decode_entities("a &xyz; b"), "a &xyz; b");
        assert_eq!(decode_entities("&#8212; and &#x2014;"), "— and —");
        assert_eq!(decode_entities("unclosed & amp"), "unclosed & amp");
        assert_eq!(decode_entities("&#55357;&#56578;"), "&#55357;&#56578;");
    }

    #[test]
    fn a_plain_text_page_is_passed_through_not_stripped() {
        assert!(!looks_like_markup("just a log line\nwith <less than in it"));
        assert!(looks_like_markup("<!doctype html><p>hi"));
        assert!(looks_like_markup("<html><body>x"));
    }

    #[test]
    fn a_bracket_inside_a_quoted_attribute_does_not_end_the_tag() {
        // Parsoid's own annotation, in the shape the live provider serves it.
        let html = "<body><p about=\"#mwt1\" data-mw='{\"source\":\"<link rel=x />\"}' \
             class=\"mw-parser-output\">Rust is a language.</p></body>";
        let text = html_to_text(html);
        assert_eq!(text, "Rust is a language.", "{text}");
    }

    /// The label a row shows is the page's own name, not whatever its body happens to open with: a
    /// Wikipedia article begins with its skip links, so the first line read as a page about "Jump to
    /// content" rather than about the thing the model asked for.
    #[test]
    fn a_page_titles_itself_rather_than_by_its_first_line() {
        let html = "<html><head><title>Rust (programming language) - Wikipedia</title>\
             <script>var tracking = 1;</script></head><body><div>Jump to content</div>\
             <p>Rust is a language.</p></body></html>";
        assert_eq!(
            head_title(html).as_deref(),
            Some("Rust (programming language) - Wikipedia")
        );
        assert_eq!(head_title("<html><body><p>no head</p></body>"), None);
        assert_eq!(
            head_title("<html><head><title>   </title></head></html>"),
            None
        );
        // `<titles>` is not `<title>`.
        assert_eq!(head_title("<p><titles>x</titles></p>"), None);
        let url = Url::parse("https://example.org/a").unwrap();
        assert_eq!(
            line_title("\n\n  Grace Hopper  \nnext", &url),
            "Grace Hopper"
        );
        assert_eq!(line_title("", &url), "https://example.org/a");
    }

    #[test]
    fn the_private_ranges_are_checked_by_hand_where_the_standard_library_cannot() {
        // CGNAT and 0/8 are the two the stable predicates do not cover, and both are how a container
        // reaches its own host.
        assert!(is_private(IpAddr::from([100, 64, 0, 1])));
        assert!(is_private(IpAddr::from([100, 127, 255, 255])));
        assert!(!is_private(IpAddr::from([100, 128, 0, 1])));
        assert!(is_private(IpAddr::from([0, 0, 0, 0])));
        assert!(is_private(IpAddr::from([192, 168, 0, 7])));
        assert!(!is_private(IpAddr::from([172, 32, 0, 1])));
        assert!(is_private(IpAddr::from([172, 16, 0, 1])));
    }

    /// The one part of this module no test can reach: whether the endpoint answers the way the
    /// parser assumes it does. It leaves the machine, so it stays ignored — run it by hand with
    /// `cargo test -p smollm-tools -- --ignored` after any change to the request, the guard or the
    /// shape of a hit.
    #[test]
    #[ignore]
    fn the_provider_answers_in_the_shape_this_module_reads_it() {
        let hits = search("Rust programming language", 3).unwrap();
        assert!(!hits.is_empty(), "the search returned no pages");
        let first = &hits[0];
        assert!(
            first.url.starts_with("https://en.wikipedia.org/"),
            "{first:?}"
        );
        assert!(
            !first.title.is_empty() && !first.snippet.is_empty(),
            "{first:?}"
        );

        let page = read_page(&first.url, DEFAULT_PAGE_CHARS).unwrap();
        assert!(
            page.text.chars().count() > 200,
            "the page came back nearly empty"
        );
        assert!(
            page.title.contains("Rust"),
            "the row's label came out as {:?}",
            page.title
        );
        let opening: String = page.text.chars().take(200).collect();
        // Run by hand, so the person running it can read what the model would be handed rather than
        // guess from a green line.
        eprintln!(
            "{title} — {url}\n{opening}\n…",
            title = page.title,
            url = page.url
        );
        assert!(
            !page.text.contains('<') && !page.text.contains("class="),
            "markup survived the strip: {opening}"
        );
    }
}
