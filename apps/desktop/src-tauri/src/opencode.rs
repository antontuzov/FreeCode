//! The OpenCode app on this Mac, as a second route for the free-model tier.
//!
//! OpenCode's free models are served only to OpenCode's own runtime: a request from anywhere else is
//! refused by their gateway. The desktop app this module talks to is that runtime — it keeps a
//! daemon on loopback, writes its address and password to a state file under the user's home, and
//! relays the turn with the handshake the gateway expects. Using it is using the copy the user
//! already installed, not a way around it: the page says the OpenCode app has to be running, and a
//! turn without it takes the gateway route instead.
//!
//! The password in that file is a local secret. It is read when a request needs it, lives in
//! [`Service`] for the life of that turn, and is never logged, filed, returned over IPC, or written
//! back. The state file is untrusted input: only a loopback address is accepted, because sending that
//! password anywhere else would hand a local credential to whatever the file names.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};
use smollm_llama::{Reply, ReplyDelta, StreamControl};

/// The provider id the local service's own model list uses.
pub const PROVIDER: &str = "opencode";

/// How long a probe may wait. The daemon answers `/api/info` in a millisecond or two on a live
/// machine; this bound only decides how long a status poll waits when it is not there at all.
const PROBE_TIMEOUT: Duration = Duration::from_millis(900);

/// How long the service may take to acknowledge a prompt before the turn is called failed.
const ACCEPT_TIMEOUT: Duration = Duration::from_secs(30);

/// How long the event stream may stay silent mid-answer. Long enough for a slow first token, short
/// enough that a daemon that died holding the socket is reported rather than waited on forever.
const IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// How often the wait loop looks at the Stop flag while no event is arriving.
const STOP_POLL: Duration = Duration::from_millis(250);

/// The title a session opened from here carries in OpenCode's own session list.
const TITLE: &str = "smollm-studio";

/// Whether the last probe found the service. Read synchronously by the status document, which is
/// built on the blocking pool and cannot wait for a probe of its own.
static REACHABLE: AtomicBool = AtomicBool::new(false);

/// What the window learns about the service: that it is there, and which version — never its address
/// beyond loopback, and never its password.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub available: bool,
    pub version: Option<String>,
}

/// A live local service: where it listens and the password its own state file holds.
///
/// Deliberately not `Debug`: a derived one would print the password into any log line that formats a
/// turn, and there is no debugging value in that which is worth the secret.
#[derive(Clone)]
pub struct Service {
    base_url: String,
    password: String,
}

impl Service {
    fn client(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(2))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    }

    fn get(&self, client: &reqwest::Client, path: &str) -> reqwest::RequestBuilder {
        client
            .get(format!("{}{}", self.base_url, path))
            .basic_auth("opencode", Some(&self.password))
    }

    fn post(&self, client: &reqwest::Client, path: &str) -> reqwest::RequestBuilder {
        client
            .post(format!("{}{}", self.base_url, path))
            .basic_auth("opencode", Some(&self.password))
    }
}

/// Where OpenCode writes the state of its local service.
fn state_file() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_STATE_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir).join("opencode").join("service.json"));
    }
    let home = std::env::var_os("HOME").filter(|v| !v.is_empty())?;
    Some(
        PathBuf::from(home)
            .join(".local")
            .join("state")
            .join("opencode")
            .join("service.json"),
    )
}

/// Read the state file, refusing anything that is not a loopback service.
fn parse_state(raw: &str) -> Result<Service, String> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|e| format!("the OpenCode service state is unreadable: {e}"))?;
    let base_url = value
        .get("url")
        .and_then(Value::as_str)
        .ok_or("the OpenCode service state names no address")?
        .trim_end_matches('/')
        .to_string();
    let password = value
        .get("password")
        .and_then(Value::as_str)
        .ok_or("the OpenCode service state names no password")?
        .to_string();
    if password.is_empty() {
        return Err("the OpenCode service state holds an empty password".into());
    }
    // A state file that named a public host would have this app send a local credential to it.
    let loopback = ["http://127.0.0.1:", "http://localhost:", "http://[::1]:"]
        .iter()
        .any(|prefix| base_url.starts_with(prefix));
    if !loopback {
        return Err("the OpenCode service state does not point at a loopback address".into());
    }
    Ok(Service { base_url, password })
}

/// The service the state file describes, without proving it answers.
pub fn service() -> Result<Service, String> {
    let path =
        state_file().ok_or("this machine has no home directory to find the OpenCode service in")?;
    let raw = std::fs::read_to_string(&path).map_err(|_| {
        "the OpenCode app on this Mac is not running, so its free models are not reachable. \
         Open OpenCode once, or pick another free model."
            .to_string()
    })?;
    parse_state(&raw)
}

