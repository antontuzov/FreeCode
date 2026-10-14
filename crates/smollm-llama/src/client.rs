//! The OpenAI-compatible client for a supervised fork server: build a request the template accepts,
//! stream the reply, and keep the engine's own failure modes out of the conversation.
//!
//! Four constraints from the model's KNOWN_ISSUES.md are enforced here rather than in the agent loop,
//! because each is a property of the *wire format* this crate produces:
//!   * exactly one system message, first — two of them, or one anywhere else, is an HTTP 500
//!     (`normalize` merges them, which is the documented workaround);
//!   * empty or malformed tool arguments are an HTTP 500/400 (llama.cpp#112), so a replayed
//!     `tool_call` always carries a JSON object — `"{}"` at worst;
//!   * `reasoning_effort: "high"` is an HTTP 500 — the template accepts `low`, `medium`, `xhigh` and
//!     `none`; `low` is accepted but measures about as long as `xhigh`, so it goes out as `xhigh` and
//!     the UI says so;
//!   * reasoning from earlier turns must not be re-rendered into later ones (Bonsai-demo#183), so an
//!     assistant turn replays as text plus its tool calls and never as its `reasoning_content`.
//!
//! The budget is clamped against the server's `-c`: llama.cpp needs `prompt + max_tokens` to fit the
//! loaded context, and a cap that runs out mid-thought produces the empty reply the card calls the
//! single most common report.

use futures::StreamExt;
use serde::Serialize;
use serde_json::{json, Value};

use smollm_models::catalog::{AgentConstraints, ModelEntry, ReasoningValues};

use crate::LlamaError;

