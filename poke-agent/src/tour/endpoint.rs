//! A [`ChatEndpoint`] a [`Brain`] answers in this process, with no server, and the SSE body each
//! [`Reply`] streams as. The tests' mock server sends the same body over HTTP, so both are parsed by
//! [`read_stream`] into the same completion.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::llm::LlmError;
use crate::llm::client::ChatEndpoint;
use crate::llm::protocol::{ChatRequest, Completion, Fragment, describe_error_body, read_stream, reset_at_ms};
use crate::published::now_ms;
use crate::tour::turn::{Brain, Call, Fault, Reply, TurnRequest};

/// The endpoint, holding its brain.
pub struct BrainEndpoint {
    brain: Mutex<Box<dyn Brain>>,
    seen: AtomicUsize,
}

impl BrainEndpoint {
    pub fn new(brain: Box<dyn Brain>) -> Self {
        Self { brain: Mutex::new(brain), seen: AtomicUsize::new(0) }
    }

    /// How many requests the endpoint has answered.
    pub fn requests_served(&self) -> usize {
        self.seen.load(Ordering::SeqCst)
    }
}

impl ChatEndpoint for BrainEndpoint {
    fn stream_completion(
        &self,
        request: &ChatRequest,
        on_delta: &mut dyn FnMut(Fragment<'_>),
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Completion, LlmError> {
        let wire = serde_json::to_value(request)
            .map_err(|e| LlmError::Protocol(format!("could not encode the request: {e}")))?;
        let turn = TurnRequest::from_wire(&wire, self.seen.fetch_add(1, Ordering::SeqCst));
        let reply = self.brain.lock().expect("not poisoned").respond(&turn);
        if let Some(body) = sse_body(&reply) {
            return read_stream(body.as_bytes(), on_delta, cancelled);
        }
        let Reply::Fault(fault) = reply else { unreachable!("every other reply is a stream") };
        let body = |message: String| describe_error_body(&error_body(&message));
        Err(match fault {
            Fault::Http { status, message } => LlmError::Http { status, message: body(message) },
            Fault::RateLimited { retry_after, message } => {
                let after = retry_after.map(|after| after.as_secs().to_string());
                LlmError::RateLimited { resets_at_ms: reset_at_ms(after.as_deref(), None, now_ms()), message: body(message) }
            }
            _ => LlmError::Timeout("the brain's endpoint was not answered".to_string()),
        })
    }
}

/// The JSON body a refused request carries.
pub fn error_body(message: &str) -> String {
    serde_json::json!({ "error": { "message": message } }).to_string()
}

/// The SSE body `reply` streams as, or `None` for a fault that is not a stream: a refusal, or a
/// body that stops arriving.
pub fn sse_body(reply: &Reply) -> Option<String> {
    match reply {
        Reply::Calls(calls) => Some(sse_calls(calls)),
        Reply::Content(text) => Some(sse_content(text)),
        Reply::Fault(Fault::MalformedToolArgs) => Some(sse_raw_call("choose_action", "{not json")),
        // Ends part-way through a `data:` line, as `read_stream` sees a socket closed mid-frame.
        Reply::Fault(Fault::TruncatedStream) =>
            Some("data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"hel".to_string()),
        Reply::Fault(Fault::EmptyChoice) => Some(format!(
            "{}{}",
            frame(serde_json::json!({ "choices": [{ "delta": {}, "finish_reason": "stop" }] })),
            "data: [DONE]\n\n",
        )),
        Reply::Fault(Fault::Http { .. } | Fault::RateLimited { .. } | Fault::Timeout) => None,
    }
}

pub fn frame(value: serde_json::Value) -> String {
    format!("data: {value}\n\n")
}

/// One completion carrying prose and no tool call.
fn sse_content(text: &str) -> String {
    let mut out = String::new();
    out.push_str(&frame(serde_json::json!({
        "choices": [{ "delta": { "role": "assistant", "content": text } }]
    })));
    out.push_str(&frame(serde_json::json!({
        "choices": [{ "delta": {}, "finish_reason": "stop" }]
    })));
    out.push_str(&usage_frame());
    out.push_str("data: [DONE]\n\n");
    out
}

/// One call whose arguments are sent verbatim, so a fault can send something that is not JSON.
fn sse_raw_call(name: &str, arguments: &str) -> String {
    let mut out = String::new();
    out.push_str(&frame(serde_json::json!({
        "choices": [{ "delta": { "tool_calls": [{
            "index": 0, "id": "call_mock_0", "type": "function",
            "function": { "name": name, "arguments": arguments },
        }] } }]
    })));
    out.push_str(&frame(serde_json::json!({
        "choices": [{ "delta": {}, "finish_reason": "tool_calls" }]
    })));
    out.push_str(&usage_frame());
    out.push_str("data: [DONE]\n\n");
    out
}

fn usage_frame() -> String {
    frame(serde_json::json!({
        "choices": [], "usage": { "prompt_tokens": 1200, "completion_tokens": 40, "total_tokens": 1240 }
    }))
}

/// One completion as an OpenAI-compatible stream, arguments chopped into three-character fragments
/// and interleaved across calls as a parallel tool call is.
fn sse_calls(calls: &[Call]) -> String {
    let rendered: Vec<(String, String)> = calls
        .iter()
        .map(|call| {
            let mut arguments = call.arguments.clone();
            if let Some(object) = arguments.as_object_mut() {
                object
                    .entry("summary")
                    .or_insert_with(|| serde_json::json!("what the brain is doing"));
            }
            (call.name.clone(), serde_json::to_string(&arguments).expect("valid JSON"))
        })
        .collect();

    let mut out = String::new();
    out.push_str(&frame(serde_json::json!({
        "choices": [{ "delta": { "role": "assistant", "content": "Let me look at where I am." } }]
    })));
    for (index, (name, _)) in rendered.iter().enumerate() {
        out.push_str(&frame(serde_json::json!({
            "choices": [{ "delta": { "tool_calls": [{
                "index": index, "id": format!("call_mock_{index}"), "type": "function",
                "function": { "name": name, "arguments": "" },
            }] } }]
        })));
    }
    let fragments: Vec<Vec<&str>> =
        rendered.iter().map(|(_, arguments)| chunks(arguments, 3)).collect();
    for step in 0..fragments.iter().map(Vec::len).max().unwrap_or(0) {
        for (index, call) in fragments.iter().enumerate() {
            let Some(fragment) = call.get(step) else { continue };
            out.push_str(&frame(serde_json::json!({
                "choices": [{ "delta": { "tool_calls": [{
                    "index": index, "function": { "arguments": fragment },
                }] } }]
            })));
        }
    }
    out.push_str(&frame(serde_json::json!({
        "choices": [{ "delta": {}, "finish_reason": "tool_calls" }]
    })));
    out.push_str(&usage_frame());
    out.push_str("data: [DONE]\n\n");
    out
}

fn chunks(text: &str, size: usize) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let split = rest.char_indices().nth(size).map_or(rest.len(), |(i, _)| i);
        let (head, tail) = rest.split_at(split);
        out.push(head);
        rest = tail;
    }
    out
}


#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::llm::protocol::{FunctionSpec, Message, StreamOptions, ToolSpec};

