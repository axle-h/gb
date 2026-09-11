//! The OpenAI chat-completions wire format, and the streaming parser over it.

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

/// What a message says: either a plain string, or the multi-part form an image needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Parts(Vec<ContentPart>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageUrl {
    /// A `data:image/png;base64,…` URL: nothing is hosted, as the run has no public address.
    pub url: String,
    /// `"low"` or `"high"`.
    pub detail: String,
    #[serde(skip, default = "default_image_tokens")]
    pub tokens: u64,
}

fn default_image_tokens() -> u64 { IMAGE_TOKENS }

/// How much detail the endpoint is asked to look at, and therefore what the picture costs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageDetail {
    Low,
    High,
}

impl ImageDetail {
    fn as_str(self) -> &'static str {
        match self {
            ImageDetail::Low => "low",
            ImageDetail::High => "high",
        }
    }
}

/// Roughly what a `width × height` picture costs at `detail`.
pub fn image_tokens(detail: ImageDetail, width: u32, height: u32) -> u64 {
    if detail == ImageDetail::Low || width == 0 || height == 0 {
        return IMAGE_TOKENS;
    }
    let (mut w, mut h) = (width as f64, height as f64);
    let fit = (2048.0 / w.max(h)).min(1.0);
    w *= fit;
    h *= fit;
    let short = 768.0 / w.min(h);
    w *= short;
    h *= short;
    IMAGE_TOKENS + 170 * (w / 512.0).ceil() as u64 * (h / 512.0).ceil() as u64
}

/// One message in the conversation, in the shape the endpoint wants it back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Content>,
    /// Assistant messages only.
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

    /// The assistant turn; `content` is kept because several endpoints echo their reasoning there.
    pub fn assistant(content: String, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            content: (!content.is_empty()).then_some(Content::Text(content)),
            tool_calls: history_safe(tool_calls),
            tool_call_id: None,
        }
    }

    /// The answer to one tool call.
    pub fn tool_result(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: Some(Content::Text(content.into())),
            tool_calls: Vec::new(),
            tool_call_id: Some(call_id.into()),
        }
    }

    /// A user message carrying a picture, which is how a `screenshot` result reaches the model.
    pub fn user_with_image(caption: impl Into<String>, data_url: String) -> Self {
        Self::user_with_image_detail(caption, data_url, ImageDetail::Low, IMAGE_TOKENS)
    }

    /// For a picture whose size makes the flat `"low"` price wrong; see [`image_tokens`].
    pub fn user_with_image_detail(
        caption: impl Into<String>, data_url: String, detail: ImageDetail, tokens: u64,
    ) -> Self {
        Self {
            role: Role::User,
            content: Some(Content::Parts(vec![
                ContentPart::Text { text: caption.into() },
                ContentPart::ImageUrl {
                    image_url: ImageUrl { url: data_url, detail: detail.as_str().to_string(), tokens },
                },
            ])),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    fn plain(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: Some(Content::Text(content.into())),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    /// The message's prose; `None` for a message that is only a picture.
    pub fn text(&self) -> Option<&str> {
        match self.content.as_ref()? {
            Content::Text(text) => Some(text),
            Content::Parts(parts) => parts.iter().find_map(|part| match part {
                ContentPart::Text { text } => Some(text.as_str()),
                ContentPart::ImageUrl { .. } => None,
            }),
        }
    }

    pub fn has_image(&self) -> bool {
        matches!(self.content.as_ref(), Some(Content::Parts(parts))
            if parts.iter().any(|part| matches!(part, ContentPart::ImageUrl { .. })))
    }

    /// Roughly how many tokens this message costs, for the fallback in [`Usage::estimate`].
    pub fn approximate_tokens(&self) -> u64 {
        let text = self.text().unwrap_or("").len()
            + self.tool_calls.iter().map(|c| c.function.name.len() + c.function.arguments.len()).sum::<usize>();
        // Each picture is charged what it costs, not a flat rate.
        let images: u64 = match self.content.as_ref() {
            Some(Content::Parts(parts)) => parts.iter().map(|part| match part {
                ContentPart::ImageUrl { image_url } => image_url.tokens,
                ContentPart::Text { .. } => 0,
            }).sum(),
            _ => 0,
        };
        (text as f64 / CHARS_PER_TOKEN).ceil() as u64 + images
    }
}

/// What one `detail: "low"` image costs, by OpenAI's published figure.
pub const IMAGE_TOKENS: u64 = 85;

/// English through a byte-pair tokeniser runs about this many characters per token.
const CHARS_PER_TOKEN: f64 = 3.7;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    /// Always `"function"`, which the endpoint requires sent back.
    #[serde(rename = "type")]
    pub kind: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    /// A JSON string, not an object, and it arrives in fragments.
    pub arguments: String,
}

/// Tool calls fit for the history: a call whose arguments are not a JSON object gets `{}` in
/// place, because the assistant message is replayed on every request after.
pub(crate) fn history_safe(tool_calls: Vec<ToolCall>) -> Vec<ToolCall> {
    tool_calls
        .into_iter()
        .map(|mut call| {
            let raw = call.function.arguments.trim();
            let object = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(raw);
            if object.is_err() {
                // The `tool_result` beside it rejects the call and quotes the raw text.
                call.function.arguments = "{}".to_string();
            }
            call
        })
        .collect()
}

impl ToolCall {
    /// An empty string means no arguments, as several endpoints send for a zero-parameter tool.
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
    /// A JSON Schema object.
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    /// `None` omits the key, which every endpoint reads as no ceiling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// How hard the model should think, for endpoints that expose it. `None` omits the key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    pub temperature: f32,
    pub stream: bool,
    /// Several endpoints stream no `usage` unless asked; [`Usage::estimate`] covers the rest.
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
    /// The numbers came from [`Usage::estimate`], so the UI can call them a guess.
    #[serde(default, skip_deserializing)]
    pub estimated: bool,
}

impl Usage {
    /// The fallback when an endpoint reports nothing: count characters, wrong by tens of percent.
    pub fn estimate(request: &[Message], completion: &Completion) -> Self {
        let prompt = request.iter().map(Message::approximate_tokens).sum();
        // Reasoning never enters the history, but it was billed.
        let reply = Message::assistant(completion.content.clone(), completion.tool_calls.clone())
            .approximate_tokens()
            + Message::assistant(completion.reasoning.clone(), Vec::new()).approximate_tokens();
        Self { prompt_tokens: prompt, completion_tokens: reply, total_tokens: prompt + reply, estimated: true }
    }
}

/// One completed assistant turn, however many chunks it arrived in.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Completion {
    pub content: String,
    /// What the model thought, for the endpoints that stream it apart from `content`.
    pub reasoning: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Option<Usage>,
    pub finish_reason: Option<String>,
}