/// A chunk of the reply, in the order the server sent it.
#[derive(Debug, Clone, PartialEq)]
pub enum ReplyDelta {
    /// `reasoning_content` — the thinking the UI shows collapsed under "Thinking".
    Thought(String),
    /// `content` — the answer.
    Answer(String),
    /// A tool call the model asked for. `arguments` is incremental text of its JSON object.
    ToolCall {
        index: usize,
        id: Option<String>,
        name: Option<String>,
        arguments: Option<String>,
    },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reply {
    pub text: String,
    pub reasoning: String,
    pub tool_calls: Vec<ToolCall>,
    /// `stop`, `length`, `tool_calls` … — `length` means the budget ran out mid-reply.
    pub finish_reason: Option<String>,
    /// What the window holds after this run: the whole prompt plus the whole answer, as the server
    /// counted them. `None` when the stream never reported a usage frame.
    pub total_tokens: Option<u64>,
    /// How much of that window the prompt alone took, as the server counted it. `None` with no usage
    /// frame. This is the number that cannot be inferred from the request: a server that trimmed the
    /// conversation to fit reports what it kept, not what it was handed.
    pub prompt_tokens: Option<u64>,
}

impl Reply {
    pub fn ran_out_of_budget(&self) -> bool {
        self.finish_reason.as_deref() == Some("length")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub role: Role,
    pub content: String,
    /// For an assistant turn: the calls it made. Replayed so the template sees its own history; the
    /// reasoning that produced them is deliberately not part of the message.
    pub tool_calls: Vec<ToolCall>,
    /// For a `Role::Tool` result: the call it answers.
    pub call_id: Option<String>,
}

impl Message {
    fn new(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            tool_calls: Vec::new(),
            call_id: None,
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self::new(Role::System, content)
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self::new(Role::User, content)
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new(Role::Assistant, content)
    }
    pub fn tool_result(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            call_id: Some(call_id.into()),
            ..Self::new(Role::Tool, content)
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    pub messages: Vec<Message>,
    /// Requested output budget, clamped against the server's context.
    pub max_tokens: u32,
    /// The picker's value, before the template's vocabulary is applied. `None` leaves the server's
    /// `--reasoning auto` behaviour alone.
    pub reasoning_effort: Option<String>,
    /// Tool schemas in OpenAI's `{"type":"function", …}` form.
    pub tools: Vec<Value>,
}

impl ChatRequest {
    pub fn new(messages: Vec<Message>) -> Self {
        Self {
            messages,
            max_tokens: 16_384,
            reasoning_effort: None,
            tools: Vec::new(),
        }
    }
}

/// Why a request was refused before it left the process. Each variant is a status the fork would
/// otherwise answer with.
#[derive(Debug, thiserror::Error)]
pub enum RequestError {
    #[error("a chat request needs at least one message")]
    Empty,
    #[error(
        "this model's chat template does not accept reasoning effort {0}; it takes the values the catalog lists"
    )]
    EffortRejected(String),
}

fn valid_object(raw: &str) -> bool {
    serde_json::from_str::<Value>(raw)
        .map(|v| v.is_object())
        .unwrap_or(false)
}

/// Merge every system message into one leading message — the documented workaround for the fork's
/// "HTTP 500 when a system message isn't first, or when there are two".
pub fn normalize(messages: &[Message]) -> Vec<Message> {
    let mut system: Vec<String> = Vec::new();
    let mut rest: Vec<Message> = Vec::new();
    for m in messages {
        if m.role == Role::System {
            if !m.content.trim().is_empty() {
                system.push(m.content.clone());
            }
        } else {
            rest.push(m.clone());
        }
    }
    let merged = system.join("\n\n");
    let mut out = Vec::with_capacity(rest.len() + 1);
    if !merged.is_empty() {
        out.push(Message::system(merged));
    }
    out.extend(rest);
    out
}

/// Map the picker's effort onto what the template accepts. `low` is accepted but measured about as
/// costly as `xhigh` (KNOWN_ISSUES.md), so it is sent as the catalog's `low_behaves_like` — but only
/// where such a mapping was measured. A generation whose template has no thinking branch declares
/// `low_behaves_like` empty, and inventing a synonym for it would put `reasoning_effort: ""` on the
/// wire.
pub fn wire_effort(
    requested: Option<&str>,
    reasoning: &ReasoningValues,
) -> Result<Option<String>, RequestError> {
    let Some(effort) = requested else {
        return Ok(None);
    };
    let supported = reasoning.supported_efforts.iter().any(|e| e == effort);
    if supported {
        return Ok(Some(effort.to_string()));
    }
    if effort == "low" && !reasoning.low_behaves_like.is_empty() {
        return Ok(Some(reasoning.low_behaves_like.clone()));
    }
    Err(RequestError::EffortRejected(effort.to_string()))
}

/// The budget that goes on the wire: never below the model's minimum — a small cap "ends generation
/// mid-thought" and yields an empty reply — and never above what can share the loaded context with
/// the prompt.
pub fn clamp_budget(max_tokens: u32, context_tokens: u32, constraints: &AgentConstraints) -> u32 {
    let floor = constraints.min_generation_budget_tokens;
    let ceiling = context_tokens / 2;
    if ceiling <= floor {
        // Too small to hold both halves: prefer the floor, because a truncated answer is the failure
        // the model card tells us to avoid.
        return floor;
    }
    max_tokens.clamp(floor, ceiling)
}

/// The wire form of one message.
fn message_json(m: &Message, constraints: &AgentConstraints) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("role".into(), json!(m.role));
    obj.insert("content".into(), json!(m.content));
    if let Some(id) = &m.call_id {
        obj.insert("tool_call_id".into(), json!(id));
    }
    if !m.tool_calls.is_empty() {
        let calls: Vec<Value> = m
            .tool_calls
            .iter()
            .map(|c| {
                // #112: the fork 500s on empty or non-JSON arguments, so an unfinished call is
                // replayed as the empty object it was meant to be.
                let arguments =
                    if !valid_object(&c.arguments) && constraints.empty_tool_arguments_as_object {
                        "{}".to_string()
                    } else {
                        c.arguments.clone()
                    };
                json!({
                    "id": c.id,
                    "type": "function",
                    "function": { "name": c.name, "arguments": arguments }
                })
            })
            .collect();
        obj.insert("tool_calls".into(), Value::Array(calls));
    }
    Value::Object(obj)
}

/// The request body, with every constraint above applied.
pub fn chat_body(
    req: &ChatRequest,
    model: &ModelEntry,
    context_tokens: u32,
) -> Result<Value, RequestError> {
    if req.messages.is_empty() {
        return Err(RequestError::Empty);
    }
    let effort = wire_effort(req.reasoning_effort.as_deref(), &model.sampling.reasoning)?;
    let mut body = json!({
        "messages": normalize(&req.messages)
            .iter()
            .map(|m| message_json(m, &model.agent_constraints))
            .collect::<Vec<_>>(),
        "max_tokens": clamp_budget(req.max_tokens, context_tokens, &model.agent_constraints),
        "stream": true,
        // The fork sends a usage frame only when asked for one, and the frame is where the context
        // meter's number comes from; without it the stream ends and says nothing about its size.
        "stream_options": { "include_usage": true },
    });
    if let Some(effort) = effort {
        body["reasoning_effort"] = json!(effort);
    }
    if !req.tools.is_empty() {
        body["tools"] = Value::Array(req.tools.clone());
    }
    Ok(body)
}

