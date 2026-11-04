//! MCP: the servers this machine declares, and the calls they publish.
//!
//! The Model Context Protocol is how a tool that is not part of this build reaches a run at all. A
//! skill (#240) adds instructions and grants nothing; a server claims the opposite — it publishes
//! calls, and the whole point of it is that the app did not write them. That is why a call it
//! publishes goes through the same approval gate as one this build wrote, and why it only exists
//! once the user has pressed Check on the server that publishes it.
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
//! Two things happen here. The press on the Extensions page *asks*: it spawns the command, completes
//! the handshake, takes `tools/list`, and shuts the child down — and what the server published then
//! enters the host's registry as calls a run may ask for. The run itself *uses* one: the same server
//! is started again for that one call, asked `tools/call`, and stopped again, because a server this
//! app did not write is not a process it can leave running behind a transcript.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use freecode_config::agent::PermissionClass;
use freecode_tools::ToolFacts;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
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
    /// Whether this build already has a call of that name. A collision is not fatal — a server's
    /// call is offered under a name of its own, so it never answers for one of this app's — but it
    /// is the thing a user has to see before a later build lets a server answer for a call the app
    /// means to own.
    pub collides: bool,
    /// The name a run calls it by, once the server has been connected. Empty for a call that was
    /// never registered — one the server published without a name, which cannot be routed.
    pub offered: String,
    /// The schema the server published, in the form the model is handed. It stays off the wire: the
    /// page has no use for it, and a server's schema is not this app's to publish.
    #[serde(skip)]
    pub input_schema: Value,
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
            input_schema: schema(value),
            name,
            offered: String::new(),
        }
    }
}

/// The schema a server published, in a shape the model can be handed.
///
/// A server is free to publish a schema that is not an object of properties, and the model is not:
/// a function whose parameters are an array is a call no model can fill in. So anything that is not
/// an object with properties becomes the empty object schema, which asks for nothing and refuses
/// nothing.
fn schema(value: &Value) -> Value {
    let declared = value.get("inputSchema");
    if declared.map(|schema| schema.is_object()).unwrap_or(false) {
        return declared.cloned().unwrap_or_else(|| json!({}));
    }
    json!({ "type": "object", "properties": {} })
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// One call a declared server publishes, as the host's own registry sees it.
///
/// The model calls it by [`Self::id`] — the server's name and the call's name folded into one
/// function name — because two servers may publish a call of the same name and the model has to be
/// able to tell them apart. The server is still asked by the name it published the call under.
///
/// It answers the approval gate as a side effect that reaches the network, because that is what it
/// is: a command this app did not write, running on this user's machine, on the far side of a
/// switch the user has to turn on before any of it is offered.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Published {
    /// The name the model calls it by.
    pub id: String,
    /// The server that published it.
    pub server: String,
    /// The name the server published it under — what `tools/call` asks for.
    pub tool: String,
    /// The label a refusal card names it by.
    pub title: String,
    /// What the call does, in the words of whoever declared it.
    pub description: String,
    /// The schema the model is handed. It stays off the wire: the page has no use for a server's
    /// schema, and a schema is not this app's to publish.
    #[serde(skip)]
    pub input_schema: Value,
}

impl ToolFacts for Published {
    fn tool_id(&self) -> &str {
        &self.id
    }

    fn tool_title(&self) -> &str {
        &self.title
    }

    fn tool_description(&self) -> &str {
        &self.description
    }

    fn tool_class(&self) -> PermissionClass {
        PermissionClass::SideEffect
    }

    fn tool_needs_network(&self) -> bool {
        true
    }

    fn tool_schema(&self) -> &Value {
        &self.input_schema
    }

    fn tool_headline(&self, _arguments: &Value) -> String {
        format!("Call `{}` on the {} server", self.tool, self.server)
    }

    /// The arguments the server would be given, as they were written. It is the whole of what the
    /// call would do that this app can show, and a call with no arguments has nothing to show.
    fn tool_payload(&self, arguments: &Value) -> Option<String> {
        match arguments.as_object() {
            Some(map) if !map.is_empty() => Some(arguments.to_string()),
            _ => None,
        }
    }
}