/// The service, proven to answer. `None` is the ordinary "OpenCode is not running" case.
pub async fn ready() -> Option<Service> {
    let service = service().ok()?;
    let answered = match service
        .get(&service.client(), "/api/info")
        .timeout(PROBE_TIMEOUT)
        .send()
        .await
    {
        Ok(response) => response.status().is_success(),
        Err(_) => false,
    };
    REACHABLE.store(answered, Ordering::Relaxed);
    answered.then_some(service)
}

/// Ask whether the service answers, and answer with its version when it does.
pub async fn status() -> Status {
    let Ok(service) = service() else {
        REACHABLE.store(false, Ordering::Relaxed);
        return Status::default();
    };
    let client = service.client();
    let response = match service
        .get(&client, "/api/info")
        .timeout(PROBE_TIMEOUT)
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => response,
        _ => {
            REACHABLE.store(false, Ordering::Relaxed);
            return Status::default();
        }
    };
    REACHABLE.store(true, Ordering::Relaxed);
    let version = response.json::<Value>().await.ok().and_then(|body| {
        body.get("version")
            .or_else(|| body.get("data").and_then(|d| d.get("version")))
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    Status {
        available: true,
        version,
    }
}

/// The tools a turn from here may not use.
///
/// The app already owns a tool loop against the project the user picked; a chat answered by OpenCode
/// answers in prose, and a model that silently wrote into the home directory would be doing work the
/// window never showed. The list is the one measured to keep the free tier's handshake intact —
/// denying *every* tool, or the wildcard action, makes OpenCode's gateway refuse the turn, so this
/// names writes and the shell rather than inventing a rule the service does not honour.
fn read_only_permissions() -> Value {
    json!([
        {"action": "edit", "resource": "*", "effect": "deny"},
        {"action": "write", "resource": "*", "effect": "deny"},
        {"action": "patch", "resource": "*", "effect": "deny"},
        {"action": "shell", "resource": "*", "effect": "deny"},
        {"action": "bash", "resource": "*", "effect": "deny"},
        {"action": "webfetch", "resource": "*", "effect": "deny"},
        {"action": "browser", "resource": "*", "effect": "deny"}
    ])
}

async fn body_message(response: reqwest::Response) -> String {
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    let parsed = serde_json::from_str::<Value>(&text).ok();
    let message = parsed
        .as_ref()
        .and_then(|v| {
            v.get("message")
                .and_then(Value::as_str)
                .or_else(|| {
                    v.get("data")
                        .and_then(|d| d.get("message"))
                        .and_then(Value::as_str)
                })
                .or_else(|| {
                    v.get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(Value::as_str)
                })
                .or_else(|| v.get("error").and_then(Value::as_str))
        })
        .map(str::to_string)
        .unwrap_or_else(|| text.trim().to_string());
    if message.is_empty() {
        format!("the OpenCode service answered {status}")
    } else {
        message
    }
}

/// Open a session bound to `model`. The id is what later prompts on this conversation continue.
pub async fn create_session(service: &Service, model: &str) -> Result<String, String> {
    let body = json!({
        "title": TITLE,
        "model": {"id": model, "providerID": PROVIDER},
        "permissions": read_only_permissions(),
    });
    let response = service
        .post(&service.client(), "/api/session")
        .json(&body)
        .timeout(ACCEPT_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("the OpenCode app would not open a session: {e}"))?;
    if !response.status().is_success() {
        return Err(body_message(response).await);
    }
    let value: Value = response
        .json()
        .await
        .map_err(|e| format!("the OpenCode app's session answer was unreadable: {e}"))?;
    value
        .get("data")
        .and_then(|d| d.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "the OpenCode app opened a session without naming it".to_string())
}

/// Whether a session the conversation remembers is still one the service holds.
pub async fn session_lives(service: &Service, session: &str) -> Result<bool, String> {
    let response = service
        .get(&service.client(), &format!("/api/session/{session}"))
        .timeout(PROBE_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("the OpenCode app could not be asked about the session: {e}"))?;
    Ok(response.status().is_success())
}

/// Point an existing session at the model the picker now names.
pub async fn switch_model(service: &Service, session: &str, model: &str) -> Result<(), String> {
    let body = json!({"model": {"id": model, "providerID": PROVIDER}});
    let response = service
        .post(&service.client(), &format!("/api/session/{session}/model"))
        .json(&body)
        .timeout(ACCEPT_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("the OpenCode app would not switch the session's model: {e}"))?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(body_message(response).await)
    }
}

/// Take back a turn that is still running. Best effort: the answer already streamed is kept either
/// way, and a service that has already finished has nothing to interrupt.
pub async fn interrupt(service: &Service, session: &str) {
    let _ = service
        .post(
            &service.client(),
            &format!("/api/session/{session}/interrupt"),
        )
        .json(&json!({}))
        .timeout(Duration::from_secs(5))
        .send()
        .await;
}

