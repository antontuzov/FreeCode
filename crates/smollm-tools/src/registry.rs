//! The tool registry: one description of every capability the agent may reach for, with the
//! permission class that gates it and the JSON schema the model is handed.
//!
//! What lives here is the contract every executor obeys — a tool cannot exist unless it has an id, a
//! schema, and a declared permission class, because both the approval engine and the sandbox make
//! their decisions from those two facts alone. Only [`EXECUTABLE`] of them have an executor yet, in
//! [`crate::exec`]; the rest wait on the patch and git halves of the loop.
//!
//! One detail from `KNOWN_ISSUES.md` is baked into [`ToolCall::arguments_wire`]: this model emits tool
//! calls whose `arguments` are empty or non-JSON, and the fork's server answers those with HTTP 500
//! (issue #112), so callers must normalize an empty argument object to `"{}"` before replaying a call.

use serde::Serialize;
use serde_json::{json, Value};
use smollm_config::agent::PermissionClass;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDescriptor {
    pub id: &'static str,
    /// Human label for the transcript row ("Read file", "Terminal command ran").
    pub title: &'static str,
    pub description: &'static str,
    pub permission: PermissionClass,
    /// Only reachable when the model may touch the network — which requires an explicitly
    /// configured remote provider, never a silent default.
    pub requires_network: bool,
    pub input_schema: Value,
}

impl ToolDescriptor {
    pub fn is_read_only(&self) -> bool {
        self.permission == PermissionClass::ReadOnly
    }
}

/// A request to run a tool, as parsed out of the model's stream.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub id: String,
    pub tool_id: String,
    pub arguments: Value,
}

impl ToolCall {
    /// The wire form this engine requires: `arguments` must always be a JSON object string, and
    /// `""` or `null` makes the fork answer HTTP 500 (llama.cpp#112).
    pub fn arguments_wire(&self) -> String {
        if self.arguments.is_null() {
            "{}".to_string()
        } else if self.arguments.is_object() {
            self.arguments.to_string()
        } else {
            // A scalar sneaking through is wrapped rather than sent, so the call cannot 500.
            json!({ "value": self.arguments }).to_string()
        }
    }
}