/// The calls a connected server has published, and the ones a run may ask for.
///
/// The registry is the app's own state rather than the declaration file's: a call enters it when
/// the user presses Check on a server and the handshake answered, which is the only moment this
/// app has agreed to run that command at all. A server that is declared but never connected
/// publishes nothing a run can reach, and a server that has answered once keeps its calls until it
/// is asked again — a declaration edited behind the app's back does not silently change what a run
/// may ask for.
#[derive(Default)]
pub struct Registry {
    published: Mutex<Vec<Published>>,
}

impl Registry {
    /// Remember what one server publishes, replacing what it published before.
    ///
    /// The ids are handed out here rather than taken from the server, because a function name has to
    /// be one word of a small alphabet and a server's names are not that. The ids come back in the
    /// order of the tools, so the page can show a run's name for a call beside the server's own.
    pub fn register(&self, server: &str, tools: &[Tool]) -> Vec<String> {
        let Ok(mut held) = self.published.lock() else {
            return vec![String::new(); tools.len()];
        };
        held.retain(|published| published.server != server);
        let mut taken: Vec<String> = held.iter().map(|published| published.id.clone()).collect();
        let mut ids = Vec::with_capacity(tools.len());
        for tool in tools {
            if tool.name.is_empty() {
                ids.push(String::new());
                continue;
            }
            let id = offered_id(server, &tool.name, &taken);
            taken.push(id.clone());
            ids.push(id.clone());
            held.push(Published {
                id,
                server: server.to_string(),
                tool: tool.name.clone(),
                title: if tool.title.is_empty() {
                    tool.name.clone()
                } else {
                    tool.title.clone()
                },
                description: if tool.description.is_empty() {
                    format!("A call the {} server publishes.", server)
                } else {
                    tool.description.clone()
                },
                input_schema: tool.input_schema.clone(),
            });
        }
        ids
    }

    /// One published call, by the name the model calls it by.
    pub fn find(&self, id: &str) -> Option<Published> {
        self.published
            .lock()
            .ok()
            .and_then(|held| held.iter().find(|published| published.id == id).cloned())
    }

    /// Every published call, in the order the servers were connected.
    pub fn all(&self) -> Vec<Published> {
        self.published
            .lock()
            .map(|held| held.clone())
            .unwrap_or_default()
    }

    /// The published calls as the declarations the model is handed.
    pub fn specs(&self) -> Vec<Value> {
        self.all()
            .iter()
            .map(|published| {
                json!({
                    "type": "function",
                    "function": {
                        "name": published.id,
                        "description": published.description,
                        "parameters": published.input_schema,
                    }
                })
            })
            .collect()
    }
}

/// The name the model calls a published call by.
///
/// A function name has to be one word of letters, digits, `_` and `-`, and this app's own calls are
/// plain words — so a server's call is prefixed and both halves are folded into that alphabet. The
/// two lengths keep the whole inside the sixty-four characters the format allows, and a name that
/// two servers fold to the same thing gets a number rather than being one of them.
pub fn offered_id(server: &str, tool: &str, taken: &[String]) -> String {
    let base = format!("mcp__{}__{}", fold(server, 20), fold(tool, 32));
    if !taken.iter().any(|id| id == &base) {
        return base;
    }
    for n in 2..100 {
        let candidate = format!("{base}_{n}");
        if !taken.iter().any(|id| id == &candidate) {
            return candidate;
        }
    }
    base
}

/// One name folded into the alphabet a function name may use, and cut to `max` characters.
fn fold(name: &str, max: usize) -> String {
    let folded: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
                character
            } else {
                '_'
            }
        })
        .collect();
    let cut: String = folded.chars().take(max).collect();
    if cut.is_empty() {
        "x".to_string()
    } else {
        cut
    }
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
/// What it published enters the registry as calls a run may ask for: the press on the Extensions
/// page is the moment this app agreed to run that command at all, so it is the moment its calls
/// become reachable.
///
/// The child is killed on every path, and `kill_on_drop` is the backstop for a future early return
/// that forgets to: an MCP server that outlives the check that started it is a process with no owner.
pub async fn check(name: &str, registry: &Registry) -> Result<Report, String> {
    let server = inventory()
        .servers
        .into_iter()
        .find(|declared| declared.name == name)
        .ok_or_else(|| format!("no server named {name} is declared"))?;
    if let Some(reason) = refusal(&server) {
        return Err(reason);
    }
    let mut report = ask_server(&server).await?;
    let ids = registry.register(&server.name, &report.tools);
    for (tool, id) in report.tools.iter_mut().zip(ids) {
        tool.offered = id;
    }
    Ok(report)
}

