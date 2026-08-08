//! **W4** — the OpenAI chat-completions wire format, and the streaming parser over it.
//!
//! Deliberately free of HTTP. Everything here is a serde type or a state machine over lines of
//! text, which is what lets the awkward part — tool-call arguments arriving in fragments across many
//! SSE chunks — be tested against a byte string rather than against a network.
//!
//! Only the fields we actually send or read are modelled. An OpenAI-compatible endpoint returns a
//! great deal more (`id`, `created`, `system_fingerprint`, log probabilities), and every one of them
//! is a field that could change shape under us for no benefit; `serde` ignores unknown keys by
//! default and that is the whole handling this needs.

use std::io::BufRead;

use serde::{Deserialize, Serialize};

use crate::llm::LlmError;

// ── Messages ─────────────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// One message in the conversation, in the shape the endpoint wants it back.
///
/// `content` is a plain `String`. W5's `screenshot` tool needs the multi-part content form
/// (`[{"type":"image_url",…}]`) for its result, and that arrives as an enum here when it does —
/// modelling it now would be a variant nothing constructs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Assistant messages only, and the reason the whole turn loop exists.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// Tool messages only: which call this is the result of.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self::plain(Role::System, content)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::plain(Role::User, content)
    }

    /// The assistant turn that carries tool calls. `content` is kept even when empty-ish because
    /// several endpoints echo their own reasoning there and dropping it loses the thread.
    pub fn assistant(content: String, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            content: (!content.is_empty()).then_some(content),
            tool_calls,
            tool_call_id: None,
        }
    }

    /// The answer to one tool call. Every `tool_calls` entry needs exactly one of these before the
    /// next request, or the endpoint rejects the whole history — which is what §7.3's one-step
    /// rollback exists to guarantee.
    pub fn tool_result(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: Some(content.into()),
            tool_calls: Vec::new(),
            tool_call_id: Some(call_id.into()),
        }
    }

    fn plain(role: Role, content: impl Into<String>) -> Self {
        Self { role, content: Some(content.into()), tool_calls: Vec::new(), tool_call_id: None }
    }

    /// Roughly how many tokens this message costs, for the fallback in [`Usage::estimate`].
    pub fn approximate_tokens(&self) -> u64 {
        let text = self.content.as_deref().unwrap_or("").len()
            + self.tool_calls.iter().map(|c| c.function.name.len() + c.function.arguments.len()).sum::<usize>();
        (text as f64 / CHARS_PER_TOKEN).ceil() as u64
    }
}

/// English through a byte-pair tokeniser runs about this many characters per token. Only ever used
/// when the endpoint declines to report `usage` at all — see [`Usage::estimate`].
const CHARS_PER_TOKEN: f64 = 3.7;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    /// Always `"function"`. Sent back verbatim because the endpoint requires the field, not because
    /// there is a second kind.
    #[serde(rename = "type")]
    pub kind: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    /// A JSON *string*, not an object — that is how the API sends it, and it arrives in fragments.
    pub arguments: String,
}

impl ToolCall {
    /// The call's arguments as JSON. An empty string means "no arguments", which several endpoints
    /// send for a zero-parameter tool and which `serde_json` would otherwise reject.
    pub fn arguments(&self) -> Result<serde_json::Value, LlmError> {
        let raw = self.arguments.trim();
        if raw.is_empty() {
            return Ok(serde_json::Value::Object(Default::default()));
        }
        serde_json::from_str(raw).map_err(|e| {
            LlmError::Protocol(format!("tool call `{}` had unparseable arguments ({e}): {raw}", self.function.name))
        })
    }
}

impl std::ops::Deref for ToolCall {
    type Target = FunctionCall;
    fn deref(&self) -> &FunctionCall {
        &self.function
    }
}

// ── Requests ─────────────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ToolSpec {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: FunctionSpec,
}

#[derive(Debug, Clone, Serialize)]
pub struct FunctionSpec {
    pub name: &'static str,
    pub description: String,
    /// A JSON Schema object. `serde_json::Value` rather than a typed builder: these are written once,
    /// read by a model rather than by code, and a builder would be more machinery than the thing it
    /// builds.
    pub parameters: serde_json::Value,
}