/// The registry, in the order the model sees it.
pub fn registry() -> Vec<ToolDescriptor> {
    use PermissionClass::{Network, ReadOnly, SideEffect, WorkspaceWrite};

    let s = |props: Value, required: &[&str]| {
        json!({
            "type": "object",
            "properties": props,
            "required": required,
            "additionalProperties": false,
        })
    };

    vec![
        ToolDescriptor {
            id: "read_file",
            title: "Read file",
            description: "Read a text file inside the workspace, optionally a line range.",
            permission: ReadOnly,
            requires_network: false,
            input_schema: s(
                json!({
                    "path": { "type": "string", "description": "Workspace-relative path" },
                    "start_line": { "type": "integer", "minimum": 1 },
                    "end_line": { "type": "integer", "minimum": 1 }
                }),
                &["path"],
            ),
        },
        ToolDescriptor {
            id: "list_dir",
            title: "List directory",
            description: "List the entries of a directory inside the workspace.",
            permission: ReadOnly,
            requires_network: false,
            input_schema: s(
                json!({
                    "path": { "type": "string" },
                    "include_hidden": { "type": "boolean" }
                }),
                &["path"],
            ),
        },
        ToolDescriptor {
            id: "glob",
            title: "Find files",
            description: "Match file paths against a glob pattern inside the workspace.",
            permission: ReadOnly,
            requires_network: false,
            input_schema: s(
                json!({ "pattern": { "type": "string" }, "path": { "type": "string" } }),
                &["pattern"],
            ),
        },
        ToolDescriptor {
            id: "grep_search",
            title: "Search content",
            description: "Search file contents by regular expression, with line numbers.",
            permission: ReadOnly,
            requires_network: false,
            input_schema: s(
                json!({
                    "pattern": { "type": "string" },
                    "path": { "type": "string" },
                    "glob": { "type": "string" },
                    "ignore_case": { "type": "boolean" },
                    "context_lines": { "type": "integer", "minimum": 0, "maximum": 20 }
                }),
                &["pattern"],
            ),
        },
        ToolDescriptor {
            id: "codebase_search",
            title: "Search codebase",
            description: "Semantic search over the current workspace.",
            permission: ReadOnly,
            requires_network: false,
            input_schema: s(
                json!({ "query": { "type": "string" }, "max_results": { "type": "integer" } }),
                &["query"],
            ),
        },
        ToolDescriptor {
            id: "write_file",
            title: "Create file",
            description: "Create a file inside the workspace with the given contents, making the \
                directories its path names. It only creates: a path that already holds a file is \
                refused, so an existing file is changed with edit_file and never overwritten whole.",
            permission: WorkspaceWrite,
            requires_network: false,
            input_schema: s(
                json!({
                    "path": { "type": "string" },
                    "contents": { "type": "string" }
                }),
                &["path", "contents"],
            ),
        },
        ToolDescriptor {
            id: "edit_file",
            title: "Edit file",
            description:
                "Replace an exact string in a workspace file; fails if the match is ambiguous.",
            permission: WorkspaceWrite,
            requires_network: false,
            input_schema: s(
                json!({
                    "path": { "type": "string" },
                    "old_string": { "type": "string" },
                    "new_string": { "type": "string" },
                    "replace_all": { "type": "boolean" }
                }),
                &["path", "old_string", "new_string"],
            ),
        },
        ToolDescriptor {
            id: "apply_patch",
            title: "Apply patch",
            description: "Apply one file's unified diff to a file that already exists in the \
                workspace. Every hunk's original lines must match the file exactly, and every hunk \
                is applied or nothing is written. One file per call: send a separate patch for each \
                file. Creating, deleting and renaming a file are not this call.",
            permission: WorkspaceWrite,
            requires_network: false,
            input_schema: s(json!({ "patch": { "type": "string" } }), &["patch"]),
        },
        ToolDescriptor {
            id: "delete_path",
            title: "Delete file",
            description: "Remove a file or directory inside the workspace.",
            permission: WorkspaceWrite,
            requires_network: false,
            input_schema: s(
                json!({ "path": { "type": "string" }, "recursive": { "type": "boolean" } }),
                &["path"],
            ),
        },
        ToolDescriptor {
            id: "move_path",
            title: "Move file",
            description: "Rename or move a workspace path within the sandbox.",
            permission: WorkspaceWrite,
            requires_network: false,
            input_schema: s(
                json!({ "from": { "type": "string" }, "to": { "type": "string" } }),
                &["from", "to"],
            ),
        },
        ToolDescriptor {
            id: "terminal",
            title: "Terminal command ran",
            description: "Run one shell command with the workspace as its starting directory. Only \
                offered while Terminal access is switched on in Settings. The command runs with the \
                user's own permissions, so it is not confined to the workspace; it is stopped at the \
                timeout, and only the tail of its output comes back. Use it to build, run or test what \
                this workspace contains.",
            permission: SideEffect,
            requires_network: false,
            input_schema: s(
                json!({
                    "command": { "type": "string" },
                    "cwd": { "type": "string", "description": "Workspace-relative directory" },
                    "timeout_seconds": { "type": "integer", "minimum": 1, "maximum": 600 }
                }),
                &["command"],
            ),
        },
        ToolDescriptor {
            id: "run_tests",
            title: "Run tests",
            description: "Run the workspace's test command and report failures.",
            permission: SideEffect,
            requires_network: false,
            input_schema: s(
                json!({ "filter": { "type": "string" }, "target": { "type": "string" } }),
                &[],
            ),
        },
        ToolDescriptor {
            id: "run_build",
            title: "Build project",
            description: "Run the workspace's build and surface compiler errors.",
            permission: SideEffect,
            requires_network: false,
            input_schema: s(
                json!({ "target": { "type": "string" }, "release": { "type": "boolean" } }),
                &[],
            ),
        },
        ToolDescriptor {
            id: "run_linter",
            title: "Run linter",
            description: "Run lint and type checks, returning machine-readable diagnostics.",
            permission: SideEffect,
            requires_network: false,
            input_schema: s(
                json!({ "paths": { "type": "array", "items": { "type": "string" } } }),
                &[],
            ),
        },
        ToolDescriptor {
            id: "format_code",
            title: "Format code",
            description: "Apply the workspace formatter to the given paths.",
            permission: WorkspaceWrite,
            requires_network: false,
            input_schema: s(
                json!({ "paths": { "type": "array", "items": { "type": "string" } }, "check": { "type": "boolean" } }),
                &["paths"],
            ),
        },
        ToolDescriptor {
            id: "git_status",
            title: "Git status",
            description: "Show working-tree status for the workspace repository.",
            permission: ReadOnly,
            requires_network: false,
            input_schema: s(json!({}), &[]),
        },
        ToolDescriptor {
            id: "git_diff",
            title: "Git diff",
            description: "Show the staged or unstaged diff, with per-file +N −M counts.",
            permission: ReadOnly,
            requires_network: false,
            input_schema: s(
                json!({ "staged": { "type": "boolean" }, "path": { "type": "string" } }),
                &[],
            ),
        },
        ToolDescriptor {
            id: "git_log",
            title: "Git log",
            description: "List recent commits, optionally scoped to a path.",
            permission: ReadOnly,
            requires_network: false,
            input_schema: s(
                json!({ "limit": { "type": "integer", "maximum": 200 }, "path": { "type": "string" } }),
                &[],
            ),
        },
        ToolDescriptor {
            id: "git_stage",
            title: "Stage changes",
            description: "Stage specific paths for the next commit.",
            permission: SideEffect,
            requires_network: false,
            input_schema: s(
                json!({ "paths": { "type": "array", "items": { "type": "string" } } }),
                &["paths"],
            ),
        },
        ToolDescriptor {
            id: "git_commit",
            title: "Commit changes",
            description: "Create a commit. Always requires approval outside Auto approve.",
            permission: SideEffect,
            requires_network: false,
            input_schema: s(
                json!({ "message": { "type": "string" }, "amend": { "type": "boolean" } }),
                &["message"],
            ),
        },
        ToolDescriptor {
            id: "git_push",
            title: "Push branch",
            description:
                "Push the current branch to its remote. Network plus an explicit approval.",
            permission: Network,
            requires_network: true,
            input_schema: s(
                json!({ "remote": { "type": "string" }, "branch": { "type": "string" } }),
                &[],
            ),
        },
        ToolDescriptor {
            id: "fetch_url",
            title: "Read web page",
            description: "Read one web page by its full http(s) address and return its text, with the \
                markup taken out. Public pages only: an address on this machine, a file path, or a \
                non-text file is refused. The result is cut to max_chars, which says so when it is.",
            permission: Network,
            requires_network: true,
            input_schema: s(
                json!({ "url": { "type": "string", "format": "uri" }, "max_chars": { "type": "integer" } }),
                &["url"],
            ),
        },
        ToolDescriptor {
            id: "web_search",
            title: "Search the web",
            description: "Search Wikipedia and return each hit's title, one line about it, and the \
                address of its page. It answers what a thing is or who someone was; it does not answer \
                what happened today, a price, or anything in a niche field. Use fewer, plainer words \
                rather than a whole question, and read a hit in full with fetch_url.",
            permission: Network,
            requires_network: true,
            input_schema: s(
                json!({ "query": { "type": "string" }, "limit": { "type": "integer", "minimum": 1, "maximum": 8 } }),
                &["query"],
            ),
        },
        ToolDescriptor {
            id: "http_request",
            title: "HTTP request",
            description: "Issue an HTTP request to an allowed host with explicit method and body.",
            permission: Network,
            requires_network: true,
            input_schema: s(
                json!({
                    "method": { "type": "string", "enum": ["GET", "POST", "PUT", "PATCH", "DELETE"] },
                    "url": { "type": "string", "format": "uri" },
                    "headers": { "type": "object", "additionalProperties": { "type": "string" } },
                    "body": { "type": "string" }
                }),
                &["method", "url"],
            ),
        },
        ToolDescriptor {
            id: "install_extension",
            title: "Install extension",
            description: "Install a Skill, MCP server or plugin from the extension marketplace.",
            permission: Network,
            requires_network: true,
            input_schema: s(
                json!({ "id": { "type": "string" }, "version": { "type": "string" } }),
                &["id"],
            ),
        },
        ToolDescriptor {
            id: "mcp_call",
            title: "Call tool from extension",
            description: "Invoke a tool exposed by a connected MCP server.",
            permission: Network,
            requires_network: false,
            input_schema: s(
                json!({
                    "server": { "type": "string" },
                    "tool": { "type": "string" },
                    "arguments": { "type": "object" }
                }),
                &["server", "tool"],
            ),
        },
        ToolDescriptor {
            id: "spawn_subagent",
            title: "Run subagent",
            description: "Delegate a self-contained subtask to a subagent and collect its result.",
            permission: SideEffect,
            requires_network: false,
            input_schema: s(
                json!({
                    "task": { "type": "string" },
                    "agent_type": { "type": "string" },
                    "read_only": { "type": "boolean" }
                }),
                &["task"],
            ),
        },
        ToolDescriptor {
            id: "create_artifact",
            title: "Create artifact",
            description: "Publish a document, page or code artifact into the inspector panel.",
            permission: WorkspaceWrite,
            requires_network: false,
            input_schema: s(
                json!({
                    "kind": { "type": "string", "enum": ["document", "code", "webpage", "diagram"] },
                    "title": { "type": "string" },
                    "content": { "type": "string" }
                }),
                &["kind", "title", "content"],
            ),
        },
        ToolDescriptor {
            id: "ask_user",
            title: "Ask for input",
            description: "Pause the run and ask the user a question with selectable options.",
            permission: ReadOnly,
            requires_network: false,
            input_schema: s(
                json!({
                    "question": { "type": "string" },
                    "options": { "type": "array", "items": { "type": "string" } }
                }),
                &["question"],
            ),
        },
        ToolDescriptor {
            id: "wait_for_port",
            title: "Wait for port",
            description: "Block until a local port accepts connections, for dev-server workflows.",
            permission: SideEffect,
            requires_network: false,
            input_schema: s(
                json!({ "port": { "type": "integer", "minimum": 1, "maximum": 65535 }, "timeout_seconds": { "type": "integer" } }),
                &["port"],
            ),
        },
    ]
}