/// Read one event out of the byte buffer, or `None` when no complete block has arrived.
///
/// A block with no `data:` line is the service's keep-alive comment, and one that will not parse is
/// an event this build does not understand. Both are skipped rather than returned as nothing: a
/// caller that treated them as "no event yet" would leave the event queued behind them until the
/// next chunk happened to arrive.
fn take_event(buffer: &mut Vec<u8>) -> Option<Value> {
    loop {
        let position = buffer.windows(2).position(|pair| pair == b"\n\n")?;
        let block: Vec<u8> = buffer.drain(..position + 2).collect();
        let text = String::from_utf8_lossy(&block);
        let mut data = String::new();
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("data:") {
                data.push_str(rest.trim_start());
                data.push('\n');
            }
        }
        if data.trim().is_empty() {
            continue;
        }
        if let Ok(event) = serde_json::from_str::<Value>(data.trim_end()) {
            return Some(event);
        }
    }
}

/// The session id an event is about, when it names one.
fn event_session(event: &Value) -> Option<&str> {
    event
        .get("data")
        .and_then(|data| data.get("sessionID"))
        .and_then(Value::as_str)
}

/// What one `session.step.ended` counted.
fn step_tokens(event: &Value) -> (Option<u64>, Option<u64>) {
    let tokens = match event.get("data").and_then(|d| d.get("tokens")) {
        Some(tokens) => tokens,
        None => return (None, None),
    };
    let count = |field: &str| tokens.get(field).and_then(Value::as_u64).unwrap_or(0);
    let input = count("input");
    let total = input + count("output") + count("reasoning");
    (Some(input), Some(total))
}

/// The message a failed execution carries, if it names one.
fn event_failure(event: &Value) -> Option<String> {
    let error = event.get("data").and_then(|d| d.get("error"))?;
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| error.as_str())
        .unwrap_or("the OpenCode service could not answer this turn");
    Some(message.to_string())
}