impl ToolSpec {
    pub fn new(name: &'static str, description: impl Into<String>, parameters: serde_json::Value) -> Self {
        Self { kind: "function", function: FunctionSpec { name, description: description.into(), parameters } }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolSpec>,
    /// Explicit, though `true` is the default: §7.1 wants the whole batch of reads in one assistant
    /// message, because a batch is answered from one observed `GameState` and therefore cannot
    /// disagree with itself.
    pub parallel_tool_calls: bool,
    pub temperature: f32,
    pub stream: bool,
    /// ⚠️ Several endpoints report no `usage` on a streamed response unless asked. Some report none
    /// regardless, which is what [`Usage::estimate`] is for.
    pub stream_options: StreamOptions,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct StreamOptions {
    pub include_usage: bool,
}

// ── Responses ────────────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
    /// Set when the numbers came from [`Usage::estimate`] rather than from the endpoint, so the UI
    /// can say so rather than presenting a guess as a measurement.
    #[serde(default, skip_deserializing)]
    pub estimated: bool,
}

impl Usage {
    /// The fallback when an endpoint reports nothing: count characters. Wrong by tens of percent, and
    /// that is the point — a token gauge that degrades beats one that freezes at zero for a whole run.
    pub fn estimate(request: &[Message], completion: &Completion) -> Self {
        let prompt = request.iter().map(Message::approximate_tokens).sum();
        let reply = Message::assistant(completion.content.clone(), completion.tool_calls.clone())
            .approximate_tokens();
        Self { prompt_tokens: prompt, completion_tokens: reply, total_tokens: prompt + reply, estimated: true }
    }
}

/// One completed assistant turn, however many chunks it arrived in.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Completion {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Option<Usage>,
    pub finish_reason: Option<String>,
}

// ── The streaming parser ─────────────────────────────────────────────────────────────────────────

/// Consume an SSE body to the end of the completion.
///
/// `on_delta` is called with each fragment of assistant prose as it arrives — this is what makes the
/// browser show the model thinking rather than a spinner. `cancelled` is checked **on every line**,
/// which is one of §7.3's two cancel points: returning `Err(LlmError::Cancelled)` drops the reader,
/// which aborts the HTTP request and stops the endpoint billing for a completion that is already
/// answering a dead question.
pub fn read_stream(
    reader: impl BufRead,
    on_delta: &mut dyn FnMut(&str),
    cancelled: &dyn Fn() -> bool,
) -> Result<Completion, LlmError> {
    let mut accumulator = StreamAccumulator::default();
    for line in reader.lines() {
        if cancelled() {
            return Err(LlmError::Cancelled);
        }
        let line = line.map_err(|e| LlmError::Transport(format!("the response stream broke: {e}")))?;
        if accumulator.push_line(&line, on_delta)? {
            break;
        }
    }
    Ok(accumulator.finish())
}

/// The state machine [`read_stream`] drives, separated from the reader so a test can feed it lines
/// split wherever it likes.
#[derive(Debug, Default)]
pub struct StreamAccumulator {
    content: String,
    /// Indexed by the `index` field of the delta, which is how the API identifies *which* of several
    /// parallel calls a fragment belongs to.
    calls: Vec<PartialCall>,
    usage: Option<Usage>,
    finish_reason: Option<String>,
}

#[derive(Debug, Default, Clone)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}

impl StreamAccumulator {
    /// Fold one raw SSE line in. Returns `true` when the stream has said it is finished, so the
    /// caller stops reading rather than waiting for the connection to close.
    pub fn push_line(&mut self, line: &str, on_delta: &mut dyn FnMut(&str)) -> Result<bool, LlmError> {
        // Blank lines separate events and a leading `:` is a keep-alive comment. Anything that is not
        // a `data:` field — `event:`, `id:`, `retry:` — is not something this protocol uses.
        let Some(payload) = line.strip_prefix("data:") else { return Ok(false) };
        let payload = payload.trim();
        if payload.is_empty() {
            return Ok(false);
        }
        if payload == "[DONE]" {
            return Ok(true);
        }

        let chunk: StreamChunk = serde_json::from_str(payload)
            .map_err(|e| LlmError::Protocol(format!("unparseable stream chunk ({e}): {payload}")))?;

        // An error mid-stream arrives as a normal `data:` frame with a 200 already sent, so it cannot
        // be handled at the status-code layer.
        if let Some(error) = chunk.error {
            return Err(LlmError::Protocol(format!("the endpoint reported: {}", error.message)));
        }
        // ⚠️ The usage frame is the *last* one and carries an empty `choices` array. Take it before
        // looking at choices, or an endpoint that also sets `[DONE]` on it loses the numbers.
        if let Some(usage) = chunk.usage {
            self.usage = Some(usage);
        }

        for choice in chunk.choices {
            if let Some(text) = choice.delta.content.filter(|t| !t.is_empty()) {
                on_delta(&text);
                self.content.push_str(&text);
            }
            for delta in choice.delta.tool_calls {
                self.merge_call(delta);
            }
            if let Some(reason) = choice.finish_reason {
                self.finish_reason = Some(reason);
            }
        }
        Ok(false)
    }