/// One published call, by the name the model calls it by.
pub fn published(app: &AppHandle, id: &str) -> Option<Published> {
    app.state::<Registry>().find(id)
}

/// Every published call, as the declarations the model is handed.
pub fn offered(app: &AppHandle) -> Vec<Value> {
    app.state::<Registry>().specs()
}

/// The calls a run may ask for, grouped by the server that published them.
///
/// What the page shows after a reload: a server that answered once keeps its calls until it is asked
/// again, so the page has to be able to say which ones are reachable rather than only what the last
/// press said.
pub fn connected(app: &AppHandle) -> Vec<Published> {
    app.state::<Registry>().all()
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

    /// Ask the server to run one of the calls it published, and say what it answered.
    pub async fn call(&mut self, tool: &str, arguments: &Value) -> Result<String, String> {
        let result = self
            .request(
                "tools/call",
                json!({ "name": tool, "arguments": arguments }),
            )
            .await?;
        said(&result)
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

/// The text a `tools/call` result carries.
///
/// The protocol's own shape for a result is a list of typed blocks rather than a string, and the
/// text blocks are what a model can read. A result that says `isError` is the server saying the
/// call failed, and that is an error here rather than a sentence the model would read as an answer.
fn said(result: &Value) -> Result<String, String> {
    if result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Err(format!("the call failed: {}", answer(result)));
    }
    Ok(answer(result))
}

/// The readable part of a result: its text blocks, or its structured content when it published no
/// text at all.
fn answer(result: &Value) -> String {
    let mut out = String::new();
    if let Some(blocks) = result.get("content").and_then(Value::as_array) {
        for block in blocks {
            if block.get("type").and_then(Value::as_str) != Some("text") {
                continue;
            }
            let body = block
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(body);
        }
    }
    if out.is_empty() {
        if let Some(structured) = result.get("structuredContent") {
            if !structured.is_null() {
                return structured.to_string();
            }
        }
    }
    out
}

/// Run one published call: start the server, ask it, and stop it again.
///
/// The server is read fresh from the declaration file rather than from whatever was declared when
/// it was connected, so a command that has been edited or removed since is the one that runs — or
/// the one that is refused. The child is killed on every path out, including the ones that failed:
/// a server this app did not write is not a process it can leave behind a transcript.
pub async fn call_tool(server_name: &str, tool: &str, arguments: &Value) -> Result<String, String> {
    let server = inventory()
        .servers
        .into_iter()
        .find(|declared| declared.name == server_name)
        .ok_or_else(|| format!("no server named {server_name} is declared"))?;
    if let Some(reason) = refusal(&server) {
        return Err(reason);
    }
    let (pipes, mut child) = spawn(&server)?;
    let mut client = Client::new(pipes);
    let outcome = async {
        client.initialize(env!("CARGO_PKG_VERSION")).await?;
        client.call(tool, arguments).await
    }
    .await;
    let _ = child.kill().await;
    let _ = child.wait().await;
    outcome.map_err(|why| format!("{}: {why}", server.name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use freecode_config::agent::{ApprovalMode, Approvals};
    use freecode_core::approvals::{evaluate, ApprovalDecision, Pending};
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
        let why = check("no-such-server-anywhere", &Registry::default())
            .await
            .unwrap_err();
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

    /// The name the model calls a published call by: the server and the call, in one word.
    #[test]
    fn a_published_call_is_named_after_its_server_and_its_own_name() {
        assert_eq!(
            offered_id("weather", "get_forecast", &[]),
            "mcp__weather__get_forecast"
        );
    }

    #[test]
    fn a_name_outside_the_alphabet_is_folded_into_it() {
        // A function name is one token of letters, digits, `_` and `-`, and a server's names are not
        // that. Folding is what lets the call be called at all.
        assert_eq!(
            offered_id("my server", "get/forecast.v2", &[]),
            "mcp__my_server__get_forecast_v2"
        );
    }

    #[test]
    fn two_calls_that_fold_to_the_same_name_are_told_apart() {
        let registry = Registry::default();
        let ids = registry.register(
            "weather",
            &[
                Tool::declared(&json!({ "name": "get forecast" })),
                Tool::declared(&json!({ "name": "get_forecast" })),
            ],
        );
        assert_eq!(
            ids,
            ["mcp__weather__get_forecast", "mcp__weather__get_forecast_2"],
            "the second call gets a number rather than being the first one"
        );
    }

    #[test]
    fn a_long_name_stays_inside_the_function_name_limit() {
        let id = offered_id(&"s".repeat(60), &"t".repeat(60), &[]);
        assert_eq!(id, format!("mcp__{}__{}", "s".repeat(20), "t".repeat(32)));
        assert!(id.len() <= 64, "{id} is {} characters", id.len());
    }

    #[test]
    fn a_call_the_server_published_without_a_name_is_not_offered() {
        // It cannot be routed, so it is not registered and the run is never told about it.
        let registry = Registry::default();
        let ids = registry.register(
            "weather",
            &[Tool::declared(&json!({ "description": "no name" }))],
        );
        assert_eq!(ids, [""]);
        assert!(registry.all().is_empty());
    }

    #[test]
    fn a_connected_server_publishes_calls_a_run_may_ask_for() {
        let registry = Registry::default();
        let ids = registry.register(
            "weather",
            &[Tool::declared(&json!({
                "name": "get_forecast",
                "description": "One day.",
                "inputSchema": { "type": "object", "properties": { "city": { "type": "string" } } },
            }))],
        );
        assert_eq!(ids, ["mcp__weather__get_forecast"]);
        let found = registry.find("mcp__weather__get_forecast").unwrap();
        assert_eq!(found.server, "weather");
        assert_eq!(
            found.tool, "get_forecast",
            "the server is asked by its own name"
        );
        assert_eq!(found.description, "One day.");
        assert_eq!(found.input_schema["properties"]["city"]["type"], "string");
        // The model is handed the server's schema under the name the model calls it by.
        let specs = registry.specs();
        assert_eq!(specs[0]["function"]["name"], "mcp__weather__get_forecast");
        assert_eq!(
            specs[0]["function"]["parameters"]["properties"]["city"]["type"],
            "string"
        );
    }

    #[test]
    fn a_second_check_replaces_what_the_server_published_before() {
        // The page's press is the moment the app agreed to run that command, so a later press is the
        // moment its calls change — a server that dropped a call must not leave the old one reachable.
        let registry = Registry::default();
        registry.register(
            "weather",
            &[Tool::declared(&json!({ "name": "get_forecast" }))],
        );
        registry.register("weather", &[Tool::declared(&json!({ "name": "radar" }))]);
        assert!(
            registry.find("mcp__weather__get_forecast").is_none(),
            "the call the server no longer publishes is gone"
        );
        assert!(registry.find("mcp__weather__radar").is_some());
        assert_eq!(registry.all().len(), 1);
    }

    #[test]
    fn a_server_that_never_connected_publishes_nothing() {
        // Declared is not connected: only the press on the Extensions page starts the command.
        let registry = Registry::default();
        assert!(registry.find("mcp__weather__get_forecast").is_none());
        assert!(registry.all().is_empty());
    }

    /// The call the model makes, over the protocol, against the peer at the far end of a pipe pair.
    #[tokio::test]
    async fn a_published_call_is_asked_by_its_own_name_and_its_answer_read_back() {
        let (mut client, mut peer) = pair();
        let script = async {
            let (id, _) = peer.request("initialize").await;
            peer.reply(id, json!({ "protocolVersion": PROTOCOL_VERSION }))
                .await;
            peer.notification("notifications/initialized").await;
            let (id, params) = peer.request("tools/call").await;
            assert_eq!(
                params["name"],
                json!("get_forecast"),
                "the server is asked by the name it published"
            );
            assert_eq!(params["arguments"], json!({ "city": "Berlin" }));
            peer.reply(
                id,
                json!({ "content": [{ "type": "text", "text": "12C and raining" }] }),
            )
            .await;
        };
        let (outcome, ()) = tokio::join!(
            async {
                client.initialize(env!("CARGO_PKG_VERSION")).await?;
                client
                    .call("get_forecast", &json!({ "city": "Berlin" }))
                    .await
            },
            script
        );
        assert_eq!(outcome.unwrap(), "12C and raining");
    }

    #[tokio::test]
    async fn a_call_the_server_failed_is_a_failure_rather_than_an_answer() {
        let (mut client, mut peer) = pair();
        let script = async {
            let (id, _) = peer.request("initialize").await;
            peer.reply(id, json!({ "protocolVersion": PROTOCOL_VERSION }))
                .await;
            peer.notification("notifications/initialized").await;
            let (id, _) = peer.request("tools/call").await;
            peer.reply(
                id,
                json!({
                    "content": [{ "type": "text", "text": "no station for that city" }],
                    "isError": true,
                }),
            )
            .await;
        };
        let (outcome, ()) = tokio::join!(
            async {
                client.initialize(env!("CARGO_PKG_VERSION")).await?;
                client
                    .call("get_forecast", &json!({ "city": "Atlantis" }))
                    .await
            },
            script
        );
        let why = outcome.unwrap_err();
        assert!(why.contains("the call failed"), "{why}");
        assert!(why.contains("no station"), "{why}");
    }

    /// The published call the gate is asked about.
    fn published_call() -> Published {
        Published {
            id: "mcp__weather__get_forecast".to_string(),
            server: "weather".to_string(),
            tool: "get_forecast".to_string(),
            title: "get_forecast".to_string(),
            description: "One day.".to_string(),
            input_schema: json!({ "type": "object", "properties": {} }),
        }
    }

    fn sandbox(network: bool) -> freecode_tools::SandboxPolicy {
        freecode_tools::SandboxPolicy {
            network_allowed: network,
            ..Default::default()
        }
    }

    #[test]
    fn a_published_call_asks_before_it_runs() {
        // A call the app did not write is a side effect that reaches out of the machine, so it goes
        // through the same gate as one this build wrote — and a side effect prompts.
        let call = published_call();
        match evaluate(
            &call,
            &json!({ "city": "Berlin" }),
            &Approvals::default(),
            &sandbox(true),
            "call-1",
        ) {
            Pending::Prompt(request) => {
                assert_eq!(request.tool_id, "mcp__weather__get_forecast");
                assert_eq!(request.permission, PermissionClass::SideEffect);
                assert_eq!(
                    request.headline,
                    "Call `get_forecast` on the weather server"
                );
                assert_eq!(request.payload.as_deref(), Some(r#"{"city":"Berlin"}"#));
            }
            other => panic!("a published call must prompt: {other:?}"),
        }
    }

    #[test]
    fn a_published_call_is_refused_while_the_privacy_switch_is_off() {
        // The switch that keeps the "Local" badge true also speaks for a server's call: it is a
        // command this app did not write, running on this user's machine.
        let call = published_call();
        assert_eq!(
            evaluate(
                &call,
                &json!({}),
                &Approvals::default(),
                &sandbox(false),
                "call-1"
            ),
            Pending::Decided(ApprovalDecision::BlockedByPolicy)
        );
    }

    #[test]
    fn a_plan_only_run_refuses_a_published_call() {
        let call = published_call();
        let approvals = Approvals {
            mode: ApprovalMode::PlanOnly,
            ..Default::default()
        };
        assert_eq!(
            evaluate(&call, &json!({}), &approvals, &sandbox(true), "call-1"),
            Pending::Decided(ApprovalDecision::BlockedByPolicy)
        );
    }

    #[test]
    fn an_allow_list_entry_does_not_admit_a_published_call() {
        // The lists are the user's own words about commands. A server's call is not this user's
        // shell, so an entry that names it says nothing about it.
        let call = published_call();
        let approvals = Approvals {
            shell_allow: vec!["get_forecast".into()],
            shell_enabled: true,
            ..Default::default()
        };
        let policy = freecode_tools::SandboxPolicy {
            network_allowed: true,
            shell_allowed: true,
            ..Default::default()
        };
        assert!(
            matches!(
                evaluate(&call, &json!({}), &approvals, &policy, "call-1"),
                Pending::Prompt(_)
            ),
            "the list is about commands, and this is not one of them"
        );
    }
}