/// The `tools` array for the chat request, in the OpenAI function form llama.cpp's server parses.
///
/// `arguments` normalization happens in [`ToolCall::arguments_wire`]; this function only shapes the
/// declaration side.
pub fn tool_specs(allow_network: bool) -> Vec<Value> {
    registry()
        .into_iter()
        .filter(|t| allow_network || !t.requires_network)
        .map(spec)
        .collect()
}

/// The tools this build can actually run: the four workspace reads, the three writes, the command,
/// and the two calls that reach the internet, whose executors live in [`crate::exec`] and
/// [`crate::web`].
///
/// The rest of the registry is a declaration with no verb behind it — a schema, a permission class,
/// and nothing that touches the disk. Offering one of those would invite the model to ask for a
/// delete that can only come back refused, so a run that carries tools hands over this list and
/// nothing else. The writes are on it because the sandbox bounds them to the workspace root — one
/// makes a file that is not there, one changes text it has to find first, and one applies a diff
/// whose every old line it has to find first. The shell is on it because the user's Terminal access
/// switch bounds it: the switch is off by default, and [`executable_specs`] does not hand it over
/// until the switch says so.
pub const EXECUTABLE: &[&str] = &[
    "read_file",
    "list_dir",
    "glob",
    "grep_search",
    "write_file",
    "edit_file",
    "apply_patch",
    "terminal",
    "web_search",
    "fetch_url",
];

