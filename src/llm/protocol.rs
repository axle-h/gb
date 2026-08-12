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

/// What a message says: either a plain string, or the multi-part form an image needs.
///
/// `#[serde(untagged)]` is what makes the plain case serialise as `"content": "…"` rather than as a
/// one-element array — which matters, because a *tool* message is only allowed the string form on
/// several endpoints (see [`Message::user_with_image`]).
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
    /// A `data:image/png;base64,…` URL. Nothing is hosted: the run has no public address, and a
    /// screenshot that outlived the turn would be a privacy question nobody asked for.
    pub url: String,
    /// `"low"` or `"high"`. A screenshot is `"low"`: the Game Boy screen is 160×144 in four shades,
    /// so there is no detail for the expensive tier to find, and §8's ⚠️ is that pictures dominate
    /// the token cost of a run. A **map** is `"high"` — it is up to 1600 px on a side and squashing
    /// it into one 512×512 tile would turn forty squares of terrain into mush.
    pub detail: String,
    /// What this picture is estimated to cost, from its pixel dimensions at the moment it was
    /// encoded.
    ///
    /// ⚠️ **`serde(skip)`, because it is ours and not the wire's** — and defaulted on the way back
    /// in, so a history deserialised from anywhere prices its images at the `"low"` flat rate rather
    /// than at zero.
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
///
/// OpenAI's published tiling: fit inside 2048×2048, scale the *shortest* side to 768, then
/// `85 + 170 × ⌈w/512⌉ × ⌈h/512⌉`. Other providers differ — Anthropic caps the long edge and charges
/// about `w·h/750` — but every one of them charges by area, and the estimate only ever has to be
/// closer than the flat 85 it replaces.
///
/// ⚠️ **It has to be roughly right, not exact.** [`Usage::estimate`] is what decides when the history
/// is compacted, and a `"high"` map priced at 85 is out by 10–45×, so a genuinely full context would
/// never compact at all on an endpoint that reports no `usage` of its own. That is the failure this
/// function exists to prevent, not a billing question.
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
            content: (!content.is_empty()).then_some(Content::Text(content)),
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
            content: Some(Content::Text(content.into())),
            tool_calls: Vec::new(),
            tool_call_id: Some(call_id.into()),
        }
    }

    /// A user message carrying a picture, which is how a `screenshot` result reaches the model.
    ///
    /// ⚠️ **The image cannot ride on the `tool` message that answered the call.** OpenAI's schema
    /// allows an array of content parts on a tool result but only *text* parts in it, and several
    /// compatible endpoints reject an `image_url` there outright. So the call is answered with a
    /// sentence and the picture follows as a user message — which every endpoint accepts, and which
    /// keeps §7.3's rule that every `tool_call` has exactly one matching result.
    pub fn user_with_image(caption: impl Into<String>, data_url: String) -> Self {
        Self::user_with_image_detail(caption, data_url, ImageDetail::Low, IMAGE_TOKENS)
    }

    /// As [`Self::user_with_image`], for a picture whose size makes the flat `"low"` price a lie —
    /// see [`image_tokens`].
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

    /// The message's prose, for anything that wants to read rather than send it. `None` for a
    /// message that is only a picture.
    pub fn text(&self) -> Option<&str> {
        match self.content.as_ref()? {
            Content::Text(text) => Some(text),
            Content::Parts(parts) => parts.iter().find_map(|part| match part {
                ContentPart::Text { text } => Some(text.as_str()),
                ContentPart::ImageUrl { .. } => None,
            }),
        }
    }

    /// Whether this message carries a picture. W6 evicts these first, which is the reason it is
    /// asked as a question rather than derived by whoever needs it.
    pub fn has_image(&self) -> bool {
        matches!(self.content.as_ref(), Some(Content::Parts(parts))
            if parts.iter().any(|part| matches!(part, ContentPart::ImageUrl { .. })))
    }

    /// Roughly how many tokens this message costs, for the fallback in [`Usage::estimate`].
    ///
    /// ⚠️ The base64 payload is deliberately **not** counted by its length — a 3 KB data URL is
    /// four thousand characters and about eighty tokens, so charging it as text would overstate the
    /// context by fifty times and trip compaction on a history that is nowhere near full.
    pub fn approximate_tokens(&self) -> u64 {
        let text = self.text().unwrap_or("").len()
            + self.tool_calls.iter().map(|c| c.function.name.len() + c.function.arguments.len()).sum::<usize>();
        // ⚠️ Each picture is charged what *it* costs, not a flat rate. A map is `detail: "high"` and
        // up to forty-five times the price of a screenshot; charging both 85 would leave a full
        // context reading as a nearly empty one, and compaction would never run.
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

/// What one `detail: "low"` image costs. OpenAI's published figure, and the right order of magnitude
/// everywhere else; only ever used when the endpoint reports no `usage` of its own.
pub const IMAGE_TOKENS: u64 = 85;

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
    ///
    /// ⚠️ **`None` — the key absent entirely — whenever `tools` is empty.** OpenAI rejects the field
    /// outright without tools ("only allowed when 'tools' are specified"), which is exactly the shape
    /// of W6's summarisation request; a `false` there would be a 400 in the one place a 400 stalls
    /// the run rather than costing a turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
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
        // Reasoning counts here even though it never enters the history: the endpoint billed for it,
        // and this is the bill. On a reasoning model it is most of the completion — leaving it out
        // would report a run costing four times what the gauge showed.
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
    /// What the model thought on its way to `content`, for the endpoints that stream it separately.
    ///
    /// ⚠️ **It is shown and recorded, and never sent back.** Reasoning is billed as completion
    /// tokens once; putting it into the next request's history would pay for it again on every turn
    /// after that, and this model spends three quarters of a trivial turn's output on it. Nothing
    /// downstream builds a `Message` out of this field — see [`Message::assistant`], which takes
    /// `content` alone.
    pub reasoning: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Option<Usage>,
    pub finish_reason: Option<String>,
}