    /// ⚠️ **The arguments of one call arrive across many chunks and must be concatenated before
    /// parsing.** The first fragment carries the id and the name; every one after it carries a few
    /// more characters of the JSON and nothing else.
    fn merge_call(&mut self, delta: ToolCallDelta) {
        // Not every endpoint sends `index`. Zero is the right default for the overwhelmingly common
        // single-call case, but it would then merge two *different* calls into one — so a fragment
        // that names an id the slot does not hold starts a new slot instead.
        let index = match delta.index {
            Some(index) => index,
            None => match (&delta.id, self.calls.last()) {
                (Some(id), Some(last)) if !last.id.is_empty() && last.id != *id => self.calls.len(),
                _ => self.calls.len().saturating_sub(1),
            },
        };
        if self.calls.len() <= index {
            self.calls.resize(index + 1, PartialCall::default());
        }
        let call = &mut self.calls[index];
        if let Some(id) = delta.id {
            call.id = id;
        }
        if let Some(function) = delta.function {
            if let Some(name) = function.name {
                call.name.push_str(&name);
            }
            if let Some(arguments) = function.arguments {
                call.arguments.push_str(&arguments);
            }
        }
    }

    pub fn finish(self) -> Completion {
        Completion {
            content: self.content,
            tool_calls: self
                .calls
                .into_iter()
                // A slot with no name was never a call: an endpoint that sends a sparse `index` leaves
                // gaps, and `resize` filled them.
                .filter(|call| !call.name.is_empty())
                .enumerate()
                .map(|(i, call)| ToolCall {
                    // An id is required when the result is sent back. Endpoints that omit it entirely
                    // (some local servers do) get a synthetic one rather than a 400 on the next
                    // request; the only requirement is that it is unique within the message.
                    id: if call.id.is_empty() { format!("call_{i}") } else { call.id },
                    kind: "function".to_string(),
                    function: FunctionCall { name: call.name, arguments: call.arguments },
                })
                .collect(),
            usage: self.usage,
            finish_reason: self.finish_reason,
        }
    }
}

#[derive(Debug, Deserialize)]
struct StreamChunk {
    #[serde(default)]
    choices: Vec<StreamChoice>,
    #[serde(default)]
    usage: Option<Usage>,
    #[serde(default)]
    error: Option<ApiError>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    #[serde(default)]
    delta: MessageDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct MessageDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ToolCallDelta>,
}

#[derive(Debug, Deserialize)]
struct ToolCallDelta {
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<FunctionDelta>,
}

#[derive(Debug, Deserialize)]
struct FunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

/// The body of an error, whether it arrives with a non-200 or as a frame inside a 200 stream.
#[derive(Debug, Deserialize)]
pub struct ApiError {
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub code: Option<String>,
}