/// Stream one turn through the local service.
///
/// The event stream is opened before the prompt is posted — the service writes `server.connected`
/// only once it has subscribed — so no delta can be missed between the prompt and the subscription.
/// Only events naming this session are considered; the stream carries every session the app has open.
pub async fn stream_turn(
    service: &Service,
    session: &str,
    text: &str,
    stop: &AtomicBool,
    mut on_delta: impl FnMut(&[ReplyDelta]) -> StreamControl,
) -> Result<Reply, String> {
    let client = service.client();
    let mut events = service
        .get(&client, "/api/event")
        .send()
        .await
        .and_then(|response| response.error_for_status())
        .map_err(|e| format!("the OpenCode app would not open its event stream: {e}"))?;

    // Wait until the subscription is acknowledged, not merely dialled.
    let mut buffer: Vec<u8> = Vec::new();
    let mut connected = false;
    while !connected {
        let chunk = events
            .chunk()
            .await
            .map_err(|e| format!("the OpenCode event stream stopped: {e}"))?;
        let Some(chunk) = chunk else {
            return Err("the OpenCode event stream closed before the turn began".into());
        };
        buffer.extend_from_slice(&chunk);
        while let Some(event) = take_event(&mut buffer) {
            if event.get("type").and_then(Value::as_str) == Some("server.connected") {
                connected = true;
            }
        }
    }

    let response = service
        .post(&client, &format!("/api/session/{session}/prompt"))
        .json(&json!({ "text": text }))
        .timeout(ACCEPT_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("the OpenCode app would not take the prompt: {e}"))?;
    if !response.status().is_success() {
        return Err(body_message(response).await);
    }

    let mut answer = String::new();
    let mut reasoning = String::new();
    let mut prompt_tokens = None;
    let mut total_tokens = None;
    let mut finish = None;
    let mut last_event = Instant::now();
    let mut stopped = false;

    loop {
        if stop.load(Ordering::Relaxed) {
            interrupt(service, session).await;
            stopped = true;
            break;
        }
        let chunk = tokio::select! {
            chunk = events.chunk() => chunk.map_err(|e| format!("the OpenCode event stream stopped: {e}"))?,
            _ = tokio::time::sleep(STOP_POLL) => {
                if last_event.elapsed() > IDLE_TIMEOUT {
                    return Err(
                        "the OpenCode app stopped sending events while this turn was running. \
                         Its window or the service may have quit; start OpenCode and try again."
                            .into(),
                    );
                }
                continue;
            }
        };
        let Some(chunk) = chunk else {
            return Err("the OpenCode event stream closed before the turn finished".into());
        };
        last_event = Instant::now();
        buffer.extend_from_slice(&chunk);
        let mut done = false;
        while let Some(event) = take_event(&mut buffer) {
            if event_session(&event) != Some(session) {
                continue;
            }
            let kind = event
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            match kind {
                "session.reasoning.delta" => {
                    let delta = event
                        .get("data")
                        .and_then(|d| d.get("delta"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    if !delta.is_empty() {
                        reasoning.push_str(&delta);
                        if on_delta(&[ReplyDelta::Thought(delta)]) == StreamControl::Stop {
                            interrupt(service, session).await;
                            stopped = true;
                            done = true;
                        }
                    }
                }
                "session.text.delta" => {
                    let delta = event
                        .get("data")
                        .and_then(|d| d.get("delta"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    if !delta.is_empty() {
                        answer.push_str(&delta);
                        if on_delta(&[ReplyDelta::Answer(delta)]) == StreamControl::Stop {
                            interrupt(service, session).await;
                            stopped = true;
                            done = true;
                        }
                    }
                }
                "session.text.ended" => {
                    // A service that reported the whole block without deltas would otherwise look like
                    // a turn that said nothing. One copy of the text is the rule: only when no delta
                    // landed does the block itself go out.
                    if answer.is_empty() {
                        let whole = event
                            .get("data")
                            .and_then(|d| d.get("text"))
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        if !whole.is_empty() {
                            answer.push_str(&whole);
                            if on_delta(&[ReplyDelta::Answer(whole)]) == StreamControl::Stop {
                                interrupt(service, session).await;
                                stopped = true;
                                done = true;
                            }
                        }
                    }
                }
                "session.step.ended" => {
                    let (prompt, total) = step_tokens(&event);
                    prompt_tokens = prompt.or(prompt_tokens);
                    total_tokens = total.or(total_tokens);
                    finish = event
                        .get("data")
                        .and_then(|d| d.get("finish"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .or(finish);
                }
                "session.step.failed" | "session.execution.failed" => {
                    let why = event_failure(&event).unwrap_or_else(|| {
                        "the OpenCode service could not answer this turn".into()
                    });
                    return Err(why);
                }
                "session.execution.succeeded" => done = true,
                _ => {}
            }
            if done {
                break;
            }
        }
        if done {
            break;
        }
    }

    Ok(Reply {
        text: answer,
        reasoning,
        tool_calls: Vec::new(),
        finish_reason: if stopped {
            Some("stopped".to_string())
        } else {
            finish.or_else(|| Some("stop".to_string()))
        },
        total_tokens,
        prompt_tokens,
        timings: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_state_file_must_point_at_this_machine() {
        let state =
            |url: &str| format!(r#"{{"url":"{url}","password":"local-secret","version":"2.1.3"}}"#);
        assert!(parse_state(&state("http://127.0.0.1:49374")).is_ok());
        assert!(parse_state(&state("http://localhost:49374/")).is_ok());
        assert!(parse_state(&state("https://opencode.ai")).is_err());
        assert!(parse_state(&state("http://192.168.1.20:49374")).is_err());
        assert!(parse_state(r#"{"url":"http://127.0.0.1:1"}"#).is_err());
        assert!(parse_state("not json").is_err());
    }

    #[test]
    fn a_trailing_slash_does_not_double_the_path() {
        let service =
            parse_state(r#"{"url":"http://127.0.0.1:49374/","password":"local-secret"}"#).unwrap();
        assert_eq!(service.base_url, "http://127.0.0.1:49374");
    }

    #[test]
    fn one_event_block_is_one_event() {
        let mut buffer =
            b"data: {\"type\":\"server.connected\"}\n\n: heartbeat\n\ndata: {\"type\":\"second\"}\n\n"
                .to_vec();
        let first = take_event(&mut buffer).unwrap();
        assert_eq!(first["type"], "server.connected");
        let second = take_event(&mut buffer).unwrap();
        assert_eq!(second["type"], "second");
        assert!(take_event(&mut buffer).is_none());
    }

    #[test]
    fn an_incomplete_block_waits_for_its_terminator() {
        let mut buffer = b"data: {\"type\":\"half\"}".to_vec();
        assert!(take_event(&mut buffer).is_none());
        buffer.extend_from_slice(b"\n\n");
        assert_eq!(take_event(&mut buffer).unwrap()["type"], "half");
    }

    #[test]
    fn a_failed_execution_names_the_reason() {
        let event = json!({
            "type": "session.execution.failed",
            "data": {"error": {"type": "provider.auth", "message": "the gateway refused it"}}
        });
        assert_eq!(
            event_failure(&event).as_deref(),
            Some("the gateway refused it")
        );
    }

    #[test]
    fn tokens_are_the_whole_fill() {
        let event = json!({
            "type": "session.step.ended",
            "data": {"finish": "stop", "tokens": {"input": 10, "output": 4, "reasoning": 6}}
        });
        assert_eq!(step_tokens(&event), (Some(10), Some(20)));
    }
}