#[derive(Debug, Clone, PartialEq)]
pub enum SseEvent {
    Data(Value),
    Done,
    Malformed { payload: String, reason: String },
}

/// Incremental SSE reader. `push` takes a network chunk and returns every `data:` payload that is now
/// complete; a chunk may hold several events or half of one, so the remainder is carried over.
#[derive(Debug, Default)]
pub struct SseDecoder {
    pending: String,
    finished: bool,
}

const LF_FRAME: &str = "\n\n";
const CRLF_FRAME: &str = "\r\n\r\n";

impl SseDecoder {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        self.pending.push_str(&String::from_utf8_lossy(bytes));
        let mut events = Vec::new();
        while let Some(frame) = self.take_frame() {
            for line in frame.lines() {
                let Some(data) = line.strip_prefix("data:") else {
                    continue;
                };
                let data = data.trim();
                if data == "[DONE]" {
                    self.finished = true;
                    events.push(SseEvent::Done);
                    continue;
                }
                match serde_json::from_str::<Value>(data) {
                    Ok(value) => events.push(SseEvent::Data(value)),
                    Err(e) => events.push(SseEvent::Malformed {
                        payload: data.to_string(),
                        reason: e.to_string(),
                    }),
                }
            }
        }
        events
    }

    /// The earliest complete event, delimited by either LF-LF or CRLF-CRLF. Searching `pending`
    /// rather than the incoming chunk is what makes a boundary that straddles two packets work.
    fn take_frame(&mut self) -> Option<String> {
        let (at, sep) = match (self.pending.find(LF_FRAME), self.pending.find(CRLF_FRAME)) {
            (Some(lf), Some(crlf)) if crlf < lf => (crlf, CRLF_FRAME),
            (Some(lf), _) => (lf, LF_FRAME),
            (None, Some(crlf)) => (crlf, CRLF_FRAME),
            (None, None) => return None,
        };
        let frame = self.pending[..at].to_string();
        self.pending.drain(..at + sep.len());
        Some(frame)
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }
}