    fn request() -> ChatRequest {
        ChatRequest {
            model: "in-process".to_string(),
            messages: vec![
                Message::system("the rules"),
                Message::user("Location: PalletTown at (5, 6)\n- `PalletTown:Sign` — read the sign"),
            ],
            tools: vec![ToolSpec {
                kind: "function",
                function: FunctionSpec { name: "choose_action", description: String::new(), parameters: serde_json::json!({}) },
            }],
            parallel_tool_calls: None,
            max_tokens: None,
            reasoning_effort: None,
            temperature: None,
            stream: true,
            stream_options: StreamOptions { include_usage: true },
        }
    }

    /// The brain is shown what the mock server shows it, and its answer arrives as that stream does.
    #[test]
    fn a_brain_answers_in_process_as_it_does_over_the_wire() {
        let shown = Arc::new(Mutex::new(None));
        let saw = Arc::clone(&shown);
        let endpoint = BrainEndpoint::new(Box::new(move |turn: &TurnRequest| {
            *saw.lock().unwrap() = Some((turn.location(), turn.menu_ids(), turn.has_tool("choose_action"), turn.seen));
            Reply::call("choose_action", serde_json::json!({ "id": "PalletTown:Sign" }))
        }));
        let mut prose = String::new();
        let completion = endpoint
            .stream_completion(&request(), &mut |fragment| if let Fragment::Content(text) = fragment { prose.push_str(text) }, &|| false)
            .expect("a completion");

        assert_eq!(*shown.lock().unwrap(),
                   Some((Some("PalletTown".to_string()), vec!["PalletTown:Sign".to_string()], true, 0)));
        assert_eq!(prose, "Let me look at where I am.");
        assert_eq!(completion.tool_calls.len(), 1);
        assert_eq!(completion.tool_calls[0].function.name, "choose_action");
        let arguments: serde_json::Value = serde_json::from_str(&completion.tool_calls[0].function.arguments).expect("JSON");
        assert_eq!(arguments, serde_json::json!({ "id": "PalletTown:Sign", "summary": "what the brain is doing" }));
        assert!(completion.usage.is_some());
        assert_eq!(endpoint.requests_served(), 1);
    }

    /// A refusal is the error the HTTP client makes of the same status and body.
    #[test]
    fn a_refusal_in_process_is_the_error_a_server_would_make_it() {
        let endpoint = BrainEndpoint::new(Box::new(|_: &TurnRequest| {
            Reply::Fault(Fault::Http { status: 402, message: "no credit".to_string() })
        }));
        match endpoint.stream_completion(&request(), &mut |_| {}, &|| false) {
            Err(LlmError::Http { status: 402, message }) => assert!(message.contains("no credit"), "{message}"),
            other => panic!("not a 402: {other:?}"),
        }
    }
}