pub fn is_executable(tool_id: &str) -> bool {
    EXECUTABLE.contains(&tool_id)
}

/// The declaration side for [`EXECUTABLE`], minus the two web calls while `network` is off and the
/// command while `shell` is off.
///
/// The gate itself is [`crate::authorize`], which refuses either one anyway — but a tool that is
/// offered and then refused is a promise the interface made and broke, and it costs the model a round
/// to learn so. When a switch is off, the offer itself has to say what this run cannot reach.
pub fn executable_specs(network: bool, shell: bool) -> Vec<Value> {
    registry()
        .into_iter()
        .filter(|t| {
            is_executable(t.id) && (network || !t.requires_network) && (shell || t.id != "terminal")
        })
        .map(spec)
        .collect()
}

fn spec(t: ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": t.id,
            "description": t.description,
            "parameters": t.input_schema,
        }
    })
}

pub fn find(tool_id: &str) -> Option<ToolDescriptor> {
    registry().into_iter().find(|t| t.id == tool_id)
}

/// The approval question for a call, derived from its class and arguments.
pub fn summary(tool_id: &str, arguments: &Value) -> String {
    match tool_id {
        "terminal" => format!(
            "Run `{}`",
            arguments
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or("<no command>")
        ),
        "git_commit" => format!(
            "Commit: {}",
            arguments
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("")
                .lines()
                .next()
                .unwrap_or("")
        ),
        "fetch_url" | "http_request" => format!(
            "Read {}",
            arguments.get("url").and_then(Value::as_str).unwrap_or("")
        ),
        "web_search" => format!(
            "Search for {}",
            arguments
                .get("query")
                .and_then(Value::as_str)
                .unwrap_or("<no words>")
        ),
        // A write is named by its path alone: the whole contents of a new file, both sides of an
        // edit, or every line of a patch would be quoted back at the person being asked to approve
        // it.
        "write_file" | "edit_file" => format!(
            "Write {}",
            arguments
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("<no path>")
        ),
        "apply_patch" => match arguments
            .get("patch")
            .and_then(Value::as_str)
            .and_then(patch_target)
        {
            Some(path) => format!("Write {path}"),
            None => "Apply a patch".to_string(),
        },
        other => format!("{other} {arguments}"),
    }
}

