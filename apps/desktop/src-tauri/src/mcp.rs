//! MCP: the servers this machine declares, and what each one says it can do.
//!
//! The Model Context Protocol is how a tool that is not part of this build reaches a run at all. A
//! skill (#240) adds instructions and grants nothing; a server claims the opposite — it publishes
//! calls, and the whole point of it is that the app did not write them. That is why this file stops
//! short of the run.
//!
//! Servers are declared in one place, `~/.freecode/mcp.json`, in the shape the ecosystem already
//! pastes around:
//!
//! ```json
//! { "mcpServers": { "weather": { "command": "weather-mcp", "args": ["--port", "8080"],
//!   "env": { "API_KEY": "…" }, "disabled": false } } }
//! ```
//!
//! Machine-only on purpose. A project folder is a thing you open, not a thing you agreed to run, and
//! an MCP server is an arbitrary command: a `.freecode/mcp.json` in a cloned repository would be
//! executed by whoever looked at its Extensions page. The skills scan has no such edge because it
//! reads files rather than running them.
//!
//! And what a check does here is *ask*, not *use*: spawn the command, complete the handshake, take
//! `tools/list`, and shut the child down. The page can then show what a server publishes and name the
//! one thing this build cannot yet do with it — the free route's calls are run by the service itself
//! ([`crate::opencode`]'s ruleset), so a tool this app declares has no loop to be offered from. The
//! row says so rather than letting a green light imply a capability.

use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

/// How long one answer from a server may take. A check runs while the page waits on it, so a server
/// that never answers has to become a sentence rather than a window that never finishes.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(10);

/// The newest protocol version this client speaks. A server answers with the version it will use,
/// which is reported rather than assumed, because the row is where a mismatch becomes visible.
const PROTOCOL_VERSION: &str = "2025-03-26";

/// How many lines a reply may be buried under. A server that streams notifications forever is a
/// server that did not answer, and the count is what turns that into an error instead of a hang.
const MAX_LINES_BEFORE_REPLY: usize = 256;

/// How many pages of tools one check reads. The protocol paginates; a server that keeps handing back
/// a cursor is not going to finish, and the page has to say that.
const MAX_PAGES: usize = 20;

/// The tools one server may report before the check stops counting. A row that listed four hundred
/// names would be a wall of text nobody reads, and the count is in it either way.
const MAX_TOOLS: usize = 200;

/// The longest piece of a server's own text this side repeats back. A refusal has to be readable, and
/// a misbehaving server must not be able to spend the window's budget on one line.
const QUOTED_CHARS: usize = 200;

/// One declared server, exactly as the file states it.
///
/// The environment is carried as key names only. A declaration file is where a token goes, and the
/// page's job is to show the command a press would run — which needs the program and its arguments
/// and never needs the secret's value.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    /// The key under `mcpServers`, which is the name the user calls it by.
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    /// The names of the variables the declaration sets, never their values.
    pub env_keys: Vec<String>,
    pub disabled: bool,
    /// What the declaration says it is. Only `"stdio"` — a command's stdin and stdout — is spoken
    /// here; a URL-shaped server is listed so the page can name the transport this build lacks.
    pub transport: String,
    /// Whether a press would try to run this one. False is a refusal with a reason already in
    /// [`transport`] or [`disabled`], not a failure waiting to happen.
    pub runnable: bool,
}

/// Everything the page lists before anything is checked.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inventory {
    pub servers: Vec<Server>,
    /// The file these came from, so an empty section can say where to put one and a filled one can
    /// say which file a mistake is in.
    pub path: String,
    /// The one sentence about what went wrong: a file that could not be read, a file that is not
    /// JSON, or the entries that had to be skipped. It rides with a non-empty list rather than in
    /// place of one, because the page's job is to say "these two are declared, this one was not
    /// understood" instead of showing a clean list over a broken file.
    pub error: String,
}

