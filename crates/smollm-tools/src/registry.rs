//! The tool registry: one description of every capability the agent may reach for, with the
//! permission class that gates it and the JSON schema the model is handed.
//!
//! What lives here is the contract every executor obeys — a tool cannot exist unless it has an id, a
//! schema, and a declared permission class, because both the approval engine and the sandbox make
//! their decisions from those two facts alone. Only [`EXECUTABLE`] of them have an executor yet, in
//! [`crate::exec`]; the rest wait on the write and shell halves of the loop.
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
            title: "Write file",
            description: "Create or replace a file inside the workspace with the given contents.",
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
            description: "Apply a unified diff across several workspace files.",
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
            description: "Run a shell command in the workspace directory.",
            permission: SideEffect,
            requires_network: false,
            input_schema: s(
                json!({
                    "command": { "type": "string" },
                    "cwd": { "type": "string" },
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
            description: "Fetch a URL and return readable text for the model.",
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
            description: "Run a web search through the configured provider.",
            permission: Network,
            requires_network: true,
            input_schema: s(
                json!({ "query": { "type": "string" }, "recency": { "type": "string" } }),
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

/// The tools this build can actually run: the four workspace reads and the one write whose executors
/// live in [`crate::exec`].
///
/// The rest of the registry is a declaration with no verb behind it — a schema, a permission class,
/// and nothing that touches the disk. Offering one of those would invite the model to ask for a shell
/// command or a delete that can only come back refused, so a run that carries tools hands over this
/// list and nothing else. The write is on it because the sandbox bounds it to the workspace root;
/// the shell is not, because nothing bounds it.
pub const EXECUTABLE: &[&str] = &["read_file", "list_dir", "glob", "grep_search", "edit_file"];

pub fn is_executable(tool_id: &str) -> bool {
    EXECUTABLE.contains(&tool_id)
}

/// The declaration side for [`EXECUTABLE`] only.
pub fn executable_specs() -> Vec<Value> {
    registry()
        .into_iter()
        .filter(|t| is_executable(t.id))
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
            "{tool_id} {}",
            arguments.get("url").and_then(Value::as_str).unwrap_or("")
        ),
        other => format!("{other} {arguments}"),
    }
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
    }

    /// A tool with no executor must not be offered: the model would ask for a command and every round
    /// of that is a refusal the run has to explain.
    ///
    /// The other half of the rule is the one that keeps the loop's promise: what is offered may read
    /// the workspace and change one file of it, and nothing that reaches past the workspace is on the
    /// list at all. A side-effect or network class appearing here means the sandbox is the only thing
    /// between a run and a machine.
    #[test]
    fn only_workspace_reads_and_the_one_write_are_offered_to_a_run() {
        for id in EXECUTABLE {
            let class = find(id).map(|t| t.permission);
            assert!(
                matches!(
                    class,
                    Some(PermissionClass::ReadOnly) | Some(PermissionClass::WorkspaceWrite)
                ),
                "{id} is offered with {class:?}, which the sandbox does not bound"
            );
        }
        assert_eq!(
            EXECUTABLE
                .iter()
                .filter(|id| find(id).is_some_and(|t| !t.is_read_only()))
                .count(),
            1,
            "exactly one offered tool changes a file"
        );
        assert!(!is_executable("write_file"));
        assert!(!is_executable("terminal"));
        // `codebase_search` is a declared read with no index behind it, so it stays out too.
        assert!(!is_executable("codebase_search"));
        let specs = executable_specs();
        assert_eq!(specs.len(), EXECUTABLE.len());
        assert!(specs.iter().all(|s| s["type"] == json!("function")));
    }
}