// ── The streaming parser ─────────────────────────────────────────────────────────────────────────

/// One piece of a completion as it arrives, tagged with which channel it came in on.
///
/// ⚠️ **Two channels, not one string.** A reasoning model streams its thinking and its answer
/// separately, and they have to stay separate all the way to the UI: the thinking is shown in a
/// block that collapses when it ends, and — unlike the answer — it is never sent back to the
/// endpoint. Concatenating them at the parser is what makes both of those impossible later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fragment<'a> {
    /// Assistant prose: the reply itself.
    Content(&'a str),
    /// The model's own thinking, from `reasoning_content` / `reasoning`.
    Reasoning(&'a str),
}

impl<'a> Fragment<'a> {
    pub fn text(self) -> &'a str {
        match self {
            Fragment::Content(text) | Fragment::Reasoning(text) => text,
        }
    }
}

/// Consume an SSE body to the end of the completion.
///
/// `on_delta` is called with each fragment as it arrives — this is what makes the browser show the
/// model thinking rather than a spinner. `cancelled` is checked **on every line**, which is one of
/// §7.3's two cancel points: returning `Err(LlmError::Cancelled)` drops the reader, which aborts the
/// HTTP request and stops the endpoint billing for a completion that is already answering a dead
/// question.
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

/// Whether an `io::Error` from the body reader is the deadline expiring rather than the connection
/// breaking, so it can become an [`LlmError::Timeout`] rather than a retryable transport fault.
///
/// ⚠️ **Both a kind and a string check, and the string is not belt-and-braces.** A read deadline
/// reaches us as `ErrorKind::TimedOut` on most paths, but ureq wraps its own `Error::Timeout` in an
/// `io::Error` on the streaming-body path, where the kind is whatever the wrapper chose — and
/// getting this wrong silently restores the old behaviour rather than failing a test.
fn is_timeout(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::TimedOut
        || format!("{error}").to_ascii_lowercase().contains("timeout")
}

