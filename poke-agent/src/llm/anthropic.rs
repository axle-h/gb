//! The Anthropic Messages API: its own wire format, streaming parser and client. The request is
//! rendered straight from the conversation the turn loop keeps, never through chat completions.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::sync::Mutex;

use serde_json::{Map, Value, json};

use crate::llm::LlmError;
use crate::llm::client::{ChatEndpoint, http_agent, post_event_stream};
use crate::llm::compaction::is_turn_start;
use crate::llm::config::LlmConfig;
use crate::llm::protocol::{
    ChatRequest, Completion, Content, ContentPart, Fragment, FunctionCall, Message, Role, ToolCall, Usage, is_timeout,
};

const VERSION: &str = "2023-06-01";
/// Lets a request say what becomes of a thinking block whose conversation changed under it.
const THINKING_BINDING_BETA: &str = "thinking-binding-controls-2026-08-01";
/// Lets a request a safety classifier declines be answered by another model.
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
/// The models that take `fallbacks: "default"`; anything else would refuse the field.
const FALLBACK_MODELS: &[&str] = &["claude-fable-5-1", "claude-opus-5-5", "claude-opus-5", "claude-sonnet-5-5"];
/// `max_tokens` is required here, so `GB_MAX_TOKENS=0` sends this instead.
const UNCAPPED_MAX_TOKENS: u32 = 64_000;
/// Far more than one turn's `GB_MAX_TOOL_STEPS`, which is all that is ever replayed.
const REMEMBERED_REPLIES: usize = 64;

pub struct AnthropicClient {
    agent: ureq::Agent,
    url: String,
    api_key: String,
    /// Each reply's content blocks exactly as they streamed, signed thinking included, keyed by its
    /// first tool call's id: the history keeps only the prose and the calls.
    replies: Mutex<VecDeque<(String, Vec<Value>)>>,
}

impl AnthropicClient {
    pub fn new(config: &LlmConfig) -> Self {
        Self {
            agent: http_agent(config),
            url: config.messages_url(),
            api_key: config.api_key.clone(),
            replies: Mutex::new(VecDeque::new()),
        }
    }

    /// The reply that made exactly these calls, if it is still held.
    fn reply_for(&self, calls: &[ToolCall]) -> Option<Vec<Value>> {
        let first = calls.first()?;
        let replies = self.replies.lock().unwrap_or_else(|e| e.into_inner());
        let (_, blocks) = replies.iter().find(|(id, _)| *id == first.id)?;
        let ids = blocks.iter().filter(|block| block["type"] == "tool_use").map(|block| block["id"].as_str());
        ids.eq(calls.iter().map(|call| Some(call.id.as_str()))).then(|| blocks.clone())
    }

    fn remember(&self, completion: &Completion, blocks: Vec<Value>) {
        let Some(first) = completion.tool_calls.first() else { return };
        let mut replies = self.replies.lock().unwrap_or_else(|e| e.into_inner());
        replies.push_back((first.id.clone(), blocks));
        while replies.len() > REMEMBERED_REPLIES {
            replies.pop_front();
        }
    }
}