/// Turn one `chat.completion.chunk` into deltas, plus its `finish_reason` when it carries one.
pub fn chunk_deltas(chunk: &Value) -> (Vec<ReplyDelta>, Option<String>) {
    let mut out = Vec::new();
    let Some(choice) = chunk
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
    else {
        return (out, None);
    };
    let finish = choice
        .get("finish_reason")
        .and_then(|f| f.as_str())
        .map(str::to_string);
    let delta = &choice["delta"];
    if let Some(thought) = delta.get("reasoning_content").and_then(|t| t.as_str()) {
        if !thought.is_empty() {
            out.push(ReplyDelta::Thought(thought.to_string()));
        }
    }
    if let Some(answer) = delta.get("content").and_then(|t| t.as_str()) {
        if !answer.is_empty() {
            out.push(ReplyDelta::Answer(answer.to_string()));
        }
    }
    if let Some(calls) = delta.get("tool_calls").and_then(|c| c.as_array()) {
        for (position, call) in calls.iter().enumerate() {
            let index = call
                .get("index")
                .and_then(|n| n.as_u64())
                .unwrap_or(position as u64) as usize;
            let function = call.get("function");
            out.push(ReplyDelta::ToolCall {
                index,
                id: call.get("id").and_then(|v| v.as_str()).map(str::to_string),
                name: function
                    .and_then(|f| f.get("name"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                arguments: function
                    .and_then(|f| f.get("arguments"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            });
        }
    }
    (out, finish)
}

/// What the server counted for one finished run. Both numbers are the server's own, and both are
/// counted *after* it decided what to do with an oversized prompt — a prompt it trimmed reports the
/// trimmed size, which is the only way the host learns the model did not see the whole conversation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub total_tokens: u64,
}

/// The usage frame: the fork reports it once, in a chunk whose `choices` is empty, after the answer
/// is done.
fn chunk_usage(chunk: &Value) -> Option<Usage> {
    let usage = chunk.get("usage")?;
    let read = |key: &str| usage.get(key).and_then(|t| t.as_u64());
    Some(Usage {
        prompt_tokens: read("prompt_tokens")?,
        total_tokens: read("total_tokens")?,
    })
}

fn apply_deltas(reply: &mut Reply, deltas: &[ReplyDelta]) {
    for d in deltas {
        match d {
            ReplyDelta::Thought(t) => reply.reasoning.push_str(t),
            ReplyDelta::Answer(t) => reply.text.push_str(t),
            ReplyDelta::ToolCall {
                index,
                id,
                name,
                arguments,
            } => {
                while reply.tool_calls.len() <= *index {
                    reply.tool_calls.push(ToolCall::default());
                }
                let call = &mut reply.tool_calls[*index];
                if let Some(id) = id {
                    call.id.push_str(id);
                }
                if let Some(name) = name {
                    call.name.push_str(name);
                }
                if let Some(arguments) = arguments {
                    call.arguments.push_str(arguments);
                }
            }
        }
    }
}

/// What the caller watching a stream says about the rest of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamControl {
    Continue,
    /// Stop here. The text streamed so far is returned as the answer, because that is what the model
    /// actually said before the caller stopped listening.
    Stop,
}

/// POST one chat completion and stream it, forwarding each batch of deltas as it arrives. The
/// callback answers [`StreamControl::Stop`] to end the run early; the request is then dropped, which
/// closes the connection and lets the server abandon the generation too.
pub async fn stream_chat(
    base_url: &str,
    req: &ChatRequest,
    model: &ModelEntry,
    context_tokens: u32,
    mut on_delta: impl FnMut(&[ReplyDelta]) -> StreamControl,
) -> Result<Reply, LlamaError> {
    let body = chat_body(req, model, context_tokens)?;
    let response = reqwest::Client::new()
        .post(format!("{base_url}/v1/chat/completions"))
        .json(&body)
        .send()
        .await?;
    let status = response.status();
    if !status.is_success() {
        // A template refusal is only diagnosable from the body the server sent with it.
        let text = response.text().await.unwrap_or_default();
        return Err(LlamaError::ServerError {
            status: status.as_u16(),
            body: text,
        });
    }

    let mut reply = Reply::default();
    let mut decoder = SseDecoder::default();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        let mut deltas = Vec::new();
        for event in decoder.push(&chunk) {
            match event {
                SseEvent::Data(value) => {
                    let (mut d, finish) = chunk_deltas(&value);
                    deltas.append(&mut d);
                    if let Some(f) = finish {
                        reply.finish_reason = Some(f);
                    }
                    if let Some(usage) = chunk_usage(&value) {
                        reply.total_tokens = Some(usage.total_tokens);
                        reply.prompt_tokens = Some(usage.prompt_tokens);
                    }
                }
                SseEvent::Done => break,
                SseEvent::Malformed { payload, reason } => {
                    // Reported, not swallowed: it is how a server that returned a plain-text error
                    // through a 200 would otherwise look like silence.
                    deltas.push(ReplyDelta::Answer(format!(
                        "unparsable stream frame: {reason}"
                    )));
                    tracing::warn!("malformed SSE payload: {payload}");
                }
            }
        }
        if !deltas.is_empty() {
            apply_deltas(&mut reply, &deltas);
            if on_delta(&deltas) == StreamControl::Stop {
                break;
            }
        }
        if decoder.is_finished() {
            break;
        }
    }
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use smollm_models::Catalog;
    use std::path::Path;

    fn model() -> ModelEntry {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        Catalog::load_default(&root)
            .expect("catalog must load")
            .models
            .first()
            .expect("one seeded model")
            .clone()
    }

    /// The `-c` this machine's tier produces, so the clamp tests use a real number.
    const CTX: u32 = 16_384;

    fn model_id(id: &str) -> ModelEntry {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        Catalog::load_default(&root)
            .expect("catalog must load")
            .model(id)
            .expect("seeded model")
            .clone()
    }

    #[test]
    fn a_model_that_cannot_think_rejects_every_effort_instead_of_sending_an_empty_one() {
        let m = model_id("ternary-bonsai-1-7b");
        let r = &m.sampling.reasoning;
        // Its template has no thinking branch, so `low_behaves_like` is deliberately empty. Mapping
        // `low` onto it would put `reasoning_effort: ""` on the wire.
        assert_eq!(r.low_behaves_like, "");
        for effort in ["low", "medium", "xhigh", "high"] {
            assert!(
                matches!(
                    wire_effort(Some(effort), r),
                    Err(RequestError::EffortRejected(_))
                ),
                "{effort} must be rejected, not translated"
            );
        }
        assert_eq!(
            wire_effort(Some("none"), r).unwrap().as_deref(),
            Some("none")
        );
    }

    #[test]
    fn two_system_messages_become_one_at_the_front() {
        let merged = normalize(&[
            Message::user("hi"),
            Message::system("Be terse."),
            Message::assistant("ok"),
            Message::system("You are a helpful assistant"),
        ]);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].role, Role::System);
        assert_eq!(
            merged[0].content,
            "Be terse.\n\nYou are a helpful assistant"
        );
        assert_eq!(merged[1].role, Role::User);
        assert_eq!(merged[2].role, Role::Assistant);
    }

    #[test]
    fn the_body_puts_exactly_one_system_message_first() {
        let m = model();
        let req = ChatRequest::new(vec![
            Message::system("first"),
            Message::user("hello"),
            Message::system("second"),
        ]);
        let body = chat_body(&req, &m, CTX).expect("body");
        let messages = body["messages"].as_array().expect("array");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], "first\n\nsecond");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn high_is_never_sent_and_low_goes_out_as_xhigh() {
        let m = model();
        let r = &m.sampling.reasoning;
        // KNOWN_ISSUES.md: HTTP 500 with `reasoning_effort: "high"`. It must fail here, not there.
        assert!(r.never_send.contains(&"high".to_string()));
        assert!(matches!(
            wire_effort(Some("high"), r),
            Err(RequestError::EffortRejected(_))
        ));
        assert_eq!(
            wire_effort(Some("low"), r).unwrap().as_deref(),
            Some("xhigh"),
            "`low` is accepted by the template but measures as xhigh"
        );
        assert_eq!(
            wire_effort(Some("medium"), r).unwrap().as_deref(),
            Some("medium")
        );
        assert_eq!(
            wire_effort(Some("none"), r).unwrap().as_deref(),
            Some("none")
        );
        // No picker value: the server's `--reasoning auto` default stands and the field is absent.
        assert_eq!(wire_effort(None, r).unwrap(), None);

        let mut req = ChatRequest::new(vec![Message::user("x")]);
        req.reasoning_effort = Some("high".into());
        assert!(chat_body(&req, &m, CTX).is_err());
        req.reasoning_effort = Some("low".into());
        assert_eq!(
            chat_body(&req, &m, CTX).expect("body")["reasoning_effort"],
            "xhigh"
        );
        req.reasoning_effort = None;
        assert!(chat_body(&req, &m, CTX)
            .expect("body")
            .get("reasoning_effort")
            .is_none());
    }

    #[test]
    fn an_unfinished_tool_call_is_replayed_as_an_empty_object() {
        let m = model();
        assert!(m.agent_constraints.empty_tool_arguments_as_object);
        let call = ToolCall {
            id: "call_1".into(),
            name: "read_file".into(),
            arguments: String::new(),
        };
        let req = ChatRequest::new(vec![
            Message::user("read it"),
            Message {
                tool_calls: vec![call.clone()],
                ..Message::assistant("")
            },
            Message::tool_result("call_1", "the file"),
        ]);
        let body = chat_body(&req, &m, CTX).expect("body");
        let assistant = &body["messages"][1];
        assert_eq!(
            assistant["tool_calls"][0]["function"]["arguments"], "{}",
            "#112: empty arguments are an HTTP 500"
        );
        assert_eq!(assistant["role"], "assistant");
        assert_eq!(body["messages"][2]["tool_call_id"], "call_1");

        // Valid JSON is left exactly as it arrived; so is valid JSON that is not an object.
        let done = ToolCall {
            arguments: "{\"path\":\"a.txt\"}".into(),
            ..call
        };
        let body = chat_body(
            &ChatRequest::new(vec![Message {
                tool_calls: vec![done],
                ..Message::assistant("")
            }]),
            &m,
            CTX,
        )
        .expect("body");
        assert_eq!(
            body["messages"][0]["tool_calls"][0]["function"]["arguments"],
            "{\"path\":\"a.txt\"}"
        );
    }

    #[test]
    fn a_replayed_assistant_turn_carries_no_reasoning() {
        let m = model();
        // Bonsai-demo#183: re-rendering thinking into history misses the prompt cache every turn.
        let body = chat_body(
            &ChatRequest::new(vec![Message::assistant("the answer")]),
            &m,
            CTX,
        )
        .expect("body");
        let turn = &body["messages"][0];
        assert_eq!(turn["content"], "the answer");
        assert!(turn.get("reasoning_content").is_none());
        assert!(m.agent_constraints.do_not_echo_reasoning_into_history);
    }

    #[test]
    fn the_budget_never_ends_a_reply_mid_thought_and_never_overflows_the_context() {
        let m = model();
        let c = &m.agent_constraints;
        assert_eq!(clamp_budget(256, CTX, c), 4_096);
        assert_eq!(clamp_budget(1_000_000, CTX, c), 8_192);
        assert_eq!(clamp_budget(6_000, CTX, c), 6_000);
        // A context too small to split still gets the documented minimum.
        assert_eq!(clamp_budget(1_024, 4_096, c), 4_096);
        let body = chat_body(
            &ChatRequest {
                max_tokens: 256,
                ..ChatRequest::new(vec![Message::user("x")])
            },
            &m,
            CTX,
        )
        .expect("body");
        assert_eq!(body["max_tokens"], 4_096);
    }

    /// The fork answers a stream with no usage frame unless the request asks for one, and the meter
    /// has nothing to read without it — so the ask is part of every request, not an option.
    #[test]
    fn every_request_asks_the_server_to_report_its_token_usage() {
        let m = model();
        let body = chat_body(&ChatRequest::new(vec![Message::user("x")]), &m, CTX).expect("body");
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
    }

    #[test]
    fn the_usage_frame_is_what_says_how_full_the_window_now_is() {
        // The frame the pinned fork really sent, verbatim apart from the timings it also carries. The
        // prompt half is read alongside the total because it is the only evidence of a trim: a server
        // that dropped the front of the conversation reports what it kept, not what it was handed.
        assert_eq!(
            chunk_usage(&json!({
                "choices": [],
                "usage": {
                    "completion_tokens": 16,
                    "prompt_tokens": 15,
                    "total_tokens": 31,
                    "prompt_tokens_details": { "cached_tokens": 14 }
                }
            })),
            Some(Usage {
                prompt_tokens: 15,
                total_tokens: 31
            })
        );
        assert_eq!(
            chunk_usage(&json!({ "choices": [{ "delta": { "content": "hi" } }] })),
            None
        );
        // A server that never answers the ask leaves the meter unknown rather than guessing a size.
        assert_eq!(chunk_usage(&json!({ "choices": [], "timings": {} })), None);
    }

    #[test]
    fn tools_are_passed_through_only_when_given() {
        let m = model();
        let mut req = ChatRequest::new(vec![Message::user("x")]);
        assert!(chat_body(&req, &m, CTX)
            .expect("body")
            .get("tools")
            .is_none());
        req.tools = vec![json!({"type": "function", "function": {"name": "shell"}})];
        assert_eq!(
            chat_body(&req, &m, CTX).expect("body")["tools"][0]["function"]["name"],
            "shell"
        );
    }

    #[test]
    fn an_empty_conversation_is_refused_locally() {
        let m = model();
        assert!(matches!(
            chat_body(&ChatRequest::new(vec![]), &m, CTX),
            Err(RequestError::Empty)
        ));
    }

    #[test]
    fn sse_frames_survive_arbitrary_chunk_boundaries() {
        let mut decoder = SseDecoder::default();
        let wire = concat!(
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"let me think\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"hello\"},\"finish_reason\":\"stop\"}]}\n\n",
        );
        let bytes = wire.as_bytes();
        // Cut mid-payload: a partial frame cannot be emitted, so the first push yields nothing.
        let first = decoder.push(&bytes[..30]);
        assert!(first.is_empty(), "{first:?}");
        let second = decoder.push(&bytes[30..]);
        assert_eq!(second.len(), 2, "{second:?}");
        let SseEvent::Data(chunk) = &second[1] else {
            panic!("expected a data frame, got {:?}", second[1]);
        };
        let (deltas, finish) = chunk_deltas(chunk);
        assert_eq!(deltas, vec![ReplyDelta::Answer("hello".into())]);
        assert_eq!(finish.as_deref(), Some("stop"));
        assert!(!decoder.is_finished());

        let done = decoder.push(b"data: [DONE]\n\n");
        assert_eq!(done, vec![SseEvent::Done]);
        assert!(decoder.is_finished());
    }

    #[test]
    fn a_crlf_stream_decodes_the_same_way() {
        let mut decoder = SseDecoder::default();
        // A boundary split as "…\r" + "\n\r\n" is the reason the frame is found in the buffer.
        let events = decoder.push(b"data: {\"x\":1}\r");
        assert!(events.is_empty(), "{events:?}");
        let events = decoder.push(b"\n\r\ndata: {\"x\":2}\r\n\r\n");
        assert_eq!(events.len(), 2, "{events:?}");
        assert_eq!(events[0], SseEvent::Data(json!({"x": 1})));
        assert_eq!(events[1], SseEvent::Data(json!({"x": 2})));
    }

    #[test]
    fn an_unreadable_frame_is_surfaced_as_malformed_not_dropped() {
        let mut decoder = SseDecoder::default();
        let events = decoder.push(b"data: {\"choices\": [\n\n");
        assert!(
            matches!(&events[0], SseEvent::Malformed { payload, .. } if payload.contains("choices")),
            "{events:?}"
        );
    }

    #[test]
    fn chunks_map_onto_thought_answer_and_tool_call_deltas() {
        let (deltas, finish) = chunk_deltas(&json!({
            "choices": [{
                "delta": {
                    "reasoning_content": "thinking",
                    "content": "answer",
                    "tool_calls": [{
                        "index": 0,
                        "id": "c1",
                        "type": "function",
                        "function": {"name": "shell", "arguments": "{\"cmd\":"}
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        }));
        assert_eq!(
            deltas,
            vec![
                ReplyDelta::Thought("thinking".into()),
                ReplyDelta::Answer("answer".into()),
                ReplyDelta::ToolCall {
                    index: 0,
                    id: Some("c1".into()),
                    name: Some("shell".into()),
                    arguments: Some("{\"cmd\":".into()),
                }
            ]
        );
        assert_eq!(finish.as_deref(), Some("tool_calls"));

        let (empty, finish) =
            chunk_deltas(&json!({"choices": [{"delta": {}, "finish_reason": null}]}));
        assert!(empty.is_empty(), "{empty:?}");
        assert_eq!(finish, None);
        // A chunk with no choices at all — the heartbeat llama.cpp sends first — is not an error.
        let (none, _) = chunk_deltas(&json!({"id": "chatcmpl-1"}));
        assert!(none.is_empty(), "{none:?}");
    }

    #[test]
    fn tool_call_arguments_accumulate_across_chunks_in_index_order() {
        let mut reply = Reply::default();
        for chunk in [
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","function":{"name":"read_file","arguments":""}}]}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":\"x\"}"}}]}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":1,"id":"b","function":{"name":"list_dir","arguments":"{}"}}]}}]}),
        ] {
            let (deltas, _) = chunk_deltas(&chunk);
            apply_deltas(&mut reply, &deltas);
        }
        assert_eq!(reply.tool_calls.len(), 2);
        assert_eq!(reply.tool_calls[0].id, "a");
        assert_eq!(reply.tool_calls[0].name, "read_file");
        assert_eq!(reply.tool_calls[0].arguments, "{\"path\":\"x\"}");
        assert_eq!(reply.tool_calls[1].name, "list_dir");
        assert!(valid_object(&reply.tool_calls[1].arguments));
    }

    #[test]
    fn a_length_finish_is_recognised_as_running_out_of_budget() {
        let mut reply = Reply::default();
        assert!(!reply.ran_out_of_budget());
        reply.finish_reason = Some("length".into());
        assert!(reply.ran_out_of_budget());
    }

    #[test]
    fn a_server_status_keeps_the_template_message_it_returned() {
        let err = LlamaError::ServerError {
            status: 500,
            body: "{\"error\":{\"message\":\"system message must be first\"}}".into(),
        };
        let text = err.to_string();
        assert!(text.contains("500"), "{text}");
        assert!(text.contains("system message must be first"), "{text}");
    }

    #[test]
    fn a_local_refusal_travels_through_the_crate_error_type() {
        let err: LlamaError = RequestError::Empty.into();
        assert!(err.to_string().contains("at least one message"), "{err}");
    }
}