/// The state machine [`read_stream`] drives, separated from the reader so a test can feed it lines
/// split wherever it likes.
#[derive(Debug, Default)]
pub struct StreamAccumulator {
    content: String,
    reasoning: String,
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
    pub fn push_line(
        &mut self,
        line: &str,
        on_delta: &mut dyn FnMut(Fragment<'_>),
    ) -> Result<bool, LlmError> {
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
            // Thinking first: a chunk carries one or the other, and on the endpoints that ever send
            // both together the reasoning is what led to the prose beside it.
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
            reasoning: self.reasoning,
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
    /// A reasoning model's thinking, which arrives on a channel of its own rather than in `content`.
    ///
    /// ⚠️ **The field has two spellings in the wild and neither is in OpenAI's own schema.** LM
    /// Studio, vLLM and DeepSeek send `reasoning_content`; OpenRouter sends `reasoning`. An endpoint
    /// that sends neither is the ordinary case and leaves this `None`.
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
    /// ⚠️ A rendered map is `detail: "high"` and up to forty-five times a screenshot's price. The
    /// flat 85 that was charged for every image is what [`Usage::estimate`] feeds
    /// `Accounting::occupancy`, and therefore what decides when the history is compacted — so
    /// under-pricing a map does not cost money, it stops a genuinely full context from ever being
    /// summarised on an endpoint that reports no `usage` of its own.
    #[test]
    fn a_high_detail_map_is_not_priced_like_a_screenshot() {
        use super::{image_tokens, ImageDetail, IMAGE_TOKENS};
        // The Game Boy screen at `screenshot::SCALE`, which is what `low` is for.
        assert_eq!(image_tokens(ImageDetail::Low, 480, 432), IMAGE_TOKENS);

        // Pallet Town, Celadon and Route 17 — the median map, a big city, and the long thin route
        // that is the worst case, because a narrow strip is scaled *up* until its short side is 768.
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

    /// A reasoning model streams its thinking on a channel of its own, and the two must not be
    /// concatenated: `content` is what goes back into the history, `reasoning` is what the page shows
    /// and then collapses. Before this the field was simply an unknown key and serde dropped it — the
    /// model looked silent for the whole turn and the reply was empty prose plus a tool call.
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

        // ⚠️ It is billed as completion tokens even though it never enters the history, so the
        // estimator has to count it — this model spends most of a turn's output on it.
        let with = Usage::estimate(&[], &completion);
        let without = Usage::estimate(&[], &Completion { reasoning: String::new(), ..completion });
        assert!(with.completion_tokens > without.completion_tokens, "{with:?} vs {without:?}");
    }

    /// ⚠️ The field has two spellings and neither is OpenAI's: LM Studio, vLLM and DeepSeek send
    /// `reasoning_content`, OpenRouter sends `reasoning`. A stream carrying the other one must not
    /// silently lose the thought.
    #[test]
    fn the_other_spelling_of_the_reasoning_field_is_read_too() {
        let completion = drain(&[
            r#"data: {"choices":[{"delta":{"reasoning":"Thinking about it."}}]}"#,
            "data: [DONE]",
        ]);
        assert_eq!(completion.reasoning, "Thinking about it.");
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
            parallel_tool_calls: Some(true),
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
        // An ordinary message's content is a bare string, not a one-element array. The multi-part
        // form is legal everywhere in principle and rejected in several places in practice, so it is
        // used only where it is needed — see `an_image_rides_on_a_user_message`.
        assert_eq!(json["messages"][0]["content"], "be brief");
    }

    /// ⚠️ **W5's screenshot trap.** The picture cannot go on the `tool` message that answered the
    /// call — OpenAI allows only text parts there and several compatible endpoints reject an image
    /// outright — so it follows as a `user` message. This pins the shape that goes on the wire and,
    /// with it, that a tool result stays a plain string.
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

        // …and it round-trips, because a history that has been through a compaction (W6) is rebuilt
        // from these types rather than kept as JSON.
        assert_eq!(serde_json::from_value::<Message>(json).expect("deserialises"), message);

        assert_eq!(message.text(), Some("look at this"), "the caption is still readable as prose");
        assert!(message.has_image());
        assert!(!Message::user("no picture here").has_image());
    }

    /// ⚠️ A base64 payload is four thousand characters and about eighty tokens. Estimating it as text
    /// would overstate a screenshot by fifty times and trip the context trim on a history that is
    /// nowhere near full.
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
    }
}