/// The file a unified diff names as its target, for the row and the approval sentence that have to
/// name *what* is about to be written without quoting every line of the patch. `None` when the text
/// names no target or names `/dev/null` — a create or a delete, which this call does not perform.
///
/// The executor parses and validates the patch in full; this only reads the one header, so a patch
/// this returns a name for can still be refused for every other reason.
pub fn patch_target(patch: &str) -> Option<String> {
    let rest = patch.lines().find_map(|line| line.strip_prefix("+++ "))?;
    let path = rest.split('\t').next().unwrap_or(rest).trim();
    let path = path
        .strip_prefix('"')
        .and_then(|quoted| quoted.strip_suffix('"'))
        .unwrap_or(path);
    let path = path
        .strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path);
    (!path.is_empty() && path != "/dev/null").then(|| path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registry_has_at_least_twenty_distinct_permitted_tools() {
        let r = registry();
        assert!(r.len() >= 20, "only {} tools", r.len());
        let mut ids: Vec<&str> = r.iter().map(|t| t.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), r.len(), "tool ids must be unique");
        assert!(r.iter().all(|t| !t.title.is_empty()));
        assert!(
            r.iter().all(|t| t.input_schema["type"] == json!("object")),
            "every tool needs an object parameter schema"
        );
    }

    #[test]
    fn every_class_is_represented_so_the_approval_engine_has_something_to_discriminate() {
        let r = registry();
        for class in [
            PermissionClass::ReadOnly,
            PermissionClass::WorkspaceWrite,
            PermissionClass::SideEffect,
            PermissionClass::Network,
        ] {
            assert!(
                r.iter().any(|t| t.permission == class),
                "no tool in class {class:?}"
            );
        }
    }

    #[test]
    fn writing_and_running_always_require_more_than_reading() {
        let read = find("read_file").unwrap();
        let write = find("write_file").unwrap();
        let sh = find("terminal").unwrap();
        assert!(read.is_read_only());
        assert_eq!(write.permission, PermissionClass::WorkspaceWrite);
        assert_eq!(sh.permission, PermissionClass::SideEffect);
    }

    #[test]
    fn network_tools_are_hidden_until_a_remote_provider_is_configured() {
        let offline = tool_specs(false);
        let online = tool_specs(true);
        assert!(offline.len() < online.len());
        let names: Vec<&str> = offline
            .iter()
            .map(|v| v["function"]["name"].as_str().unwrap())
            .collect();
        assert!(
            !names.contains(&"web_search"),
            "web_search leaked offline: {names:?}"
        );
        assert!(names.contains(&"read_file"));
    }

    #[test]
    fn empty_tool_arguments_are_sent_as_an_object_never_a_bare_string() {
        let null_args = ToolCall {
            id: "c1".into(),
            tool_id: "git_status".into(),
            arguments: Value::Null,
        };
        assert_eq!(null_args.arguments_wire(), "{}");
        let obj = ToolCall {
            id: "c2".into(),
            tool_id: "read_file".into(),
            arguments: json!({ "path": "src/main.rs" }),
        };
        assert!(obj.arguments_wire().starts_with('{'));
        // llama.cpp#112: a non-object argument payload 500s the server.
        let scalar = ToolCall {
            id: "c3".into(),
            tool_id: "run_tests".into(),
            arguments: json!("all"),
        };
        assert!(serde_json::from_str::<Value>(&scalar.arguments_wire()).is_ok());
        assert!(scalar.arguments_wire().starts_with('{'));
    }

    #[test]
    fn approval_summaries_quote_the_actual_command() {
        let s = summary("terminal", &json!({ "command": "cargo test" }));
        assert_eq!(s, "Run `cargo test`");
        assert!(
            summary("git_commit", &json!({ "message": "Add guard\n\nbody"}))
                .starts_with("Commit: Add guard")
        );
        // And a write is named by where it lands, never by the text it carries.
        assert_eq!(
            summary(
                "write_file",
                &json!({ "path": "my-app/index.html", "contents": "x".repeat(9_000) })
            ),
            "Write my-app/index.html"
        );
        // A patch is many lines of text, and the person approving it is owed the file it lands on
        // rather than the diff quoted back at them.
        assert_eq!(
            summary(
                "apply_patch",
                &json!({
                    "patch": "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-old\n+new\n"
                })
            ),
            "Write src/lib.rs"
        );
        assert_eq!(
            summary("apply_patch", &json!({ "patch": "not a diff" })),
            "Apply a patch"
        );
        // A header naming a create or a delete names no file this call can write.
        assert_eq!(
            patch_target("--- /dev/null\n+++ b/new.rs\n"),
            Some("new.rs".to_string())
        );
        assert_eq!(patch_target("--- a/gone.rs\n+++ /dev/null\n"), None);
    }

    /// A tool with no executor must not be offered: the model would ask for a command and every round
    /// of that is a refusal the run has to explain.
    ///
    /// The other half of the rule is what the loop promises a run: everything offered either is a
    /// workspace read the sandbox bounds, or is one of the three writes it bounds, or is a call that
    /// leaves the machine and therefore only stands on the list while the Privacy switch is on, or is
    /// the command — which stands on the list only while Terminal access is on, and is the one offer
    /// the path sandbox does not bound. A delete or a rename appearing here would mean nothing at all
    /// stands between a run and the machine.
    #[test]
    fn only_bounded_reads_the_writes_the_switchable_web_calls_and_the_shell_are_offered() {
        for id in EXECUTABLE {
            let tool = find(id).unwrap_or_else(|| panic!("{id} is offered but not declared"));
            let allowed = match tool.permission {
                PermissionClass::ReadOnly | PermissionClass::WorkspaceWrite => true,
                PermissionClass::Network => matches!(*id, "web_search" | "fetch_url"),
                PermissionClass::SideEffect => *id == "terminal",
            };
            assert!(allowed, "{id} is offered with {:?}", tool.permission);
        }
        let writes: Vec<&str> = EXECUTABLE
            .iter()
            .filter(|id| find(id).is_some_and(|t| t.permission == PermissionClass::WorkspaceWrite))
            .copied()
            .collect();
        assert_eq!(
            writes,
            vec!["write_file", "edit_file", "apply_patch"],
            "the offered writes are a create that cannot overwrite, an edit that has to match, and \
             a patch whose every old line has to match"
        );
        assert!(
            !is_executable("delete_path"),
            "a run cannot take a file away"
        );
        assert!(
            is_executable("terminal"),
            "the shell has an executor; the switch is what keeps it out of a run"
        );
        // `codebase_search` is a declared read with no index behind it, so it stays out too.
        assert!(!is_executable("codebase_search"));
        assert_eq!(executable_specs(true, true).len(), EXECUTABLE.len());
        assert!(executable_specs(true, true)
            .iter()
            .all(|s| s["type"] == json!("function")));
        // With the Privacy switch off, the two web calls are not offered at all — refused-and-explained
        // would spend a round of the run to arrive at the same answer.
        let offline = executable_specs(false, true);
        let names: Vec<&str> = offline
            .iter()
            .map(|s| s["function"]["name"].as_str().unwrap())
            .collect();
        assert_eq!(names.len(), EXECUTABLE.len() - 2, "{names:?}");
        assert!(!names.contains(&"web_search"), "{names:?}");
        assert!(!names.contains(&"fetch_url"), "{names:?}");
        // And with Terminal access off, the command is not offered either, whatever the other switch
        // says. This is the offer the default settings produce.
        for network in [true, false] {
            let specs = executable_specs(network, false);
            let names: Vec<&str> = specs
                .iter()
                .map(|s| s["function"]["name"].as_str().unwrap())
                .collect();
            assert!(!names.contains(&"terminal"), "{names:?}");
            assert_eq!(names.len(), EXECUTABLE.len() - if network { 1 } else { 3 });
        }
    }
}