impl ChatEndpoint for AnthropicClient {
    fn stream_completion(
        &self,
        request: &ChatRequest,
        on_delta: &mut dyn FnMut(Fragment<'_>),
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Completion, LlmError> {
        let body = serde_json::to_string(&request_body(request, &|calls| self.reply_for(calls)))
            .map_err(|e| LlmError::Protocol(format!("could not encode the request: {e}")))?;
        let betas = betas(request);
        let mut headers = vec![("x-api-key", self.api_key.as_str()), ("anthropic-version", VERSION)];
        if !betas.is_empty() {
            headers.push(("anthropic-beta", betas.as_str()));
        }
        let response = post_event_stream(&self.agent, &self.url, &headers, body, describe_error_body)?;
        let (completion, blocks) = read_stream(BufReader::new(response.into_reader()), on_delta, cancelled)?;
        self.remember(&completion, blocks);
        Ok(completion)
    }
}

// ── Requests ─────────────────────────────────────────────────────────────────────────────────────

/// The whole `POST /v1/messages` body. `replay` answers an assistant message's calls with the
/// reply that made them, as it streamed.
pub fn request_body(request: &ChatRequest, replay: &dyn Fn(&[ToolCall]) -> Option<Vec<Value>>) -> Value {
    let (system, messages) = render(&request.messages, replay);
    let mut body = Map::new();
    body.insert("model".into(), json!(request.model));
    body.insert("max_tokens".into(), json!(request.max_tokens.unwrap_or(UNCAPPED_MAX_TOKENS)));
    if !system.is_empty() {
        body.insert("system".into(), json!(system));
    }
    body.insert("messages".into(), Value::Array(messages));
    if !request.tools.is_empty() {
        let tools = request.tools.iter().map(|tool| {
            json!({
                "name": tool.function.name,
                "description": tool.function.description,
                "input_schema": tool.function.parameters,
            })
        });
        body.insert("tools".into(), tools.collect());
    }
    if request.parallel_tool_calls == Some(false) {
        body.insert("tool_choice".into(), json!({ "type": "auto", "disable_parallel_tool_use": true }));
    }
    body.insert("thinking".into(), thinking(request));
    if let Some(effort) = request.reasoning_effort.as_deref().filter(|_| !thinking_off(request)) {
        body.insert("output_config".into(), json!({ "effort": effort }));
    }
    if let Some(temperature) = request.temperature {
        body.insert("temperature".into(), json!(temperature));
    }
    if offers_fallback(request) {
        body.insert("fallbacks".into(), json!("default"));
    }
    // The breakpoint follows the end of the conversation as it grows.
    body.insert("cache_control".into(), json!({ "type": "ephemeral" }));
    body.insert("stream".into(), json!(true));
    Value::Object(body)
}

/// `GB_REASONING_EFFORT=none`, which means no thinking in either dialect.
fn thinking_off(request: &ChatRequest) -> bool {
    request.reasoning_effort.as_deref() == Some("none")
}

fn thinking(request: &ChatRequest) -> Value {
    if thinking_off(request) {
        return json!({ "type": "disabled" });
    }
    // Should the history ever change under a replayed block, the block goes rather than the turn.
    let binding = json!({ "prefix_mismatch_behavior": "drop_block" });
    match request.reasoning {
        Some(budget) => json!({ "type": "enabled", "budget_tokens": budget.max_tokens, "block_binding": binding }),
        // Summarised, or the page's reasoning pane stays empty.
        None => json!({ "type": "adaptive", "display": "summarized", "block_binding": binding }),
    }
}

fn offers_fallback(request: &ChatRequest) -> bool {
    FALLBACK_MODELS.contains(&request.model.as_str())
}

fn betas(request: &ChatRequest) -> String {
    let mut betas = Vec::new();
    if !thinking_off(request) {
        betas.push(THINKING_BINDING_BETA);
    }
    if offers_fallback(request) {
        betas.push(FALLBACK_BETA);
    }
    betas.join(",")
}

/// The top-level `system` and the `messages`, with consecutive turns of one role merged.
fn render(messages: &[Message], replay: &dyn Fn(&[ToolCall]) -> Option<Vec<Value>>) -> (String, Vec<Value>) {
    let leading = messages.iter().take_while(|message| message.role == Role::System).count();
    let system: Vec<&str> = messages[..leading].iter().filter_map(Message::text).collect();

    // Thinking goes back signed for the turn under way and for no earlier one: each block is bound
    // to the tools and the history it was written under, and a new kind of decision or a
    // compaction changes both.
    let turn_start = messages.iter().rposition(is_turn_start).unwrap_or(0);

    let mut turns: Vec<(&'static str, Vec<Value>)> = Vec::new();
    for (index, message) in messages.iter().enumerate().skip(leading) {
        let (role, mut blocks) = blocks_of(message);
        if index > turn_start && message.role == Role::Assistant {
            if let Some(reply) = replay(&message.tool_calls) {
                blocks = reply;
            }
        }
        if blocks.is_empty() {
            continue;
        }
        match turns.last_mut() {
            Some((last, merged)) if *last == role => merged.extend(blocks),
            _ => turns.push((role, blocks)),
        }
    }
    let turns = turns.into_iter().map(|(role, content)| json!({ "role": role, "content": content }));
    (system.join("\n\n"), turns.collect())
}

fn blocks_of(message: &Message) -> (&'static str, Vec<Value>) {
    match message.role {
        Role::Assistant => {
            let mut blocks = content_blocks(message.content.as_ref());
            blocks.extend(message.tool_calls.iter().map(tool_use));
            ("assistant", blocks)
        }
        Role::Tool => {
            let mut result = Map::new();
            result.insert("type".into(), json!("tool_result"));
            result.insert("tool_use_id".into(), json!(message.tool_call_id.as_deref().unwrap_or_default()));
            if let Some(text) = message.text().filter(|text| !text.is_empty()) {
                result.insert("content".into(), json!(text));
            }
            ("user", vec![Value::Object(result)])
        }
        // Only the leading system message is the system prompt; a later one is said in turn.
        Role::User | Role::System => ("user", content_blocks(message.content.as_ref())),
    }
}

fn content_blocks(content: Option<&Content>) -> Vec<Value> {
    match content {
        None => Vec::new(),
        Some(Content::Text(text)) => text_block(text).into_iter().collect(),
        Some(Content::Parts(parts)) => parts
            .iter()
            .filter_map(|part| match part {
                ContentPart::Text { text } => text_block(text),
                ContentPart::ImageUrl { image_url } => Some(image_block(&image_url.url)),
            })
            .collect(),
    }
}

/// A text block of nothing but whitespace is a 400.
fn text_block(text: &str) -> Option<Value> {
    (!text.trim().is_empty()).then(|| json!({ "type": "text", "text": text }))
}

fn image_block(url: &str) -> Value {
    match url.strip_prefix("data:").and_then(|rest| rest.split_once(";base64,")) {
        Some((media_type, data)) => {
            json!({ "type": "image", "source": { "type": "base64", "media_type": media_type, "data": data } })
        }
        None => json!({ "type": "image", "source": { "type": "url", "url": url } }),
    }
}

fn tool_use(call: &ToolCall) -> Value {
    // `history_safe` has already made every call's arguments an object.
    let input = serde_json::from_str::<Map<String, Value>>(call.function.arguments.trim()).unwrap_or_default();
    json!({ "type": "tool_use", "id": call.id, "name": call.function.name, "input": input })
}

// ── The streaming parser ─────────────────────────────────────────────────────────────────────────

/// Consume an SSE body to `message_stop`, into the completion and the content blocks behind it.
pub fn read_stream(
    reader: impl BufRead,
    on_delta: &mut dyn FnMut(Fragment<'_>),
    cancelled: &dyn Fn() -> bool,
) -> Result<(Completion, Vec<Value>), LlmError> {
    let mut accumulator = StreamAccumulator::default();
    for line in reader.lines() {
        if cancelled() {
            return Err(LlmError::Cancelled);
        }
        let line = line.map_err(|e| {
            let detail = format!("the response stream broke: {e}");
            match is_timeout(&e) {
                true => LlmError::Timeout(detail),
                false => LlmError::Transport(detail),
            }
        })?;
        if accumulator.push_line(&line, on_delta)? {
            return Ok(accumulator.finish());
        }
    }
    // Half a reply: its last tool call may be half an object.
    Err(LlmError::Transport("the response stream ended before the message did".to_string()))
}

/// [`read_stream`]'s state machine, apart from the reader so a test can feed it lines.
#[derive(Debug, Default)]
pub struct StreamAccumulator {
    /// By the event's `index`; `Null` where a fallback took a block back.
    blocks: Vec<Value>,
    /// Each tool call's input as it streamed, by the same index.
    inputs: Vec<String>,
    content: String,
    reasoning: String,
    /// Everything the prompt cost, cached or not, which is how full the context is.
    input_tokens: Option<u64>,
    output_tokens: u64,
    stop_reason: Option<String>,
}

impl StreamAccumulator {
    /// Fold one SSE line in; `true` once the message is finished.
    pub fn push_line(&mut self, line: &str, on_delta: &mut dyn FnMut(Fragment<'_>)) -> Result<bool, LlmError> {
        // The `event:` line repeats the payload's `type`.
        let Some(payload) = line.strip_prefix("data:") else { return Ok(false) };
        let payload = payload.trim();
        if payload.is_empty() {
            return Ok(false);
        }
        let event: Value = serde_json::from_str(payload)
            .map_err(|e| LlmError::Protocol(format!("unparseable stream event ({e}): {payload}")))?;
        let index = event["index"].as_u64().map(|index| index as usize);

        match event["type"].as_str().unwrap_or_default() {
            "message_start" => {
                self.read_usage(&event["message"]["usage"]);
                report_dropped_thinking(&event["message"]);
            }
            "content_block_start" => {
                if let Some(index) = index {
                    self.start_block(index, event["content_block"].clone(), on_delta);
                }
            }
            "content_block_delta" => {
                if let Some(index) = index.filter(|index| *index < self.blocks.len()) {
                    self.apply_delta(index, &event["delta"], on_delta);
                }
            }
            "message_delta" => {
                if let Some(reason) = event["delta"]["stop_reason"].as_str() {
                    self.stop_reason = Some(reason.to_string());
                }
                self.read_usage(&event["usage"]);
            }
            "message_stop" => return Ok(true),
            "error" => return Err(stream_failure(&event["error"])),
            // `ping`, `content_block_stop` and whatever is added later.
            _ => {}
        }
        Ok(false)
    }

    fn read_usage(&mut self, usage: &Value) {
        if usage.get("input_tokens").is_some() {
            let fields = ["input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"];
            self.input_tokens = Some(fields.iter().map(|field| usage[*field].as_u64().unwrap_or(0)).sum());
        }
        if let Some(output) = usage["output_tokens"].as_u64() {
            self.output_tokens = output;
        }
    }

    fn start_block(&mut self, index: usize, block: Value, on_delta: &mut dyn FnMut(Fragment<'_>)) {
        if self.blocks.len() <= index {
            self.blocks.resize(index + 1, Value::Null);
            self.inputs.resize(index + 1, String::new());
        }
        match block["type"].as_str().unwrap_or_default() {
            // Another model took over: what it is sent back with omits the declined model's
            // thinking and calls, and its prose stands.
            "fallback" => {
                for earlier in &mut self.blocks[..index] {
                    if matches!(earlier["type"].as_str(), Some("thinking" | "redacted_thinking" | "tool_use")) {
                        *earlier = Value::Null;
                    }
                }
                return;
            }
            "text" if !self.content.is_empty() => {
                self.content.push_str("\n\n");
                on_delta(Fragment::Content("\n\n"));
            }
            "thinking" if !self.reasoning.is_empty() => {
                self.reasoning.push_str("\n\n");
                on_delta(Fragment::Reasoning("\n\n"));
            }
            _ => {}
        }
        self.blocks[index] = block;
    }

    fn apply_delta(&mut self, index: usize, delta: &Value, on_delta: &mut dyn FnMut(Fragment<'_>)) {
        let text = |field: &str| delta[field].as_str().unwrap_or_default();
        match delta["type"].as_str().unwrap_or_default() {
            "text_delta" => {
                let text = text("text");
                append(&mut self.blocks[index], "text", text);
                self.content.push_str(text);
                on_delta(Fragment::Content(text));
            }
            "thinking_delta" => {
                let text = text("thinking");
                append(&mut self.blocks[index], "thinking", text);
                self.reasoning.push_str(text);
                on_delta(Fragment::Reasoning(text));
            }
            "signature_delta" => append(&mut self.blocks[index], "signature", text("signature")),
            "input_json_delta" => self.inputs[index].push_str(text("partial_json")),
            _ => {}
        }
    }

    pub fn finish(self) -> (Completion, Vec<Value>) {
        let mut tool_calls = Vec::new();
        let mut blocks = Vec::new();
        for (mut block, input) in self.blocks.into_iter().zip(self.inputs) {
            if block.is_null() {
                continue;
            }
            if block["type"] == "tool_use" {
                // As written, not re-serialised: that would sort the keys.
                let arguments = match input.trim() {
                    "" => block["input"].to_string(),
                    _ => input,
                };
                if let Ok(object) = serde_json::from_str::<Map<String, Value>>(&arguments) {
                    block["input"] = Value::Object(object);
                }
                tool_calls.push(ToolCall {
                    id: block["id"].as_str().unwrap_or_default().to_string(),
                    kind: "function".to_string(),
                    function: FunctionCall { name: block["name"].as_str().unwrap_or_default().to_string(), arguments },
                });
            }
            blocks.push(block);
        }
        let usage = self.input_tokens.map(|prompt| Usage {
            prompt_tokens: prompt,
            completion_tokens: self.output_tokens,
            total_tokens: prompt + self.output_tokens,
            estimated: false,
        });
        let completion = Completion {
            content: self.content,
            reasoning: self.reasoning,
            tool_calls,
            usage,
            // The turn loop's word for a reply cut off at the ceiling.
            finish_reason: self.stop_reason.map(|reason| match reason.as_str() {
                "max_tokens" => "length".to_string(),
                _ => reason,
            }),
        };
        (completion, blocks)
    }
}

fn append(block: &mut Value, field: &str, text: &str) {
    let Some(block) = block.as_object_mut() else { return };
    match block.entry(field).or_insert_with(|| json!("")) {
        Value::String(existing) => existing.push_str(text),
        other => *other = json!(text),
    }
}

/// Not an error, since the turn goes on without the reasoning, but it means a replayed history
/// changed under its thinking, which `render` exists to prevent.
fn report_dropped_thinking(message: &Value) {
    if let Some(dropped) = message["input_transformations"].as_array().filter(|dropped| !dropped.is_empty()) {
        eprintln!("anthropic: the endpoint dropped thinking it was sent back: {}", Value::Array(dropped.clone()));
    }
}

// ── Errors ───────────────────────────────────────────────────────────────────────────────────────

/// The HTTP status an error `type` stands for, so one mid-stream is retried like one before it.
fn status_of(kind: &str) -> Option<u16> {
    Some(match kind {
        "invalid_request_error" => 400,
        "authentication_error" => 401,
        "billing_error" => 402,
        "permission_error" => 403,
        "not_found_error" => 404,
        "request_too_large" => 413,
        "rate_limit_error" => 429,
        "api_error" => 500,
        "timeout_error" => 504,
        "overloaded_error" => 529,
        _ => return None,
    })
}

fn describe(error: &Value) -> String {
    let kind = error["type"].as_str().unwrap_or_default();
    match (error["message"].as_str().map(str::trim).unwrap_or_default(), kind) {
        ("", "") => "the endpoint reported an error and said nothing about it".to_string(),
        ("", kind) => format!("the endpoint reported a {kind} and said nothing else"),
        (message, "") => message.to_string(),
        (message, kind) => format!("{message} [{kind}]"),
    }
}

fn stream_failure(error: &Value) -> LlmError {
    let message = describe(error);
    match status_of(error["type"].as_str().unwrap_or_default()) {
        // No headers exist mid-stream, so this rate limit is undated and keeps the backoff.
        Some(429) => LlmError::RateLimited { resets_at_ms: None, message },
        Some(status) => LlmError::Http { status, message },
        None => LlmError::Protocol(format!("the endpoint reported: {message}")),
    }
}

/// The human half of a non-2xx body, or the body itself when it is not the expected shape.
pub fn describe_error_body(body: &str) -> String {
    match serde_json::from_str::<Value>(body) {
        Ok(envelope) if envelope["error"].is_object() => describe(&envelope["error"]),
        _ => body.chars().take(400).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::protocol::{ReasoningBudget, StreamOptions, ToolSpec};

    fn call(id: &str, name: &str, arguments: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            kind: "function".into(),
            function: FunctionCall { name: name.into(), arguments: arguments.into() },
        }
    }

    fn request(messages: Vec<Message>) -> ChatRequest {
        ChatRequest {
            model: "claude-haiku-5-5".into(),
            messages,
            tools: vec![ToolSpec::new("read_map", "Read the map.", json!({ "type": "object", "properties": {} }))],
            parallel_tool_calls: Some(true),
            max_tokens: Some(8192),
            reasoning_effort: None,
            reasoning: None,
            temperature: None,
            stream: true,
            stream_options: StreamOptions { include_usage: true },
        }
    }

    fn never(_: &[ToolCall]) -> Option<Vec<Value>> {
        None
    }

    /// A finished turn and the start of the next, the shape every request after the first takes.
    fn two_turns() -> Vec<Message> {
        vec![
            Message::system("the rules"),
            Message::user("### Turn 1"),
            Message::assistant("Looking first.".into(), vec![call("toolu_a", "read_map", "{}")]),
            Message::tool_result("toolu_a", "a 3x3 room"),
            Message::user_with_image("Screenshot of the Game Boy screen", "data:image/png;base64,AAAA".into()),
            Message::assistant(String::new(), vec![call("toolu_b", "choose_action", r#"{"id":"Room:5,7:Warp"}"#)]),
            Message::tool_result("toolu_b", "Accepted."),
            Message::user("### Turn 2"),
            Message::assistant(String::new(), vec![call("toolu_c", "read_map", "")]),
            Message::tool_result("toolu_c", "a corridor"),
        ]
    }

    #[test]
    fn the_conversation_is_rendered_as_alternating_turns_of_blocks() {
        let body = request_body(&request(two_turns()), &never);
        assert_eq!(body["system"], "the rules", "the system prompt is top-level, not a message");

        let messages = body["messages"].as_array().expect("messages");
        let roles: Vec<&str> = messages.iter().map(|m| m["role"].as_str().unwrap()).collect();
        assert_eq!(roles, ["user", "assistant", "user", "assistant", "user", "assistant", "user"]);

        // A tool's answer and the picture beside it are one user turn, the result first.
        let answered = messages[2]["content"].as_array().unwrap();
        assert_eq!(answered[0], json!({ "type": "tool_result", "tool_use_id": "toolu_a", "content": "a 3x3 room" }));
        assert_eq!(answered[1]["text"], "Screenshot of the Game Boy screen");
        assert_eq!(answered[2]["source"], json!({ "type": "base64", "media_type": "image/png", "data": "AAAA" }));

        // Prose then calls, and arguments go as an object; an empty one as `{}`.
        assert_eq!(messages[1]["content"][0], json!({ "type": "text", "text": "Looking first." }));
        assert_eq!(messages[1]["content"][1]["input"], json!({}));
        assert_eq!(messages[3]["content"][0]["input"], json!({ "id": "Room:5,7:Warp" }));
        assert_eq!(messages[5]["content"][0]["input"], json!({}));
        // Nothing for an assistant message that said nothing but called.
        assert_eq!(messages[3]["content"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn a_message_with_nothing_to_say_is_left_out_rather_than_sent_empty() {
        let messages = vec![
            Message::system("rules"),
            Message::user("### Turn 1"),
            Message::assistant("  \n".into(), vec![]),
            Message::user("still there?"),
        ];
        let body = request_body(&request(messages), &never);
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1, "the two user messages either side merge: {messages:?}");
        assert_eq!(messages[0]["content"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn only_the_turn_under_way_is_sent_back_with_its_thinking() {
        let thought = |id: &str| {
            vec![
                json!({ "type": "thinking", "thinking": "", "signature": format!("sig-{id}") }),
                json!({ "type": "tool_use", "id": id, "name": "read_map", "input": {} }),
            ]
        };
        let replay = |calls: &[ToolCall]| Some(thought(&calls.first()?.id));
        let body = request_body(&request(two_turns()), &replay);
        let messages = body["messages"].as_array().unwrap();

        assert_eq!(messages[5]["content"][0]["signature"], "sig-toolu_c", "this turn's reply goes back verbatim");
        for earlier in [&messages[1], &messages[3]] {
            assert!(
                earlier["content"].as_array().unwrap().iter().all(|block| block["type"] != "thinking"),
                "an earlier turn's thinking is bound to tools and a history that may have moved: {earlier}",
            );
        }
    }

    #[test]
    fn a_held_reply_stands_in_only_for_the_calls_it_made() {
        let client = AnthropicClient::new(&LlmConfig::from_lookup(&|name| match name {
            "GB_PROVIDER" => Some("anthropic".into()),
            "ANTHROPIC_API_KEY" | "GB_MODEL" => Some("x".into()),
            _ => None,
        }).expect("valid"));
        let reply = vec![
            json!({ "type": "thinking", "thinking": "", "signature": "s" }),
            json!({ "type": "tool_use", "id": "toolu_a", "name": "read_map", "input": {} }),
            json!({ "type": "tool_use", "id": "toolu_b", "name": "read_party", "input": {} }),
        ];
        let completion = Completion {
            tool_calls: vec![call("toolu_a", "read_map", "{}"), call("toolu_b", "read_party", "{}")],
            ..Completion::default()
        };
        client.remember(&completion, reply.clone());

        assert_eq!(client.reply_for(&completion.tool_calls), Some(reply));
        assert_eq!(client.reply_for(&completion.tool_calls[..1]), None, "a history that lost a call is not that reply");
        assert_eq!(client.reply_for(&[]), None);

        for n in 0..REMEMBERED_REPLIES {
            client.remember(&Completion { tool_calls: vec![call(&format!("t{n}"), "wait", "{}")], ..Completion::default() }, vec![]);
        }
        assert_eq!(client.reply_for(&completion.tool_calls), None, "the oldest go first");
    }

    #[test]
    fn the_defaults_think_adaptively_cache_and_show_the_thinking() {
        let body = request_body(&request(two_turns()), &never);
        assert_eq!(body["max_tokens"], 8192);
        assert_eq!(body["stream"], true);
        assert_eq!(body["cache_control"], json!({ "type": "ephemeral" }));
        assert_eq!(body["thinking"], json!({
            "type": "adaptive",
            "display": "summarized",
            "block_binding": { "prefix_mismatch_behavior": "drop_block" },
        }));
        assert_eq!(body["tools"][0], json!({
            "name": "read_map",
            "description": "Read the map.",
            "input_schema": { "type": "object", "properties": {} },
        }));
        for absent in ["output_config", "temperature", "tool_choice", "fallbacks"] {
            assert!(body.get(absent).is_none(), "{absent}: {body}");
        }
        assert_eq!(betas(&request(Vec::new())), THINKING_BINDING_BETA);
    }

    #[test]
    fn the_reasoning_settings_say_the_same_things_in_this_dialect() {
        let effort = |value: &str| ChatRequest { reasoning_effort: Some(value.into()), ..request(Vec::new()) };

        let body = request_body(&effort("xhigh"), &never);
        assert_eq!(body["output_config"], json!({ "effort": "xhigh" }));
        assert_eq!(body["thinking"]["type"], "adaptive");

        let off = effort("none");
        let body = request_body(&off, &never);
        assert_eq!(body["thinking"], json!({ "type": "disabled" }), "`none` is no thinking, not an effort");
        assert!(body.get("output_config").is_none(), "{body}");
        assert_eq!(betas(&off), "", "no binding to control without thinking");

        let budget = ChatRequest { reasoning: Some(ReasoningBudget { max_tokens: 2048 }), ..request(Vec::new()) };
        let body = request_body(&budget, &never);
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 2048);

        let other = ChatRequest {
            max_tokens: None,
            temperature: Some(0.5),
            parallel_tool_calls: Some(false),
            ..request(Vec::new())
        };
        let body = request_body(&other, &never);
        assert_eq!(body["max_tokens"], UNCAPPED_MAX_TOKENS, "the field is required here");
        assert_eq!(body["temperature"], 0.5);
        assert_eq!(body["tool_choice"], json!({ "type": "auto", "disable_parallel_tool_use": true }));
    }

    #[test]
    fn only_a_model_that_offers_a_fallback_is_asked_for_one() {
        let opus = ChatRequest { model: "claude-opus-5-5".into(), ..request(Vec::new()) };
        assert_eq!(request_body(&opus, &never)["fallbacks"], "default");
        assert_eq!(betas(&opus), format!("{THINKING_BINDING_BETA},{FALLBACK_BETA}"));
        assert!(request_body(&request(Vec::new()), &never).get("fallbacks").is_none());
    }

    fn drain(lines: &[&str]) -> (Completion, Vec<Value>) {
        let mut accumulator = StreamAccumulator::default();
        let (mut seen, mut thought) = (String::new(), String::new());
        for line in lines {
            let done = accumulator
                .push_line(line, &mut |delta| match delta {
                    Fragment::Content(text) => seen.push_str(text),
                    Fragment::Reasoning(text) => thought.push_str(text),
                })
                .expect("parses");
            if done {
                break;
            }
        }
        let (completion, blocks) = accumulator.finish();
        assert_eq!(seen, completion.content, "every content delta is reported as it arrives");
        assert_eq!(thought, completion.reasoning, "and so is every thinking delta");
        (completion, blocks)
    }

    /// The events of one reply that thinks, says something and calls a tool.
    const REPLY: &[&str] = &[
        "event: message_start",
        r#"data: {"type":"message_start","message":{"id":"msg_1","usage":{"input_tokens":12,"cache_creation_input_tokens":300,"cache_read_input_tokens":5000,"output_tokens":1}}}"#,
        "",
        r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}"#,
        r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"The door is "}}"#,
        r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"south."}}"#,
        r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"EqQB"}}"#,
        r#"data: {"type":"content_block_stop","index":0}"#,
        "event: ping",
        r#"data: {"type":"ping"}"#,
        r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}"#,
        r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Heading south."}}"#,
        r#"data: {"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_1","name":"choose_action","input":{}}}"#,
        r#"data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":""}}"#,
        r#"data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"id\": \"Room"}}"#,
        r#"data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":":5,7:Warp\"}"}}"#,
        r#"data: {"type":"content_block_stop","index":2}"#,
        r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":87}}"#,
        r#"data: {"type":"message_stop"}"#,
    ];

    #[test]
    fn a_reply_streams_into_a_completion_and_the_blocks_to_send_back() {
        let (completion, blocks) = drain(REPLY);
        assert_eq!(completion.reasoning, "The door is south.");
        assert_eq!(completion.content, "Heading south.");
        assert_eq!(completion.finish_reason.as_deref(), Some("tool_use"));
        assert_eq!(completion.tool_calls.len(), 1);
        assert_eq!(completion.tool_calls[0].id, "toolu_1");
        assert_eq!(completion.tool_calls[0].function.arguments, r#"{"id": "Room:5,7:Warp"}"#, "as written");
        assert_eq!(completion.usage, Some(Usage {
            prompt_tokens: 5312, completion_tokens: 87, total_tokens: 5399, estimated: false,
        }), "cached tokens fill the context like any other");

        assert_eq!(blocks, vec![
            json!({ "type": "thinking", "thinking": "The door is south.", "signature": "EqQB" }),
            json!({ "type": "text", "text": "Heading south." }),
            json!({ "type": "tool_use", "id": "toolu_1", "name": "choose_action", "input": { "id": "Room:5,7:Warp" } }),
        ]);
    }

    #[test]
    fn a_reply_cut_off_at_the_ceiling_reads_as_one_to_the_turn_loop() {
        let (completion, _) = drain(&[
            r#"data: {"type":"message_start","message":{"usage":{"input_tokens":10,"output_tokens":1}}}"#,
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"I think"}}"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"max_tokens"},"usage":{"output_tokens":8192}}"#,
            r#"data: {"type":"message_stop"}"#,
        ]);
        assert_eq!(completion.finish_reason.as_deref(), Some("length"));
        assert!(completion.tool_calls.is_empty());
    }

    #[test]
    fn text_after_a_call_is_kept_apart_from_the_text_before_it() {
        let (completion, blocks) = drain(&[
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"First."}}"#,
            r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"t","name":"read_map","input":{}}}"#,
            r#"data: {"type":"content_block_start","index":2,"content_block":{"type":"text","text":""}}"#,
            r#"data: {"type":"content_block_delta","index":2,"delta":{"type":"text_delta","text":"Then."}}"#,
            r#"data: {"type":"message_stop"}"#,
        ]);
        assert_eq!(completion.content, "First.\n\nThen.");
        assert_eq!(completion.tool_calls[0].function.arguments, "{}", "a call with no input streamed is an empty object");
        assert_eq!(blocks[2]["text"], "Then.", "the blocks themselves are unchanged");
    }

    #[test]
    fn a_fallback_takes_back_the_declined_models_thinking_and_calls() {
        let (completion, blocks) = drain(&[
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":"a"}}"#,
            r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}"#,
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Let me"}}"#,
            r#"data: {"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"t1","name":"read_map","input":{}}}"#,
            r#"data: {"type":"content_block_start","index":3,"content_block":{"type":"fallback","from":{"model":"claude-opus-5-5"},"to":{"model":"claude-opus-4-8"}}}"#,
            r#"data: {"type":"content_block_start","index":4,"content_block":{"type":"tool_use","id":"t2","name":"read_party","input":{}}}"#,
            r#"data: {"type":"message_stop"}"#,
        ]);
        let names: Vec<&str> = completion.tool_calls.iter().map(|c| c.function.name.as_str()).collect();
        assert_eq!(names, ["read_party"], "the declined model's call is not one to answer");
        let kinds: Vec<&str> = blocks.iter().map(|b| b["type"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["text", "tool_use"], "its prose stands, its thinking and the marker go");
    }

    #[test]
    fn a_redacted_block_is_kept_whole_for_the_reply_it_belongs_to() {
        let (_, blocks) = drain(&[
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"redacted_thinking","data":"opaque"}}"#,
            r#"data: {"type":"content_block_stop","index":0}"#,
            r#"data: {"type":"message_stop"}"#,
        ]);
        assert_eq!(blocks, vec![json!({ "type": "redacted_thinking", "data": "opaque" })]);
    }

    fn failure(event: &str) -> LlmError {
        StreamAccumulator::default().push_line(&format!("data: {event}"), &mut |_| {}).expect_err("an error event")
    }

    #[test]
    fn an_error_mid_stream_is_retried_like_the_status_it_stands_for() {
        let overloaded = failure(r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#);
        assert_eq!(overloaded, LlmError::Http { status: 529, message: "Overloaded [overloaded_error]".into() });
        assert!(overloaded.is_retryable());

        let limited = failure(r#"{"type":"error","error":{"type":"rate_limit_error","message":"slow down"}}"#);
        assert!(matches!(limited, LlmError::RateLimited { resets_at_ms: None, .. }), "{limited}");

        let refused = failure(r#"{"type":"error","error":{"type":"invalid_request_error","message":"bad"}}"#);
        assert!(matches!(refused, LlmError::Http { status: 400, .. }) && !refused.is_retryable(), "{refused}");

        let unknown = failure(r#"{"type":"error","error":{"type":"brand_new_error"}}"#);
        assert!(matches!(unknown, LlmError::Protocol(_)), "{unknown}");
        assert!(format!("{unknown}").contains("brand_new_error"), "{unknown}");
    }

    #[test]
    fn a_stream_that_stops_before_the_message_does_is_a_broken_connection() {
        let body = REPLY[..12].join("\n");
        let failure = read_stream(body.as_bytes(), &mut |_| {}, &|| false).expect_err("no message_stop");
        assert!(matches!(failure, LlmError::Transport(_)) && failure.is_retryable(), "{failure}");

        let whole = REPLY.join("\n");
        let failure = read_stream(whole.as_bytes(), &mut |_| {}, &|| true).expect_err("cancelled");
        assert_eq!(failure, LlmError::Cancelled);
    }

    #[test]
    fn an_error_body_reports_its_message_and_kind_or_itself() {
        assert_eq!(
            describe_error_body(r#"{"type":"error","error":{"type":"not_found_error","message":"model: claude-x"}}"#),
            "model: claude-x [not_found_error]",
        );
        assert_eq!(describe_error_body("<html>502 Bad Gateway</html>"), "<html>502 Bad Gateway</html>");
    }

    /// One HTTP exchange with a canned answer, handing back what was asked.
    fn serve_once(status: &str, headers: &str, body: String) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
        let url = format!("http://{}", listener.local_addr().unwrap());
        let reply = format!("HTTP/1.1 {status}\r\n{headers}Connection: close\r\n\r\n{body}");
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("a connection");
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut asked = String::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
                asked.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            asked.push_str(&String::from_utf8(body).unwrap());
            stream.write_all(reply.as_bytes()).unwrap();
            asked
        });
        (url, handle)
    }

    fn client_for(url: &str) -> AnthropicClient {
        let url = url.to_string();
        AnthropicClient::new(&LlmConfig::from_lookup(&move |name| match name {
            "GB_PROVIDER" => Some("anthropic".into()),
            "ANTHROPIC_API_KEY" => Some("sk-ant-test".into()),
            "ANTHROPIC_BASE_URL" => Some(url.clone()),
            "GB_MODEL" => Some("claude-haiku-5-5".into()),
            _ => None,
        }).expect("valid"))
    }

    #[test]
    fn the_client_speaks_the_messages_api_and_sends_its_thinking_back_next_step() {
        let (url, server) = serve_once("200 OK", "Content-Type: text/event-stream\r\n", REPLY.join("\n") + "\n");
        let client = client_for(&url);
        let mut messages = vec![Message::system("rules"), Message::user("### Turn 1")];
        let completion = client
            .stream_completion(&request(messages.clone()), &mut |_| {}, &|| false)
            .expect("a completion");

        let asked = server.join().unwrap().to_ascii_lowercase();
        assert!(asked.starts_with("post /v1/messages "), "{asked}");
        assert!(asked.contains("x-api-key: sk-ant-test\r\n"), "{asked}");
        assert!(asked.contains("anthropic-version: 2023-06-01\r\n"), "{asked}");
        assert!(asked.contains(&format!("anthropic-beta: {THINKING_BINDING_BETA}\r\n")), "{asked}");
        assert!(!asked.contains("authorization:"), "the key goes in its own header: {asked}");

        // The next step of the same turn carries the signed thinking that led to the call.
        messages.push(Message::assistant(completion.content.clone(), completion.tool_calls.clone()));
        messages.push(Message::tool_result("toolu_1", "walked"));
        let body = request_body(&request(messages), &|calls| client.reply_for(calls));
        assert_eq!(body["messages"][1]["content"][0]["signature"], "EqQB");
    }

    #[test]
    fn a_dated_rate_limit_keeps_its_date_and_its_message() {
        let (url, server) = serve_once(
            "429 Too Many Requests",
            "Content-Type: application/json\r\nretry-after: 30\r\n",
            r#"{"type":"error","error":{"type":"rate_limit_error","message":"50 requests per minute"}}"#.into(),
        );
        let failure = client_for(&url)
            .stream_completion(&request(vec![Message::user("hi")]), &mut |_| {}, &|| false)
            .expect_err("429");
        server.join().unwrap();
        match failure {
            LlmError::RateLimited { resets_at_ms: Some(_), message } => {
                assert_eq!(message, "50 requests per minute [rate_limit_error]");
            }
            other => panic!("a 429 with Retry-After is a dated rate limit: {other}"),
        }
    }
}