/// Pull the human half out of an error body, falling back to the body itself when it is not the
/// shape we expected — which for a proxy or a gateway it very often is not.
pub fn describe_error_body(body: &str) -> String {
    #[derive(Deserialize)]
    struct Envelope {
        error: ApiError,
    }
    match serde_json::from_str::<Envelope>(body) {
        Ok(Envelope { error }) if !error.message.is_empty() => match error.code {
            Some(code) => format!("{} [{code}]", error.message),
            None => error.message,
        },
        _ => body.chars().take(400).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(lines: &[&str]) -> Completion {
        let mut accumulator = StreamAccumulator::default();
        let mut seen = String::new();
        for line in lines {
            let done = accumulator.push_line(line, &mut |delta| seen.push_str(delta)).expect("parses");
            if done {
                break;
            }
        }
        let completion = accumulator.finish();
        assert_eq!(seen, completion.content, "every content delta must be reported as it arrives");
        completion
    }

    /// §7.1's ⚠️: the arguments of one call arrive across many chunks. Split at every awkward place —
    /// mid-key, mid-string, on a brace — because the endpoint splits on token boundaries and has no
    /// idea where the JSON's are.
    #[test]
    fn parses_fragmented_tool_call_arguments() {
        let completion = drain(&[
            r#"data: {"choices":[{"delta":{"role":"assistant","content":"Heading "}}]}"#,
            r#"data: {"choices":[{"delta":{"content":"north."}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"choose_action","arguments":""}}]}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"i"}}]}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"d\": \"Pall"}}]}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"etTown:5,6:Warp\"}"}}]}}]}"#,
            r#"data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            r#"data: {"choices":[],"usage":{"prompt_tokens":900,"completion_tokens":31,"total_tokens":931}}"#,
            "data: [DONE]",
        ]);

        assert_eq!(completion.content, "Heading north.");
        assert_eq!(completion.finish_reason.as_deref(), Some("tool_calls"));
        assert_eq!(completion.usage, Some(Usage {
            prompt_tokens: 900, completion_tokens: 31, total_tokens: 931, estimated: false,
        }));
        assert_eq!(completion.tool_calls.len(), 1);
        let call = &completion.tool_calls[0];
        assert_eq!(call.id, "call_a");
        assert_eq!(call.function.name, "choose_action");
        assert_eq!(call.arguments().unwrap()["id"], "PalletTown:5,6:Warp");
    }

    /// §7.1: one assistant message may carry several calls, interleaved by `index`. This is the shape
    /// that makes `read_party` and `read_map` answerable from one observation.
    #[test]
    fn parallel_tool_calls_are_kept_apart_even_interleaved() {
        let completion = drain(&[
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","function":{"name":"read_map","arguments":"{}"}}]}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":1,"id":"b","function":{"name":"read_pa"}}]}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":1,"function":{"name":"rty","arguments":"{"}}]}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":1,"function":{"arguments":"}"}}]}}]}"#,
            "data: [DONE]",
        ]);

        let names: Vec<&str> = completion.tool_calls.iter().map(|c| c.function.name.as_str()).collect();
        assert_eq!(names, ["read_map", "read_party"], "a name can be fragmented too");
        assert_eq!(completion.tool_calls[1].id, "b");
    }

    /// ⚠️ Endpoint compatibility (§17 risk 3). An endpoint that sends no `index`, no id and no usage
    /// must still produce a well-formed call — with an id, because the tool result cannot be sent
    /// back without one.
    #[test]
    fn a_minimal_endpoint_still_yields_a_usable_call() {
        let completion = drain(&[
            r#"data: {"choices":[{"delta":{"tool_calls":[{"function":{"name":"wait"}}]}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"function":{"arguments":"{\"ticks\":3}"}}]}}]}"#,
            "data: [DONE]",
        ]);

        assert_eq!(completion.tool_calls.len(), 1, "the second fragment must not open a second call");
        assert_eq!(completion.tool_calls[0].id, "call_0");
        assert_eq!(completion.tool_calls[0].arguments().unwrap()["ticks"], 3);
        assert!(completion.usage.is_none());

        // …and the estimate is what stands in for the missing usage.
        let estimate = Usage::estimate(&[Message::user("a".repeat(370))], &completion);
        assert!(estimate.estimated);
        assert_eq!(estimate.prompt_tokens, 100);
        assert!(estimate.total_tokens > estimate.prompt_tokens);
    }

    /// Two different calls arriving with no `index` at all must not be concatenated into one — the id
    /// is the only thing distinguishing them.
    #[test]
    fn indexless_calls_split_on_a_new_id() {
        let completion = drain(&[
            r#"data: {"choices":[{"delta":{"tool_calls":[{"id":"a","function":{"name":"read_map","arguments":"{}"}}]}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"id":"b","function":{"name":"read_bag","arguments":"{}"}}]}}]}"#,
            "data: [DONE]",
        ]);
        let names: Vec<&str> = completion.tool_calls.iter().map(|c| c.function.name.as_str()).collect();
        assert_eq!(names, ["read_map", "read_bag"]);
    }

    /// Keep-alive comments, `event:` lines and blank separators are all normal traffic and none of
    /// them are data.
    #[test]
    fn non_data_lines_are_ignored() {
        let completion = drain(&[
            ": keep-alive",
            "",
            "event: message",
            "id: 42",
            r#"data:{"choices":[{"delta":{"content":"hi"}}]}"#, // no space after the colon
            "data: [DONE]",
            r#"data: {"choices":[{"delta":{"content":"never"}}]}"#,
        ]);
        assert_eq!(completion.content, "hi", "nothing after [DONE] is read");
    }

    /// An error inside a 200 stream: the status code already said everything was fine.
    #[test]
    fn a_mid_stream_error_frame_is_an_error() {
        let mut accumulator = StreamAccumulator::default();
        let failure = accumulator
            .push_line(r#"data: {"error":{"message":"context length exceeded","code":"400"}}"#, &mut |_| {})
            .expect_err("an error frame is not a completion");
        assert!(format!("{failure}").contains("context length exceeded"), "{failure}");
    }

    /// Truncated JSON is a reported error, never a panic and never a silently empty completion.
    #[test]
    fn a_corrupt_chunk_is_an_error() {
        let mut accumulator = StreamAccumulator::default();
        let failure = accumulator.push_line(r#"data: {"choices":[{"delta""#, &mut |_| {}).expect_err("truncated");
        assert!(matches!(failure, LlmError::Protocol(_)), "{failure}");
    }

    #[test]
    fn read_stream_stops_the_moment_a_turn_is_cancelled() {
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"one\"}}]}\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"two\"}}]}\n",
            "data: [DONE]\n",
        );
        let seen = std::cell::Cell::new(0);
        let failure = read_stream(
            std::io::BufReader::new(body.as_bytes()),
            &mut |_| {},
            &|| {
                seen.set(seen.get() + 1);
                seen.get() > 1
            },
        )
        .expect_err("cancellation is not a completion");
        assert!(matches!(failure, LlmError::Cancelled), "{failure}");
    }

    /// The request is what the endpoint is most likely to reject, so pin its shape: no nulls where a
    /// key should be absent, and `stream_options` present so `usage` comes back at all.
    #[test]
    fn the_request_serialises_to_the_documented_shape() {
        let request = ChatRequest {
            model: "gpt-test".to_string(),
            messages: vec![
                Message::system("be brief"),
                Message::assistant(String::new(), vec![ToolCall {
                    id: "c1".into(),
                    kind: "function".into(),
                    function: FunctionCall { name: "wait".into(), arguments: "{}".into() },
                }]),
                Message::tool_result("c1", "ok"),
            ],
            tools: vec![ToolSpec::new("wait", "do nothing", serde_json::json!({"type": "object"}))],
            parallel_tool_calls: true,
            temperature: 1.0,
            stream: true,
            stream_options: StreamOptions { include_usage: true },
        };
        let json = serde_json::to_value(&request).expect("serialises");

        assert_eq!(json["stream_options"]["include_usage"], true);
        assert_eq!(json["parallel_tool_calls"], true);
        assert_eq!(json["tools"][0]["type"], "function");
        assert_eq!(json["tools"][0]["function"]["name"], "wait");
        // An assistant message carrying only tool calls has no content key at all, and a system
        // message has no `tool_calls` key — both are `null`-intolerant on some endpoints.
        assert!(json["messages"][1].get("content").is_none());
        assert!(json["messages"][0].get("tool_calls").is_none());
        assert_eq!(json["messages"][2]["tool_call_id"], "c1");
        assert_eq!(json["messages"][2]["role"], "tool");
    }

    #[test]
    fn an_error_body_reports_its_message_or_itself() {
        assert_eq!(
            describe_error_body(r#"{"error":{"message":"you are out of credit","code":"insufficient_quota"}}"#),
            "you are out of credit [insufficient_quota]",
        );
        assert_eq!(describe_error_body("<html>502 Bad Gateway</html>"), "<html>502 Bad Gateway</html>");
    }
}