/// One call a server publishes.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    pub name: String,
    /// The server's own display name for it, when it gave one.
    pub title: String,
    pub description: String,
    /// Whether this build already has a call of that name. A collision is not fatal today — nothing
    /// is routed from here to a run — but it is the thing a user has to see before a later build lets
    /// a server answer for a call the app means to own.
    pub collides: bool,
}

/// What one server answered.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub name: String,
    /// The version the server said it will speak.
    pub protocol: String,
    /// The server's own name and version, from its `serverInfo`.
    pub server_name: String,
    pub server_version: String,
    pub tools: Vec<Tool>,
}

impl Tool {
    /// One entry of a `tools/list` result. A missing description is left empty rather than invented:
    /// the row shows the name, and a made-up sentence about someone else's tool is worse than none.
    fn declared(value: &Value) -> Self {
        let name = text(value, "name");
        Self {
            collides: name.is_empty() || freecode_tools::find(&name).is_some(),
            title: text(value, "title"),
            description: text(value, "description"),
            name,
        }
    }
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// The file's location: this machine's `.freecode`, beside the folder skills are read from.
pub fn declaration_file() -> Option<PathBuf> {
    crate::skills::machine_home().map(|home| home.join(".freecode").join("mcp.json"))
}

/// What the page shows: the declared servers, and the one sentence about the file if it could not be
/// read. A file that is simply not there is the ordinary state, not an error — the empty section says
/// where to put one.
pub fn inventory() -> Inventory {
    let Some(path) = declaration_file() else {
        return Inventory {
            servers: Vec::new(),
            path: String::new(),
            error: String::new(),
        };
    };
    let shown = path.display().to_string();
    match std::fs::read_to_string(&path) {
        Ok(text) => view(&text, &shown),
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => Inventory {
            servers: Vec::new(),
            path: shown,
            error: String::new(),
        },
        Err(why) => {
            let error = format!("{shown} could not be read: {why}");
            Inventory {
                servers: Vec::new(),
                path: shown,
                error,
            }
        }
    }
}

/// What the page shows for one file's text. Split from [`inventory`] so the half-read case — a broken
/// entry beside good ones — has a test that does not need to be this machine's home folder.
fn view(text: &str, shown: &str) -> Inventory {
    let (servers, error) = match parse(text) {
        Ok((servers, skipped)) => (servers, skipped.join("; ")),
        Err(why) => (Vec::new(), format!("{shown}: {why}")),
    };
    Inventory {
        servers,
        path: shown.to_string(),
        error,
    }
}

/// The servers named in the file's text, in name order so the list does not shuffle when a key is
/// added, and one sentence per entry that had to be skipped. A file with three good servers and one
/// broken one lists the three and says what was skipped.
fn parse(text: &str) -> Result<(Vec<Server>, Vec<String>), String> {
    let root: Value =
        serde_json::from_str(text).map_err(|why| format!("the file is not JSON: {why}"))?;
    let declared = match root.get("mcpServers") {
        Some(Value::Object(map)) => map,
        Some(Value::Null) | None => return Ok((Vec::new(), Vec::new())),
        Some(_) => return Err("\"mcpServers\" is not an object of servers".into()),
    };
    let mut names: Vec<&String> = declared.keys().collect();
    names.sort();
    let mut out = Vec::with_capacity(names.len());
    let mut skipped = Vec::new();
    for name in names {
        let entry = &declared[name];
        match server(name, entry) {
            Ok(server) => out.push(server),
            Err(why) => skipped.push(format!("{name}: {why}")),
        }
    }
    Ok((out, skipped))
}

/// One entry of `mcpServers`.
fn server(name: &str, entry: &Value) -> Result<Server, String> {
    if !entry.is_object() {
        return Err("not an object".into());
    }
    let transport = match text(entry, "type").as_str() {
        "" => "stdio".into(),
        other => other.to_string(),
    };
    let command = text(entry, "command");
    let args: Vec<String> = entry
        .get("args")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .map(|one| match one {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    let mut env_keys: Vec<String> = entry
        .get("env")
        .and_then(Value::as_object)
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default();
    env_keys.sort();
    let disabled = entry
        .get("disabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    // A URL-shaped server is a real declaration this build cannot honour, and naming it is more
    // useful than pretending the entry is empty.
    let stdio = transport == "stdio";
    if !stdio && text(entry, "url").is_empty() && command.is_empty() {
        return Err(format!("neither a command nor a url (type {transport})"));
    }
    if stdio && command.is_empty() {
        return Err("a stdio server needs a command".into());
    }
    Ok(Server {
        runnable: stdio && !command.is_empty() && !disabled,
        transport,
        name: name.to_string(),
        command,
        args,
        env_keys,
        disabled,
    })
}

/// Ask one declared server what it publishes, then shut it down.
///
/// The child is killed on every path, and `kill_on_drop` is the backstop for a future early return
/// that forgets to: an MCP server that outlives the check that started it is a process with no owner.
pub async fn check(name: &str) -> Result<Report, String> {
    let server = inventory()
        .servers
        .into_iter()
        .find(|declared| declared.name == name)
        .ok_or_else(|| format!("no server named {name} is declared"))?;
    if let Some(reason) = refusal(&server) {
        return Err(reason);
    }
    ask_server(&server).await
}

/// Why a press would refuse this server, or nothing when it would start it.
///
/// The page reads [`Server::runnable`] for the same answer, and the host checks again on the press,
/// because a file the user edits between the two is the one case the row cannot know about.
fn refusal(server: &Server) -> Option<String> {
    if server.disabled {
        return Some(format!(
            "{} is switched off in the declaration file",
            server.name
        ));
    }
    if !server.runnable {
        return Some(format!(
            "{} is a {} server; this build speaks MCP over a command's stdin",
            server.name, server.transport
        ));
    }
    None
}

/// The handshake and `tools/list` against a server this process starts and always stops.
///
/// Split from [`check`] so a test can hand it a server of its own instead of writing to this
/// machine's declaration file.
async fn ask_server(server: &Server) -> Result<Report, String> {
    let (pipes, mut child) = spawn(server)?;
    let mut client = Client::new(pipes);
    let outcome = report(&mut client, &server.name).await;
    let _ = child.kill().await;
    let _ = child.wait().await;
    outcome.map_err(|why| format!("{}: {why}", server.name))
}

/// `serverInfo`, or an object with nothing in it when the server did not send one.
fn child_info(info: &Value) -> Value {
    info.get("serverInfo").cloned().unwrap_or_else(|| json!({}))
}

/// The two ends of a server process, as the client sees them.
///
/// A trait rather than the concrete child type so the protocol can be tested against an in-memory
/// pair of pipes: the same request/response code is what runs against a real command, which is the
/// only way the framing gets checked without installing somebody else's server to test against.
pub trait ChildPipes {
    fn reader(&mut self) -> &mut BufReader<Box<dyn AsyncRead + Unpin + Send>>;
    fn writer(&mut self) -> &mut Box<dyn AsyncWrite + Unpin + Send>;
}

struct Process {
    stdin: Box<dyn AsyncWrite + Unpin + Send>,
    stdout: BufReader<Box<dyn AsyncRead + Unpin + Send>>,
}

impl ChildPipes for Process {
    fn reader(&mut self) -> &mut BufReader<Box<dyn AsyncRead + Unpin + Send>> {
        &mut self.stdout
    }

    fn writer(&mut self) -> &mut Box<dyn AsyncWrite + Unpin + Send> {
        &mut self.stdin
    }
}

/// Start a declared server. A command that cannot be found is reported as that, in the words the OS
/// used, because "the server did not answer" would send the user looking at the wrong thing.
fn spawn(server: &Server) -> Result<(Process, Child), String> {
    let mut command = Command::new(&server.command);
    command
        .args(&server.args)
        // The declaration's own variables, on top of the environment the app already has.
        .envs(declared_env(&server.name))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        // The server's own log is not this page's surface, and a pipe nobody drains is a server that
        // blocks writing to it. Null is the choice that cannot hang.
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|why| format!("{} could not be started: {why}", server.command))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| format!("{} gave us no stdin to write to", server.command))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("{} gave us no stdout to read", server.command))?;
    Ok((
        Process {
            stdin: Box::new(stdin),
            stdout: BufReader::new(Box::new(stdout)),
        },
        child,
    ))
}

/// The `env` the file gives this server, read fresh from disk.
///
/// [`Server`] crosses to the page, so it carries the key names only; a server that needs a token
/// still has to be handed one, and the file is the only place that value belongs. Reading it here
/// rather than keeping it on the struct means a declaration's secret has no path to the webview.
fn declared_env(name: &str) -> Vec<(String, String)> {
    let Some(entry) = declared_entry(name) else {
        return Vec::new();
    };
    let Some(map) = entry.get("env").and_then(Value::as_object) else {
        return Vec::new();
    };
    map.iter()
        .map(|(key, value)| {
            let text = match value {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            (key.clone(), text)
        })
        .collect()
}

/// One entry of the file's `mcpServers`, by name, or nothing when there is no file to read it from.
fn declared_entry(name: &str) -> Option<Value> {
    let path = declaration_file()?;
    let text = std::fs::read_to_string(path).ok()?;
    let root: Value = serde_json::from_str(&text).ok()?;
    root.get("mcpServers")?.get(name).cloned()
}

/// A JSON-RPC 2.0 client over one server's newline-delimited stdio.
pub struct Client<C> {
    pipes: C,
    next_id: i64,
}

impl<C: ChildPipes> Client<C> {
    pub fn new(pipes: C) -> Self {
        Self { pipes, next_id: 1 }
    }

    /// The `initialize` handshake, and the `initialized` notification that tells the server it may
    /// start working. The result is the server's own answer, so the caller reports the version the
    /// server chose rather than the one we offered.
    pub async fn initialize(&mut self, version: &str) -> Result<Value, String> {
        let info = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": "FreeCode", "version": version },
                }),
            )
            .await?;
        self.notify("notifications/initialized", json!({})).await?;
        Ok(info)
    }

    /// Every tool the server publishes, following its cursors.
    pub async fn tools(&mut self) -> Result<Vec<Tool>, String> {
        let mut out: Vec<Tool> = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let params = match &cursor {
                Some(at) => json!({ "cursor": at }),
                None => json!({}),
            };
            let result = self.request("tools/list", params).await?;
            let page = result
                .get("tools")
                .and_then(Value::as_array)
                .ok_or("tools/list answered without a tools list")?;
            for entry in page {
                if out.len() == MAX_TOOLS {
                    return Ok(out);
                }
                out.push(Tool::declared(entry));
            }
            cursor = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_string);
            if cursor.is_none() {
                return Ok(out);
            }
        }
        Err(format!(
            "still had more tools after {MAX_PAGES} pages, so the count stopped there"
        ))
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await?;
        self.reply(method, id).await
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}))
            .await
    }

    /// One message, then the newline the protocol frames on.
    async fn send(&mut self, message: Value) -> Result<(), String> {
        let line = serde_json::to_string(&message)
            .map_err(|why| format!("the request could not be written: {why}"))?;
        let writer = self.pipes.writer();
        writer
            .write_all(line.as_bytes())
            .await
            .map_err(|why| format!("the server's stdin refused the write: {why}"))?;
        writer
            .write_all(b"\n")
            .await
            .map_err(|why| format!("the server's stdin refused the write: {why}"))?;
        writer
            .flush()
            .await
            .map_err(|why| format!("the server's stdin refused the flush: {why}"))
    }

    /// Read until the answer to `id` arrives.
    ///
    /// A server may push notifications whenever it likes, and the protocol says so, so a line that is
    /// not ours is skipped rather than treated as the reply. What is not skipped is a line that is not
    /// JSON at all: the stdio transport reserves stdout for the protocol, and a server that prints a
    /// banner there has broken the one rule that makes it safe to read.
    async fn reply(&mut self, method: &str, id: i64) -> Result<Value, String> {
        for _ in 0..MAX_LINES_BEFORE_REPLY {
            let mut line = String::new();
            let read =
                tokio::time::timeout(RESPONSE_TIMEOUT, self.pipes.reader().read_line(&mut line))
                    .await
                    .map_err(|_| {
                        format!(
                            "{method} did not answer within {} seconds",
                            RESPONSE_TIMEOUT.as_secs()
                        )
                    })?
                    .map_err(|why| format!("{method} could not be read: {why}"))?;
            if read == 0 {
                return Err(format!(
                    "{method} was never answered — the server closed its stdout"
                ));
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
                return Err(format!(
                    "the server put non-protocol text on stdout: {}",
                    clip(trimmed)
                ));
            };
            if value.get("id").and_then(Value::as_i64) != Some(id) {
                continue;
            }
            if let Some(error) = value.get("error") {
                return Err(format!(
                    "{method} refused: {}",
                    clip(&text(error, "message"))
                ));
            }
            return Ok(value.get("result").cloned().unwrap_or_else(|| json!({})));
        }
        Err(format!(
            "{method} was still not answered after {MAX_LINES_BEFORE_REPLY} lines from the server"
        ))
    }
}

