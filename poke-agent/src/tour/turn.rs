//! What a scripted model sees of a turn and what it answers with: the request as strings, the reply
//! as tool calls, prose or a fault, and the [`Brain`] that maps one to the other.

use std::time::Duration;


// ── What a brain is allowed to see ──

/// One message, as the endpoint received it.
#[derive(Debug, Clone)]
pub struct SeenMessage {
    pub role: String,
    pub text: String,
    pub images: Vec<(String, String)>,
}

/// One tool, as the endpoint was offered it.
#[derive(Debug, Clone)]
pub struct SeenTool {
    pub name: String,
}

/// Everything a [`Brain`] is allowed to see: the request, as strings.
#[derive(Debug, Clone)]
pub struct TurnRequest {
    pub messages: Vec<SeenMessage>,
    pub tools: Vec<SeenTool>,
    /// How many requests this endpoint has answered before this one, from zero.
    pub seen: usize,
}

impl TurnRequest {
    /// The newest `user` message: the situation this turn is asked about.
    pub fn situation(&self) -> &str {
        self.messages
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .map_or("", |message| message.text.as_str())
    }

    /// The ids the situation offered, in order, parsed out of the rendered menu as a model must.
    pub fn menu_ids(&self) -> Vec<String> {
        self.situation()
            .lines()
            .filter_map(|line| line.strip_prefix("- `"))
            .filter_map(|line| line.split_once('`'))
            .map(|(id, _)| id.to_string())
            .collect()
    }

    /// The menu as `(id, description)`, parsed out of the rendered situation as a model must.
    pub fn menu_rows(&self) -> Vec<(String, String)> {
        self.situation()
            .lines()
            .filter_map(|line| line.strip_prefix("- `"))
            .filter_map(|line| line.split_once('`'))
            .map(|(id, rest)| {
                (id.to_string(), rest.trim_start_matches([' ', '—']).trim().to_string())
            })
            .collect()
    }

    /// The map the situation says the player is on; `None` on every kind but the overworld.
    pub fn location(&self) -> Option<String> {
        self.situation()
            .lines()
            .find_map(|line| line.strip_prefix("Location: "))
            .and_then(|line| line.split(" at (").next())
            .map(str::to_string)
    }

    pub fn tool_names(&self) -> Vec<&str> {
        self.tools.iter().map(|tool| tool.name.as_str()).collect()
    }

    pub fn has_tool(&self, name: &str) -> bool {
        self.tools.iter().any(|tool| tool.name == name)
    }

    pub fn is_battle(&self) -> bool {
        self.has_tool("choose_battle_action")
    }

    pub fn is_stuck(&self) -> bool {
        self.has_tool("press_buttons")
            && !self.has_tool("choose_action")
            && !self.has_tool("choose_battle_action")
    }

    /// Compaction's own request: no tools, and the last user message is the instruction.
    pub fn is_summary(&self) -> bool {
        self.tools.is_empty()
            && self.situation().starts_with(
                crate::llm::compaction::SUMMARY_INSTRUCTION.split('\n').next().unwrap_or_default(),
            )
    }

    /// Every `(data_url, detail)` the endpoint has been sent in this request, in order.
    pub fn images(&self) -> Vec<(String, String)> {
        self.messages.iter().flat_map(|message| message.images.iter().cloned()).collect()
    }

}

// ── What a brain answers with ──

/// One tool call, before it is fragmented onto the wire.
#[derive(Debug, Clone)]
pub struct Call {
    pub name: String,
    pub arguments: serde_json::Value,
}

impl Call {
    pub fn new(name: &str, arguments: serde_json::Value) -> Self {
        Self { name: name.to_string(), arguments }
    }

    /// The terminal `wait`, every brain's "I have nothing" answer.
    pub fn wait(ticks: u64) -> Self {
        Self::new("wait", serde_json::json!({ "ticks": ticks }))
    }
}

#[derive(Debug, Clone)]
pub enum Reply {
    /// Tool calls, fragmented across `data:` frames by the endpoint.
    Calls(Vec<Call>),
    /// Prose and no tool call: the nudge-then-force path in [`crate::llm::worker::Worker::decide`], and a
    /// compaction summary.
    Content(String),
    Fault(Fault),
}

impl Reply {
    pub fn call(name: &str, arguments: serde_json::Value) -> Self {
        Self::Calls(vec![Call::new(name, arguments)])
    }
}

#[derive(Debug, Clone)]
pub enum Fault {
    /// A non-2xx with a body.
    Http { status: u16, message: String },
    /// A 429.
    RateLimited { retry_after: Option<Duration>, message: String },
    /// A 200 whose body stops arriving for longer than `GB_REQUEST_TIMEOUT_SECS`.
    Timeout,
    /// Valid SSE, arguments that are not JSON.
    MalformedToolArgs,
    /// The body ends part-way through a `data:` line.
    TruncatedStream,
    /// A completion with neither content nor a tool call.
    EmptyChoice,
}

/// What decides. Handed [`TurnRequest`] and nothing else.
pub trait Brain: Send {
    fn respond(&mut self, request: &TurnRequest) -> Reply;
}

impl<F: FnMut(&TurnRequest) -> Reply + Send> Brain for F {
    fn respond(&mut self, request: &TurnRequest) -> Reply {
        self(request)
    }
}

impl TurnRequest {
    /// The request as an OpenAI-compatible body carries it, `seen` being how many came before.
    pub fn from_wire(wire: &serde_json::Value, seen: usize) -> Self {
        let messages: Vec<SeenMessage> =
            wire["messages"].as_array().expect("messages").iter().map(seen_message).collect();
        let tools: Vec<SeenTool> = wire["tools"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(|tool| SeenTool {
                name: tool["function"]["name"].as_str().unwrap_or_default().to_string(),
            })
            .collect();
        Self { messages, tools, seen }
    }
}

/// Flatten one wire message into what a brain is allowed to see.
fn seen_message(message: &serde_json::Value) -> SeenMessage {
    let role = message["role"].as_str().unwrap_or_default().to_string();
    let (mut text, mut images) = (String::new(), Vec::new());
    match &message["content"] {
        serde_json::Value::String(whole) => text.push_str(whole),
        serde_json::Value::Array(parts) => {
            for part in parts {
                match part["type"].as_str() {
                    Some("image_url") => images.push((
                        part["image_url"]["url"].as_str().unwrap_or_default().to_string(),
                        part["image_url"]["detail"].as_str().unwrap_or_default().to_string(),
                    )),
                    _ => {
                        if let Some(fragment) = part["text"].as_str() {
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(fragment);
                        }
                    }
                }
            }
        }
        _ => {}
    }
    SeenMessage { role, text, images }
}