// ── The streaming parser ─────────────────────────────────────────────────────────────────────────

/// One piece of a completion as it arrives, tagged with which channel it came in on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fragment<'a> {
    /// Assistant prose: the reply itself.
    Content(&'a str),
    /// The model's own thinking, from `reasoning_content` / `reasoning`.
    Reasoning(&'a str),
}

/// Consume an SSE body to the end of the completion.
pub fn read_stream(
    reader: impl BufRead,
    on_delta: &mut dyn FnMut(Fragment<'_>),
    cancelled: &dyn Fn() -> bool,
) -> Result<Completion, LlmError> {
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
            break;
        }
    }
    Ok(accumulator.finish())
}

/// The deadline expiring rather than the connection breaking: an [`LlmError::Timeout`].
fn is_timeout(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::TimedOut
        || format!("{error}").to_ascii_lowercase().contains("timeout")
}

/// [`read_stream`]'s state machine, apart from the reader so a test can split lines anywhere.
#[derive(Debug, Default)]
pub struct StreamAccumulator {
    content: String,
    reasoning: String,
    /// Indexed by the delta's `index`, which says which parallel call a fragment belongs to.
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
    /// Fold one SSE line in; `true` once the stream says it is finished.
    pub fn push_line(
        &mut self,
        line: &str,
        on_delta: &mut dyn FnMut(Fragment<'_>),
    ) -> Result<bool, LlmError> {
        // Blank lines separate events and a leading `:` is a keep-alive comment.
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

        // An error mid-stream is a `data:` frame after a 200, and carries its own status for the
        // retry loop.
        if let Some(error) = chunk.error {
            return Err(error.into_failure(chunk.provider.as_deref()));
        }
        // The usage frame is the last one and carries an empty `choices` array.
        if let Some(usage) = chunk.usage {
            self.usage = Some(usage);
        }

        for choice in chunk.choices {
            // Thinking first: where a chunk carries both, the reasoning led to the prose.
            if let Some(text) = choice.delta.reasoning_content.filter(|t| !t.is_empty()) {
                on_delta(Fragment::Reasoning(&text));
                self.reasoning.push_str(&text);
            }
            if let Some(text) = choice.delta.content.filter(|t| !t.is_empty()) {
                on_delta(Fragment::Content(&text));
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

    /// One call's arguments arrive across many chunks and are concatenated before parsing.
    fn merge_call(&mut self, delta: ToolCallDelta) {
        // Not every endpoint sends `index`.
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
            reasoning: self.reasoning,
            tool_calls: self
                .calls
                .into_iter()
                // A nameless slot is a gap `resize` filled for a sparse `index`.
                .filter(|call| !call.name.is_empty())
                .enumerate()
                .map(|(i, call)| ToolCall {
                    // An id is required when the result is sent back.
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
    /// Which upstream OpenRouter routed this request to.
    #[serde(default)]
    provider: Option<String>,
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
    /// A reasoning model's thinking, on a channel of its own.
    #[serde(default, alias = "reasoning")]
    reasoning_content: Option<String>,
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
    /// A string on OpenAI (`"insufficient_quota"`) and an integer on OpenRouter (`504`), so it
    /// accepts either or the whole chunk fails to parse.
    #[serde(default)]
    pub code: Option<ErrorCode>,
    /// OpenRouter's, and the tell that this is OpenRouter's envelope.
    #[serde(default)]
    pub metadata: Option<OpenRouterErrorMetadata>,
}

/// Whatever the endpoint put in `code`. See [`ApiError::code`].
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ErrorCode {
    Number(i64),
    Text(String),
}

impl ErrorCode {
    /// The HTTP status this code is: `"504"` counts, and a name answers `None`.
    pub fn status(&self) -> Option<u16> {
        let number = match self {
            Self::Number(number) => u16::try_from(*number).ok()?,
            Self::Text(text) => text.trim().parse::<u16>().ok()?,
        };
        (100..=599).contains(&number).then_some(number)
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Number(number) => write!(f, "{number}"),
            Self::Text(text) => write!(f, "{text}"),
        }
    }
}

/// The `metadata` on an OpenRouter error: which upstream provider failed, and how.
#[derive(Debug, Deserialize)]
pub struct OpenRouterErrorMetadata {
    #[serde(default)]
    pub error_type: Option<String>,
    #[serde(default)]
    pub provider_name: Option<String>,
}

impl ApiError {
    /// Whether this is OpenRouter's upstream-error envelope rather than a bare OpenAI-style one.
    fn is_openrouter(&self, chunk_provider: Option<&str>) -> bool {
        self.metadata.is_some() || chunk_provider.is_some_and(|name| !name.is_empty())
    }

    /// The provider that actually failed, from either half of the envelope.
    fn provider<'a>(&'a self, chunk_provider: Option<&'a str>) -> Option<&'a str> {
        self.metadata
            .as_ref()
            .and_then(|metadata| metadata.provider_name.as_deref())
            .or(chunk_provider)
            .map(str::trim)
            .filter(|name| !name.is_empty())
    }

    /// The human half: `Nvidia: Provider timed out after 47709ms`.
    pub fn describe(&self, chunk_provider: Option<&str>) -> String {
        let error_type = self
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.error_type.as_deref())
            .map(str::trim)
            .filter(|kind| !kind.is_empty());
        let message = match self.message.trim() {
            // Whatever the frame did carry becomes the message, `error_type` before `code`.
            "" => match (error_type, &self.code) {
                (Some(kind), _) => format!("the endpoint reported a {kind} and said nothing else"),
                (None, Some(code)) => format!("the endpoint reported {code} and said nothing else"),
                (None, None) => "the endpoint reported an error and said nothing about it".to_string(),
            },
            message => match &self.code {
                Some(code) if code.status().is_none() => format!("{message} [{code}]"),
                _ => message.to_string(),
            },
        };
        match self.provider(chunk_provider) {
            Some(provider) => format!("{provider}: {message}"),
            None => message,
        }
    }

    /// A status inside a 200 is the same status the non-200 path keys its retries on.
    fn into_failure(self, chunk_provider: Option<&str>) -> LlmError {
        let message = self.describe(chunk_provider);
        let status = match self.is_openrouter(chunk_provider) {
            true => self.code.as_ref().and_then(ErrorCode::status),
            // Not an envelope we recognise: say what it said and change nothing else.
            false => None,
        };
        match status {
            // No headers exist mid-stream, so this rate limit is undated and keeps the backoff.
            Some(429) => LlmError::RateLimited { resets_at_ms: None, message },
            Some(status) => LlmError::Http { status, message },
            None => LlmError::Protocol(format!("the endpoint reported: {message}")),
        }
    }
}

/// When a 429's quota reopens, in Unix milliseconds, from the two headers that can say so.
pub fn reset_at_ms(retry_after: Option<&str>, reset: Option<&str>, now_ms: u64) -> Option<u64> {
    // `Retry-After` may also be an HTTP-date.
    if let Some(seconds) = retry_after.and_then(|value| value.trim().parse::<u64>().ok()) {
        return Some(now_ms.saturating_add(seconds.saturating_mul(1000)));
    }
    let reset = reset?.trim().parse::<u64>().ok()?;
    match reset {
        // Unix milliseconds: anything below 10^12 cannot be one.
        _ if reset >= 1_000_000_000_000 => Some(reset),
        // Unix seconds.
        _ if reset >= 1_000_000_000 => Some(reset.saturating_mul(1000)),
        // Seconds from now.
        _ => Some(now_ms.saturating_add(reset.saturating_mul(1000))),
    }
}

/// The human half of an error body, or the body itself when it is not the expected shape.
pub fn describe_error_body(body: &str) -> String {
    #[derive(Deserialize)]
    struct Envelope {
        error: ApiError,
    }
    match serde_json::from_str::<Envelope>(body) {
        // The mid-stream describer too, so one error reads the same either side of the 200.
        Ok(Envelope { error }) if !error.message.is_empty() => error.describe(None),
        _ => body.chars().take(400).collect(),
    }
}

#[cfg(test)]
mod tests {
    /// A rendered map is `detail: "high"` and up to forty-five times a screenshot's price.
    #[test]
    fn a_high_detail_map_is_not_priced_like_a_screenshot() {
        use super::{image_tokens, ImageDetail, IMAGE_TOKENS};
        // The Game Boy screen at `screenshot::SCALE`, which is what `low` is for.
        assert_eq!(image_tokens(ImageDetail::Low, 480, 432), IMAGE_TOKENS);

        // Pallet Town, Celadon and Route 17, a thin strip scaled up until its short side is 768.
        assert_eq!(image_tokens(ImageDetail::High, 344, 330), 765);
        assert_eq!(image_tokens(ImageDetail::High, 856, 586), 1105);
        assert!(image_tokens(ImageDetail::High, 344, 2314) > 3_000);

        // A degenerate size must not divide by zero on the way to an estimate.
        assert_eq!(image_tokens(ImageDetail::High, 0, 0), IMAGE_TOKENS);
    }

    use super::*;

    fn drain(lines: &[&str]) -> Completion {
        let mut accumulator = StreamAccumulator::default();
        let mut seen = String::new();
        let mut thought = String::new();
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
        let completion = accumulator.finish();
        assert_eq!(seen, completion.content, "every content delta must be reported as it arrives");
        assert_eq!(thought, completion.reasoning, "and so must every reasoning delta");
        completion
    }

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

    /// `content` goes back into the history; `reasoning` is only shown on the page.
    #[test]
    fn reasoning_is_a_separate_channel_from_the_reply() {
        let completion = drain(&[
            r#"data: {"choices":[{"delta":{"role":"assistant","reasoning_content":"The lab is"}}]}"#,
            r#"data: {"choices":[{"delta":{"reasoning_content":" north of here."}}]}"#,
            r#"data: {"choices":[{"delta":{"content":"Heading north."}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","function":{"name":"wait","arguments":"{}"}}]}}]}"#,
            "data: [DONE]",
        ]);

        assert_eq!(completion.reasoning, "The lab is north of here.");
        assert_eq!(completion.content, "Heading north.", "the thinking is not part of the reply");
        assert_eq!(completion.tool_calls.len(), 1);

        // Reasoning is billed as completion tokens, so the estimate counts it.
        let with = Usage::estimate(&[], &completion);
        let without = Usage::estimate(&[], &Completion { reasoning: String::new(), ..completion });
        assert!(with.completion_tokens > without.completion_tokens, "{with:?} vs {without:?}");
    }

    /// LM Studio, vLLM and DeepSeek send `reasoning_content`; OpenRouter sends `reasoning`.
    #[test]
    fn the_other_spelling_of_the_reasoning_field_is_read_too() {
        let completion = drain(&[
            r#"data: {"choices":[{"delta":{"reasoning":"Thinking about it."}}]}"#,
            "data: [DONE]",
        ]);
        assert_eq!(completion.reasoning, "Thinking about it.");
    }

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

    #[test]
    fn a_mid_stream_error_frame_is_an_error() {
        let failure = error_frame(r#"{"error":{"message":"context length exceeded","code":"400"}}"#);
        assert!(format!("{failure}").contains("context length exceeded"), "{failure}");
    }

    fn error_frame(payload: &str) -> LlmError {
        StreamAccumulator::default()
            .push_line(&format!("data: {payload}"), &mut |_| {})
            .expect_err("an error frame is not a completion")
    }

    #[test]
    fn an_openrouter_upstream_error_is_a_retryable_status_not_a_parse_failure() {
        let failure = error_frame(concat!(
            r#"{"id":"gen-1786714339-L1kF3fCzShT9qsDgtgWd","object":"chat.completion.chunk","#,
            r#""created":1786714339,"model":"nvidia/nemotron-nano-12b-v2-vl:free","#,
            r#""provider":"Nvidia","choices":[],"#,
            r#""error":{"code":504,"message":"Provider timed out after 47709ms","#,
            r#""metadata":{"error_type":"timeout"}}}"#,
        ));

        assert!(matches!(failure, LlmError::Http { status: 504, .. }), "{failure}");
        assert!(failure.is_retryable(), "a provider that timed out is worth asking again: {failure}");
        let text = format!("{failure}");
        assert!(text.contains("Nvidia"), "which upstream failed is the actionable half: {text}");
        assert!(text.contains("Provider timed out after 47709ms"), "{text}");
        assert!(!text.contains("unparseable"), "the parser is not the thing that went wrong: {text}");
    }

    #[test]
    fn a_bare_error_frame_is_still_only_a_protocol_error() {
        let failure = error_frame(r#"{"choices":[],"error":{"code":503,"message":"overloaded"}}"#);
        assert!(matches!(failure, LlmError::Protocol(_)), "{failure}");
        assert!(!failure.is_retryable(), "{failure}");
        assert!(format!("{failure}").contains("overloaded"), "{failure}");

        // Either half of the envelope is enough to recognise it.
        for enveloped in [
            r#"{"provider":"Nvidia","error":{"code":503,"message":"overloaded"}}"#,
            r#"{"error":{"code":503,"message":"overloaded","metadata":{"provider_name":"Nvidia"}}}"#,
        ] {
            let failure = error_frame(enveloped);
            assert!(matches!(failure, LlmError::Http { status: 503, .. }), "{enveloped}: {failure}");
            assert!(format!("{failure}").contains("Nvidia"), "{failure}");
        }
    }

    #[test]
    fn an_error_code_is_read_whether_it_is_a_number_or_a_string() {
        assert_eq!(ErrorCode::Number(504).status(), Some(504));
        assert_eq!(ErrorCode::Text("504".into()).status(), Some(504), "some gateways send it quoted");
        assert_eq!(ErrorCode::Text("insufficient_quota".into()).status(), None);
        assert_eq!(ErrorCode::Number(0).status(), None, "not every number is a status");
        assert_eq!(ErrorCode::Number(-1).status(), None, "and it must not wrap into one");

        // A named code decides nothing but rides along in the message.
        let failure = error_frame(
            r#"{"provider":"Chutes","error":{"code":"insufficient_quota","message":"out of credit"}}"#,
        );
        assert!(matches!(failure, LlmError::Protocol(_)), "no status, so nothing to key a retry on");
        let text = format!("{failure}");
        assert!(text.contains("insufficient_quota") && text.contains("out of credit"), "{text}");

        // A status is not repeated: `LlmError::Http` prints it already.
        let failure = error_frame(r#"{"provider":"Nvidia","error":{"code":502,"message":"upstream died"}}"#);
        assert_eq!(format!("{failure}"), "the endpoint returned 502: Nvidia: upstream died");
    }

    #[test]
    fn an_openrouter_rate_limit_is_a_rate_limit_and_not_a_park() {
        let failure = error_frame(concat!(
            r#"{"provider":"Google AI Studio","choices":[],"error":{"code":429,"#,
            r#""message":"Provider returned error","metadata":{"error_type":"rate_limit"}}}"#,
        ));
        match &failure {
            LlmError::RateLimited { resets_at_ms, message } => {
                assert_eq!(*resets_at_ms, None, "a body cannot carry the headers that date one");
                assert!(message.contains("Google AI Studio"), "{message}");
            }
            other => panic!("a 429 is a rate limit wherever it arrives: {other}"),
        }
        assert!(failure.is_retryable());
    }

    #[test]
    fn an_openrouter_client_error_is_classified_but_not_retried() {
        let failure =
            error_frame(r#"{"provider":"OpenAI","error":{"code":400,"message":"tool schema rejected"}}"#);
        assert!(matches!(failure, LlmError::Http { status: 400, .. }), "{failure}");
        assert!(!failure.is_retryable(), "{failure}");
    }

    #[test]
    fn an_error_with_no_message_still_says_something() {
        let failure = error_frame(r#"{"provider":"Nvidia","error":{"metadata":{"error_type":"timeout"}}}"#);
        let text = format!("{failure}");
        assert!(text.contains("Nvidia") && text.contains("timeout"), "{text}");
    }

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

    /// No nulls where a key should be absent, and `stream_options` so `usage` comes back.
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
            parallel_tool_calls: Some(true),
            max_tokens: None,
            reasoning_effort: None,
            temperature: 1.0,
            stream: true,
            stream_options: StreamOptions { include_usage: true },
        };
        let json = serde_json::to_value(&request).expect("serialises");
        // Absent rather than null: `max_tokens: null` is a 400 on several endpoints.
        assert!(json.get("max_tokens").is_none(), "{json}");
        assert!(json.get("reasoning_effort").is_none(), "{json}");

        let capped = ChatRequest {
            max_tokens: Some(8192),
            reasoning_effort: Some("none".to_string()),
            ..request.clone()
        };
        let json_capped = serde_json::to_value(&capped).expect("serialises");
        assert_eq!(json_capped["max_tokens"], 8192);
        assert_eq!(json_capped["reasoning_effort"], "none");

        assert_eq!(json["stream_options"]["include_usage"], true);
        assert_eq!(json["parallel_tool_calls"], true);
        assert_eq!(json["tools"][0]["type"], "function");
        assert_eq!(json["tools"][0]["function"]["name"], "wait");
        // No `content` on a tool-calls-only message, no `tool_calls` on a system one.
        assert!(json["messages"][1].get("content").is_none());
        assert!(json["messages"][0].get("tool_calls").is_none());
        assert_eq!(json["messages"][2]["tool_call_id"], "c1");
        assert_eq!(json["messages"][2]["role"], "tool");
        // An ordinary message's content is a bare string, not a one-element array.
        assert_eq!(json["messages"][0]["content"], "be brief");
    }

    #[test]
    fn an_image_rides_on_a_user_message_in_the_multi_part_form() {
        let message = Message::user_with_image("look at this", "data:image/png;base64,AAAA".to_string());
        let json = serde_json::to_value(&message).expect("serialises");

        assert_eq!(json["role"], "user");
        assert_eq!(json["content"][0]["type"], "text");
        assert_eq!(json["content"][0]["text"], "look at this");
        assert_eq!(json["content"][1]["type"], "image_url");
        assert_eq!(json["content"][1]["image_url"]["url"], "data:image/png;base64,AAAA");
        assert_eq!(json["content"][1]["image_url"]["detail"], "low");

        assert_eq!(serde_json::from_value::<Message>(json).expect("deserialises"), message);

        assert_eq!(message.text(), Some("look at this"), "the caption is still readable as prose");
        assert!(message.has_image());
        assert!(!Message::user("no picture here").has_image());
    }

    #[test]
    fn an_image_is_estimated_by_the_flat_rate_rather_than_by_its_length() {
        let caption = "look at this";
        let big = Message::user_with_image(caption, format!("data:image/png;base64,{}", "A".repeat(40_000)));
        let text_only = Message::user(caption);

        assert_eq!(big.approximate_tokens(), text_only.approximate_tokens() + IMAGE_TOKENS);
        assert!(big.approximate_tokens() < 200, "a 40 kB data URL must not be charged as 40 kB of prose");
    }

    #[test]
    fn an_error_body_reports_its_message_or_itself() {
        assert_eq!(
            describe_error_body(r#"{"error":{"message":"you are out of credit","code":"insufficient_quota"}}"#),
            "you are out of credit [insufficient_quota]",
        );
        assert_eq!(describe_error_body("<html>502 Bad Gateway</html>"), "<html>502 Bad Gateway</html>");

        assert_eq!(
            describe_error_body(
                r#"{"error":{"code":504,"message":"Provider timed out","metadata":{"provider_name":"Nvidia"}}}"#
            ),
            "Nvidia: Provider timed out",
        );
    }

    #[test]
    fn a_tool_call_that_is_not_a_json_object_cannot_reach_the_history() {
        use super::{Message, ToolCall, FunctionCall};
        let call = |arguments: &str| ToolCall {
            id: "call_1".into(),
            kind: "function".into(),
            function: FunctionCall { name: "choose_action".into(), arguments: arguments.into() },
        };
        let sent = |arguments: &str| {
            Message::assistant(String::new(), vec![call(arguments)]).tool_calls[0]
                .function
                .arguments
                .clone()
        };

        // Nothing at all, and a fragment cut off mid-object.
        assert_eq!(sent(""), "{}");
        assert_eq!(sent(r#"{"id": "PalletTown:5"#), "{}");
        // JSON, but not an object.
        assert_eq!(sent("[1, 2]"), "{}");
        assert_eq!(sent("\"walk north\""), "{}");

        // A good call is left as written: re-serialising would sort its keys.
        let good = r#"{"id":"PalletTown:5,6:Warp","summary":"heading to Route 1"}"#;
        assert_eq!(sent(good), good);

        // The call itself survives whatever happens to its arguments.
        let message = Message::assistant(String::new(), vec![call("{oops")]);
        assert_eq!(message.tool_calls.len(), 1);
        assert_eq!(message.tool_calls[0].id, "call_1");
    }

    /// The three units `X-RateLimit-Reset` is sent in, which are told apart by magnitude alone.
    #[test]
    fn a_rate_limit_reset_is_read_in_whichever_unit_it_was_sent() {
        const NOW: u64 = 1_786_824_000_000;
        let at = |value: &str| super::reset_at_ms(None, Some(value), NOW);

        // Unix milliseconds — what OpenRouter sends.
        assert_eq!(at("1786824600000"), Some(1_786_824_600_000));
        // Unix seconds.
        assert_eq!(at("1786824600"), Some(1_786_824_600_000));
        // Seconds from now.
        assert_eq!(at("600"), Some(NOW + 600_000));

        // `Retry-After` wins: a delta cannot be wrong about our clock.
        assert_eq!(super::reset_at_ms(Some("30"), Some("1786824600000"), NOW), Some(NOW + 30_000));
        // ...but only when it parses.
        assert_eq!(
            super::reset_at_ms(Some("Fri, 14 Aug 2026 12:10:00 GMT"), Some("600"), NOW),
            Some(NOW + 600_000),
        );

        // `None` is the endpoint not saying, which keeps the ordinary backoff.
        assert_eq!(super::reset_at_ms(None, None, NOW), None);
        assert_eq!(super::reset_at_ms(None, Some("soon"), NOW), None);
    }
}