/// Ask a server we already started. Split out so the in-memory tests drive the same `Client` a real
/// command does.
pub async fn report<C: ChildPipes>(client: &mut Client<C>, name: &str) -> Result<Report, String> {
    let info = client.initialize(env!("CARGO_PKG_VERSION")).await?;
    let tools = client.tools().await?;
    Ok(Report {
        name: name.to_string(),
        protocol: text(&info, "protocolVersion"),
        server_name: text(&child_info(&info), "name"),
        server_version: text(&child_info(&info), "version"),
        tools,
    })
}

fn clip(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= QUOTED_CHARS {
        return text.to_string();
    }
    chars[..QUOTED_CHARS].iter().collect::<String>() + "…"
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::DuplexStream;

    fn servers(text: &str) -> Vec<Server> {
        parse(text).unwrap().0
    }

    /// A declaration built by hand, for the tests that drive a process rather than the file.
    #[cfg(unix)]
    fn stdio(name: &str, command: &str, args: Vec<String>) -> Server {
        Server {
            name: name.to_string(),
            command: command.to_string(),
            args,
            env_keys: Vec::new(),
            disabled: false,
            transport: "stdio".to_string(),
            runnable: true,
        }
    }

    #[test]
    fn names_are_listed_in_order_so_the_page_does_not_shuffle() {
        let rows = servers(
            r#"{"mcpServers":{"weather":{"command":"w"},"alpha":{"command":"a"},"mid":{"command":"m"}}}"#,
        );
        assert_eq!(
            rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["alpha", "mid", "weather"]
        );
    }

    #[test]
    fn a_token_stays_in_the_file() {
        // The declaration is where an API key goes, and the row shows the command a press would run, so
        // the only part of the entry the page is allowed to see is the name of the variable.
        let rows = servers(
            r#"{"mcpServers":{"weather":{"command":"w","env":{"API_KEY":"sk-secret","REGION":"eu"}}}}"#,
        );
        assert_eq!(rows[0].env_keys, ["API_KEY", "REGION"]);
        let shown = serde_json::to_string(&rows).unwrap();
        assert!(!shown.contains("sk-secret"), "{shown}");
    }

    #[test]
    fn a_stdio_entry_without_a_command_is_skipped_with_a_reason() {
        let (rows, skipped) = parse(r#"{"mcpServers":{"ghost":{}}}"#).unwrap();
        assert!(
            rows.is_empty(),
            "an entry with nothing to run is not a server"
        );
        assert_eq!(skipped, ["ghost: a stdio server needs a command"]);
    }

    #[test]
    fn a_url_server_is_listed_but_would_not_be_started() {
        let rows =
            servers(r#"{"mcpServers":{"remote":{"type":"http","url":"https://mcp.example/a"}}}"#);
        assert_eq!(rows[0].transport, "http");
        assert!(!rows[0].runnable);
        assert!(refusal(&rows[0]).unwrap().contains("stdin"));
    }

    #[test]
    fn a_switched_off_server_is_refused_by_name() {
        let rows = servers(r#"{"mcpServers":{"off":{"command":"x","disabled":true}}}"#);
        assert!(rows[0].disabled);
        assert!(!rows[0].runnable, "a press would not start it");
        assert!(refusal(&rows[0]).unwrap().contains("switched off"));
    }

    #[test]
    fn a_half_broken_file_lists_the_good_rows_and_names_the_rest() {
        let view = view(
            r#"{"mcpServers":{"good":{"command":"x"},"broken":{"type":"weird"}}}"#,
            "/home/u/.freecode/mcp.json",
        );
        assert_eq!(view.servers.len(), 1, "the good entry is still listed");
        assert_eq!(view.path, "/home/u/.freecode/mcp.json");
        assert!(view.error.contains("broken"), "{}", view.error);
    }

    #[test]
    fn a_file_that_is_not_json_is_reported_with_the_file_that_it_is() {
        let view = view("{ mcpServers: ", "/home/u/.freecode/mcp.json");
        assert!(view.servers.is_empty());
        assert!(
            view.error.starts_with("/home/u/.freecode/mcp.json"),
            "{}",
            view.error
        );
        assert!(view.error.contains("not JSON"), "{}", view.error);
    }

    #[test]
    fn a_server_that_claims_a_call_this_app_already_owns_is_flagged() {
        assert!(Tool::declared(&json!({"name":"list_dir"})).collides);
        assert!(!Tool::declared(&json!({"name":"get_forecast"})).collides);
        // A nameless entry cannot be routed either, so it is the same flag rather than a silent row.
        assert!(Tool::declared(&json!({"description":"no name at all"})).collides);
    }

    #[tokio::test]
    async fn an_undeclared_name_is_said_as_that() {
        // Nothing on any machine is named like this, so the answer does not depend on what the file
        // beside this test's home folder happens to hold.
        let why = check("no-such-server-anywhere").await.unwrap_err();
        assert!(why.contains("no server named"), "{why}");
    }

    /// A scripted server at the far end of an in-memory pipe pair.
    ///
    /// Driven from the same `join!` as the client, so a server that pushes a notification between two
    /// requests or answers a request nobody made is a case the framing has to survive rather than a test
    /// that waits out the timeout.
    struct Peer {
        inbox: BufReader<DuplexStream>,
        outbox: Option<DuplexStream>,
    }

    /// The client's half of a pair, and the peer that answers it.
    fn pair() -> (Client<Process>, Peer) {
        let (to_peer, peer_reads) = tokio::io::duplex(4096);
        let (peer_writes, to_client) = tokio::io::duplex(4096);
        (
            Client::new(Process {
                stdin: Box::new(to_peer),
                stdout: BufReader::new(Box::new(to_client)),
            }),
            Peer {
                inbox: BufReader::new(peer_reads),
                outbox: Some(peer_writes),
            },
        )
    }

    impl Peer {
        /// The next request carrying `method`, skipping the notifications the protocol allows a server
        /// to push whenever it likes.
        async fn request(&mut self, method: &str) -> (i64, Value) {
            loop {
                let line = self.read().await;
                let Some(id) = line.get("id").and_then(Value::as_i64) else {
                    continue;
                };
                assert_eq!(
                    line.get("method").and_then(Value::as_str),
                    Some(method),
                    "the client asked for something else"
                );
                return (id, line.get("params").cloned().unwrap_or(Value::Null));
            }
        }

        /// The next id-less message, which is what a notification is.
        async fn notification(&mut self, method: &str) {
            loop {
                let line = self.read().await;
                if line.get("id").is_some() {
                    continue;
                }
                assert_eq!(line.get("method").and_then(Value::as_str), Some(method));
                return;
            }
        }

        async fn read(&mut self) -> Value {
            let mut line = String::new();
            let read = self.inbox.read_line(&mut line).await.unwrap();
            assert!(read > 0, "the client closed its stdin mid-script");
            serde_json::from_str(line.trim()).unwrap_or_else(|why| {
                panic!("the client put non-protocol text on stdin ({why}): {line:?}")
            })
        }

        async fn reply(&mut self, id: i64, result: Value) {
            self.push(json!({"jsonrpc": "2.0", "id": id, "result": result}))
                .await;
        }

        async fn refuse(&mut self, id: i64, message: &str) {
            self.push(json!({
                "jsonrpc": "2.0", "id": id,
                "error": {"code": -32601, "message": message},
            }))
            .await;
        }

        /// One message of the test's own, for the lines a scripted reply cannot express.
        async fn push(&mut self, message: Value) {
            self.push_text(&message.to_string()).await;
        }

        async fn push_text(&mut self, text: &str) {
            let stream = self.outbox.as_mut().expect("the peer has hung up");
            stream.write_all(text.as_bytes()).await.unwrap();
            stream.write_all(b"\n").await.unwrap();
            stream.flush().await.unwrap();
        }

        /// Close the answer side while keeping the request side open, which is a server that shuts up
        /// without exiting.
        fn hang_up(&mut self) {
            self.outbox = None;
        }
    }

    #[tokio::test]
    async fn a_check_reads_the_handshake_and_every_page_of_tools() {
        let (mut client, mut peer) = pair();
        let script = async {
            let (id, params) = peer.request("initialize").await;
            assert_eq!(params["protocolVersion"], json!(PROTOCOL_VERSION));
            assert_eq!(params["clientInfo"]["name"], json!("FreeCode"));
            // A version the client did not offer: the row reports what the server chose.
            peer.reply(
                id,
                json!({
                    "protocolVersion": "2024-11-05",
                    "serverInfo": {"name": "Weather MCP", "version": "1.4.0"},
                }),
            )
            .await;
            peer.notification("notifications/initialized").await;
            let (id, _) = peer.request("tools/list").await;
            peer.reply(
                id,
                json!({"tools": [{"name": "get_forecast", "description": "One day."}], "nextCursor": "p2"}),
            )
            .await;
            let (id, params) = peer.request("tools/list").await;
            assert_eq!(params["cursor"], json!("p2"), "the cursor is sent back");
            // Both things a well-behaved server is allowed to put where an answer belongs.
            peer.push(json!({"jsonrpc":"2.0","method":"notifications/message"}))
                .await;
            peer.push(json!({"jsonrpc":"2.0","id":99,"result":{}}))
                .await;
            peer.reply(id, json!({"tools": [{"name": "radar", "title": "Radar"}]}))
                .await;
        };
        let (outcome, ()) = tokio::join!(report(&mut client, "weather"), script);
        let report = outcome.unwrap();
        assert_eq!(report.name, "weather");
        assert_eq!(report.protocol, "2024-11-05");
        assert_eq!(report.server_name, "Weather MCP");
        assert_eq!(report.server_version, "1.4.0");
        let tools: Vec<(&str, bool)> = report
            .tools
            .iter()
            .map(|t| (t.name.as_str(), t.collides))
            .collect();
        assert_eq!(tools, [("get_forecast", false), ("radar", false)]);
    }

    #[tokio::test]
    async fn a_refused_request_becomes_the_server_s_own_words() {
        let (mut client, mut peer) = pair();
        let script = async {
            let (id, _) = peer.request("initialize").await;
            peer.refuse(id, "this server needs a licence").await;
        };
        let (outcome, ()) = tokio::join!(report(&mut client, "licensed"), script);
        let why = outcome.unwrap_err();
        assert!(why.contains("refused"), "{why}");
        assert!(why.contains("needs a licence"), "{why}");
    }

    #[tokio::test]
    async fn text_on_stdout_that_is_not_the_protocol_is_named_instead_of_hanging() {
        // The stdio transport reserves stdout for the protocol. A server that prints a banner there has
        // broken the one rule that makes it safe to read, and the page has to say so.
        let (mut client, mut peer) = pair();
        let script = async { peer.push_text("Welcome to the weather server v3!").await };
        let (outcome, ()) = tokio::join!(report(&mut client, "banner"), script);
        let why = outcome.unwrap_err();
        assert!(why.contains("non-protocol text"), "{why}");
        assert!(why.contains("Welcome to the weather server"), "{why}");
    }

    #[tokio::test]
    async fn a_server_that_closes_its_stdout_is_said_as_that() {
        let (mut client, mut peer) = pair();
        peer.hang_up();
        let why = report(&mut client, "silent").await.unwrap_err();
        assert!(why.contains("closed its stdout"), "{why}");
    }

    #[tokio::test]
    async fn a_page_of_tools_that_never_ends_stops_at_the_cap() {
        let (mut client, mut peer) = pair();
        let script = async {
            let (id, _) = peer.request("initialize").await;
            peer.reply(id, json!({"protocolVersion": PROTOCOL_VERSION}))
                .await;
            peer.notification("notifications/initialized").await;
            for _ in 0..MAX_PAGES {
                let (id, _) = peer.request("tools/list").await;
                peer.reply(
                    id,
                    json!({"tools": [{"name": "get_forecast"}], "nextCursor": "again"}),
                )
                .await;
            }
        };
        let (outcome, ()) = tokio::join!(report(&mut client, "endless"), script);
        // The handshake names no server, so the first page is where the counting starts.
        let why = outcome.unwrap_err();
        assert!(why.contains("still had more tools"), "{why}");
    }

    /// A fake MCP server that speaks the protocol over its own stdin and stdout, as a shell loop.
    #[cfg(unix)]
    const FAKE_SERVER: &str = r#"
while IFS= read -r line; do
  id=$(expr "$line" : '.*"id":\([0-9][0-9]*\)')
  case "$line" in
    *initialized*) : ;;
    *initialize*) printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-03-26","serverInfo":{"name":"sh-fake","version":"0.1"}}}\n' "$id" ;;
    *tools/list*) printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"list_dir","description":"A call this app owns."}]}}\n' "$id" ;;
  esac
done
"#;

    /// The same client against a real command rather than an in-memory pair: the pipes, the framing and
    /// the shutdown are what the Extensions page's Check press does.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_real_command_answers_over_its_stdio() {
        let server = stdio("sh-fake", "/bin/sh", vec!["-c".into(), FAKE_SERVER.into()]);
        let report = ask_server(&server).await.unwrap();
        assert_eq!(report.name, "sh-fake");
        assert_eq!(report.protocol, PROTOCOL_VERSION);
        assert_eq!(report.server_name, "sh-fake");
        assert_eq!(report.server_version, "0.1");
        assert_eq!(report.tools[0].name, "list_dir");
        assert!(
            report.tools[0].collides,
            "list_dir is one of this app's own calls"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_command_that_is_not_there_is_reported_as_that() {
        let server = stdio("absent", "definitely-not-an-installed-mcp-server", vec![]);
        let why = ask_server(&server).await.unwrap_err();
        assert!(why.contains("could not be started"), "{why}");
        assert!(
            why.contains("definitely-not-an-installed-mcp-server"),
            "{why}"
        );
    }
}
