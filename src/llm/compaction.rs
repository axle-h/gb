//! **W6 / §9** — keeping the history inside the context window.
//!
//! Two stages, cheapest first, both triggered at the same occupancy threshold
//! ([`worker::COMPACT_ABOVE`](crate::llm::worker)):
//!
//! 1. **Image eviction.** Every screenshot except the two most recent becomes one line of text. A
//!    run that looks at the screen often spends most of its context on pictures it has already acted
//!    on, and this is a hundred times cheaper than asking the model to summarise.
//! 2. **Summarising compaction.** If eviction was not enough, one extra completion writes the story
//!    so far, and everything except the system prompt and the last few messages is replaced by it.
//!
//! Everything here is a pure function over `Vec<Message>` so the awkward parts — which message is a
//! safe place to cut, what survives — are testable without an endpoint. The worker owns the
//! *decision* to compact and the completion that stage 2 needs; this module owns the surgery.
//!
//! ⚠️ **The one thing that must survive a compaction is the turn contract** (§7.5). The system
//! prompt is never touched, which is the first copy; [`CONTRACT_REMINDER`] is appended to the summary
//! as fixed text, which is the second. Neither is left to the model to carry over — compaction is
//! exactly where a long-running behavioural rule gets quietly dropped, and the failure it produces
//! (prose, no terminal call) stalls a run rather than erroring.

use crate::llm::config::LlmConfig;
use crate::llm::protocol::{ChatRequest, Content, Message, Role, StreamOptions};

/// How many screenshots survive stage 1. Two, because the model regularly compares "now" against
/// "the last time I looked".
pub const KEEP_IMAGES: usize = 2;

/// How many messages survive stage 2, before the cut is moved forward to a turn boundary. §9's
/// number.
pub const KEEP_MESSAGES: usize = 8;

/// What an evicted picture leaves behind. Kept short, and kept *present*: a model that is told the
/// screenshot is gone will ask for another one if it needs it, whereas a silently vanished image
/// makes the surrounding conversation read as if it hallucinated looking at the screen.
pub const EVICTED: &str = "[screenshot removed to save context]";

/// The rule that must outlive every compaction, restated verbatim inside the summary. Deliberately
/// kind-independent: the per-kind list of terminal tools is regenerated at the bottom of every turn
/// request by [`prompt::contract`](crate::llm::prompt::contract), so what the summary has to carry
/// is the *rule*, not the list.
pub const CONTRACT_REMINDER: &str = "\
Whatever else has changed, the rule for every turn is the same as it was: end each turn with \
exactly one terminal tool call, chosen from the list at the bottom of the turn request. Read tools \
do not end the turn, and a reply with no tool call at all does nothing in the game.";

/// What stage 2 asks for. Written as an instruction to the model about its own history, and asking
/// for the four things a future turn actually needs: where it is, what it has done, what it is
/// trying to do next, and what it has learned that is not visible in the current situation.
pub const SUMMARY_INSTRUCTION: &str = "\
Your context is nearly full, so this conversation is about to be replaced by a summary of it.

Write that summary now, as a note to your future self. Cover, in prose and in this order:

1. Where you are and what is happening right now.
2. What you have achieved so far — badges, key items, the party and roughly how strong it is.
3. What you were trying to do next, and why.
4. What you have learned about this world that you would otherwise have to rediscover: routes that \
were blocked, people who wanted something, places you have not been yet, things that did not work.

Be specific — names, places, levels — and do not pad it. Nothing else from this conversation is \
kept, so anything you leave out is forgotten. Reply with the summary itself and nothing else; do \
not call a tool.";

/// **Stage 1.** Replace every image except the `keep` most recent with [`EVICTED`], keeping the
/// caption that came with it so the conversation still reads in order. Returns how many were
/// evicted.
pub fn evict_images(messages: &mut [Message], keep: usize) -> usize {
    let total = messages.iter().filter(|message| message.has_image()).count();
    let mut to_evict = total.saturating_sub(keep);
    let mut evicted = 0;
    for message in messages.iter_mut() {
        if to_evict == 0 {
            break;
        }
        if !message.has_image() {
            continue;
        }
        let caption = message.text().unwrap_or_default();
        let replacement = match caption.is_empty() {
            true => EVICTED.to_string(),
            false => format!("{caption} {EVICTED}"),
        };
        message.content = Some(Content::Text(replacement));
        to_evict -= 1;
        evicted += 1;
    }
    evicted
}

/// The request that produces a summary: the whole history, plus [`SUMMARY_INSTRUCTION`].
///
/// ⚠️ **No tools, and therefore no `parallel_tool_calls`.** OpenAI rejects that field outright when
/// `tools` is absent, and sending an empty tool array to a model that has been told to end every
/// message with a tool call is asking for one here, where a tool call would be a dead turn.
pub fn summary_request(config: &LlmConfig, messages: &[Message]) -> ChatRequest {
    let mut messages = messages.to_vec();
    messages.push(Message::user(SUMMARY_INSTRUCTION));
    ChatRequest {
        model: config.model.clone(),
        messages,
        tools: Vec::new(),
        parallel_tool_calls: None,
        temperature: config.temperature,
        stream: true,
        stream_options: StreamOptions { include_usage: true },
    }
}

/// What [`summary_message`] opens with, and therefore how [`is_summary`] recognises one.
pub const SUMMARY_HEADING: &str = "## The story so far";

/// The summary as it goes into the history: a user message, headed so it is obviously not something
/// that just happened, and ending in [`CONTRACT_REMINDER`].
pub fn summary_message(summary: &str) -> Message {
    Message::user(format!("{SUMMARY_HEADING}\n\n{}\n\n{CONTRACT_REMINDER}", summary.trim()))
}

/// Whether this is a message [`summary_message`] produced.
///
/// ⚠️ **The summary is the one message a last-resort trim must not touch.** It sits at the front of
/// the history, exactly where dropping "the oldest turn" would find it — and dropping it discards
/// everything it was written to preserve, in exchange for the fifty tokens it costs.
pub fn is_summary(message: &Message) -> bool {
    message.role == Role::User && message.text().is_some_and(|text| text.starts_with(SUMMARY_HEADING))
}

/// Whether stage 2 can achieve anything at all: there must be more in the history than the system
/// prompt, a summary and the tail that would be kept.
///
/// ⚠️ Without this, a run configured with a context limit smaller than its own system prompt would
/// summarise on **every turn**, paying for a completion each time to make the history one message
/// longer.
pub fn worth_summarising(messages: &[Message], keep: usize) -> bool {
    messages.len() > keep + 2
}

/// **Stage 2.** Keep the system prompt, the summary, and the tail of the conversation; drop
/// everything in between. Returns how many messages were dropped.
///
/// ⚠️ **The tail cannot start just anywhere.** A `tool` message whose assistant message has been
/// dropped is a 400 from the endpoint, not a slightly odd history — so the cut is moved *forward*
/// from `len - keep` to the next turn boundary, which by construction begins a turn whose tool calls
/// are all still with it. Forward rather than backward because backward can swallow an arbitrarily
/// long turn, and the point of this is to bound the history.
pub fn apply_summary(messages: &mut Vec<Message>, summary: &str, keep: usize) -> usize {
    let before = messages.len();
    let system = messages.first().filter(|first| first.role == Role::System).cloned();
    let first_body = usize::from(system.is_some());

    let from = messages.len().saturating_sub(keep).max(first_body);
    let tail = messages[from..]
        .iter()
        .position(is_turn_start)
        .map(|offset| offset + from)
        // No boundary in the tail at all — everything left belongs to a turn that started further
        // back than we are willing to keep. Keeping none of it is safe; keeping part of it is not.
        .unwrap_or(messages.len());

    let mut kept = Vec::with_capacity(keep + 2);
    kept.extend(system);
    kept.push(summary_message(summary));
    kept.extend_from_slice(&messages[tail..]);
    *messages = kept;

    // The two added messages do not count as dropped: this is the number the UI shows as "before →
    // after" and it should describe the conversation, not the bookkeeping.
    before.saturating_sub(messages.len().saturating_sub(1))
}

/// Where the history may be cut: the `user` message that opens a turn.
///
/// ⚠️ **A picture is a `user` message and is not a boundary** (W5), and neither is what a picture
/// becomes once [`evict_images`] has been over it — eviction turns an image into text, and without
/// this second check a message in the *middle* of a turn would silently become a legal cut point the
/// moment stage 1 ran.
pub fn is_turn_start(message: &Message) -> bool {
    message.role == Role::User && !message.has_image() && !is_evicted_image(message)
}

fn is_evicted_image(message: &Message) -> bool {
    message.text().is_some_and(|text| text.ends_with(EVICTED))
}

/// Whether a message is one of the multi-part ones, for a test that wants to be sure eviction left a
/// plain string behind rather than a parts array with the picture taken out of it — several
/// endpoints accept only the former on some roles.
#[cfg(test)]
fn is_multi_part(message: &Message) -> bool {
    matches!(message.content, Some(Content::Parts(_)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::protocol::{FunctionCall, ToolCall};

    fn picture(n: usize) -> Message {
        Message::user_with_image(
            format!("Screenshot of the Game Boy screen (frame {n})"),
            format!("data:image/png;base64,{}", "A".repeat(4000)),
        )
    }

    fn call(id: &str, name: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            kind: "function".into(),
            function: FunctionCall { name: name.into(), arguments: "{}".into() },
        }
    }

    /// One turn as the worker builds it: the situation, an assistant message with a read call, its
    /// result, a picture, then the terminal call and its result.
    fn turn(n: usize, with_picture: bool) -> Vec<Message> {
        let mut messages = vec![
            Message::user(format!("## Decision: what to do next in the overworld\n\nturn {n}")),
            Message::assistant(String::new(), vec![call(&format!("r{n}"), "read_map")]),
            Message::tool_result(format!("r{n}"), "{\"map\": \"PalletTown\"}"),
        ];
        if with_picture {
            messages.push(picture(n));
        }
        messages.push(Message::assistant("walking north".into(), vec![call(&format!("t{n}"), "choose_action")]));
        messages.push(Message::tool_result(format!("t{n}"), "Accepted."));
        messages
    }

    fn history(turns: usize, with_pictures: bool) -> Vec<Message> {
        let mut messages = vec![Message::system(crate::llm::prompt::SYSTEM_PROMPT)];
        for n in 0..turns {
            messages.extend(turn(n, with_pictures));
        }
        messages
    }

    /// Stage 1: the two most recent pictures survive, every older one becomes a line of text, and
    /// nothing else in the history moves.
    #[test]
    fn eviction_keeps_the_two_most_recent_pictures_and_costs_nothing_else() {
        let mut messages = history(5, true);
        let before = messages.len();
        let heavy: u64 = messages.iter().map(Message::approximate_tokens).sum();

        let evicted = evict_images(&mut messages, KEEP_IMAGES);

        assert_eq!(evicted, 3, "five pictures, two kept");
        assert_eq!(messages.len(), before, "eviction replaces messages, it does not remove them");
        let survivors: Vec<usize> = messages
            .iter()
            .enumerate()
            .filter(|(_, message)| message.has_image())
            .map(|(index, _)| index)
            .collect();
        assert_eq!(survivors.len(), KEEP_IMAGES);
        assert!(survivors[0] > 3 * 6, "the survivors are the *recent* ones, not the first two: {survivors:?}");

        // The caption is still there, so the conversation reads in order and the model can see that
        // it did look, rather than being left with an unexplained gap.
        let evicted_message = messages.iter().find(|m| m.text().is_some_and(|t| t.ends_with(EVICTED)));
        let text = evicted_message.expect("something was evicted").text().unwrap();
        assert!(text.contains("Screenshot of the Game Boy screen (frame 0)"), "{text}");
        assert!(!is_multi_part(evicted_message.unwrap()), "an evicted picture is a plain string again");

        let after: u64 = messages.iter().map(Message::approximate_tokens).sum();
        assert!(
            heavy - after >= 2 * crate::llm::protocol::IMAGE_TOKENS,
            "eviction must actually save: {heavy} → {after}",
        );

        // Idempotent: running it again finds only the two it is meant to keep.
        assert_eq!(evict_images(&mut messages, KEEP_IMAGES), 0);
    }

    /// ⚠️ The trap eviction creates for stage 2: an evicted picture is a `user` message with a plain
    /// string, which is exactly the shape of a turn boundary — and it sits in the *middle* of a turn.
    #[test]
    fn an_evicted_picture_is_never_a_cut_point() {
        let mut messages = history(3, true);
        evict_images(&mut messages, 0);
        for (index, message) in messages.iter().enumerate() {
            if message.text().is_some_and(|text| text.ends_with(EVICTED)) {
                assert!(!is_turn_start(message), "message {index} became a legal cut point");
            }
        }
        // …and the real boundaries still are ones.
        assert!(messages.iter().filter(|m| is_turn_start(m)).count() == 3, "one per turn, no more");
    }

    /// Stage 2: the system prompt and the tail survive, the summary lands between them, and what is
    /// kept is a well-formed history — every `tool` message with the assistant message that asked
    /// for it.
    #[test]
    fn a_summary_replaces_the_middle_and_leaves_a_well_formed_history() {
        let mut messages = history(6, false);
        let before = messages.len();

        let dropped = apply_summary(&mut messages, "I am in Pallet Town with a Squirtle.", KEEP_MESSAGES);

        assert!(dropped > 0 && dropped < before, "{dropped} of {before}");
        assert_eq!(messages[0].role, Role::System, "the system prompt is never compacted");
        assert!(messages[0].text().unwrap().contains("Pokémon Red"));
        assert_eq!(messages[1].role, Role::User);
        assert!(messages[1].text().unwrap().contains("I am in Pallet Town with a Squirtle."));
        assert!(messages.len() <= 2 + KEEP_MESSAGES, "the tail is bounded: {}", messages.len());

        // Well-formedness is the whole point: a `tool` message whose call was dropped is a 400.
        let mut outstanding: Vec<&str> = Vec::new();
        for message in &messages {
            for call in &message.tool_calls {
                outstanding.push(&call.id);
            }
            if let Some(id) = message.tool_call_id.as_deref() {
                let position = outstanding.iter().position(|open| *open == id);
                assert!(position.is_some(), "result for `{id}` survived without its call");
                outstanding.remove(position.unwrap());
            }
        }
        assert!(outstanding.is_empty(), "a call survived with no result: {outstanding:?}");

        // Nothing older than the tail is still there.
        assert!(!messages.iter().any(|m| m.text().is_some_and(|t| t.contains("turn 0"))));
        assert!(messages.iter().any(|m| m.text().is_some_and(|t| t.contains("turn 5"))), "the newest turn is kept");
    }

    /// The cut moves forward to a boundary, so a tail that would have started mid-turn starts at the
    /// turn instead — even when that means keeping fewer messages than asked for.
    #[test]
    fn the_tail_starts_at_a_turn_and_never_in_the_middle_of_one() {
        for keep in 1..14 {
            let mut messages = history(4, true);
            evict_images(&mut messages, 1);
            apply_summary(&mut messages, "story", keep);

            assert_eq!(messages[0].role, Role::System);
            assert!(messages[1].text().unwrap().starts_with("## The story so far"));
            match messages.get(2) {
                None => {}
                Some(third) => assert!(
                    is_turn_start(third),
                    "keep={keep}: the tail starts with {:?} / {:?}",
                    third.role,
                    third.text().map(|t| t.chars().take(40).collect::<String>()),
                ),
            }
        }
    }

    /// The two guards the worker's last resort depends on: it can recognise a summary, and it knows
    /// when summarising would make the history *longer* rather than shorter.
    #[test]
    fn a_summary_is_recognisable_and_a_short_history_is_not_worth_one() {
        let mut messages = history(6, false);
        assert!(worth_summarising(&messages, KEEP_MESSAGES));
        assert!(!messages.iter().any(is_summary));

        apply_summary(&mut messages, "story", KEEP_MESSAGES);
        assert_eq!(messages.iter().filter(|m| is_summary(m)).count(), 1);
        assert!(is_summary(&messages[1]), "and it is where the trim would look first");
        assert!(
            !worth_summarising(&messages, KEEP_MESSAGES),
            "summarising a just-summarised history only adds a message",
        );

        // ⚠️ The pathological configuration: a context limit smaller than the system prompt. Every
        // turn is over the threshold and none of them can be helped by a summary.
        assert!(!worth_summarising(&history(1, false), KEEP_MESSAGES));
    }

    /// §9's ⚠️. The contract is restated as fixed text, not left to the model to carry over.
    #[test]
    fn summary_restates_turn_contract() {
        let message = summary_message("I am somewhere doing something.");
        let text = message.text().expect("the summary is prose");

        assert!(text.ends_with(CONTRACT_REMINDER), "the reminder is the last thing the model reads");
        assert!(CONTRACT_REMINDER.contains("exactly one terminal tool call"));
        assert!(CONTRACT_REMINDER.contains("do not end the turn"));
        // The wording has to stay recognisably the same rule as the one every turn ends with.
        let real = crate::llm::prompt::contract(crate::llm::tools::DecisionKind::Overworld);
        for phrase in ["exactly one", "do not end the turn"] {
            assert!(real.contains(phrase) && CONTRACT_REMINDER.contains(phrase), "`{phrase}` has drifted");
        }

        // …and it survives being put through the compaction it exists for.
        let mut messages = history(6, false);
        apply_summary(&mut messages, "I am somewhere doing something.", KEEP_MESSAGES);
        assert!(messages.iter().any(|m| m.text().is_some_and(|t| t.contains(CONTRACT_REMINDER))));
        assert!(messages[0].text().unwrap().contains("do not end the turn"), "and so does the system prompt's copy");
    }

    /// ⚠️ The summarisation request must not offer tools, and must not carry `parallel_tool_calls`
    /// at all — OpenAI rejects that field when `tools` is absent, which would turn a compaction into
    /// a 400 and a stalled run.
    #[test]
    fn a_summary_request_asks_for_prose_and_offers_no_tools() {
        let config = LlmConfig {
            base_url: "http://x".into(),
            api_key: "k".into(),
            model: "m".into(),
            context_limit: 1000,
            temperature: 0.4,
            max_tool_steps: 4,
            stuck_timeout: None,
        };
        let messages = history(2, false);
        let request = summary_request(&config, &messages);

        assert_eq!(request.messages.len(), messages.len() + 1);
        assert_eq!(request.messages.last().unwrap().text(), Some(SUMMARY_INSTRUCTION));
        assert_eq!(request.temperature, 0.4);
        assert!(request.tools.is_empty());

        let json = serde_json::to_value(&request).expect("serialises");
        assert!(json.get("tools").is_none(), "an empty tools array is not the same as no tools");
        assert!(json.get("parallel_tool_calls").is_none(), "illegal without tools");
    }

    /// A history with no system message, and one shorter than the tail it is asked to keep: neither
    /// is a thing the worker can produce, and neither may panic.
    #[test]
    fn degenerate_histories_survive_a_compaction() {
        let mut messages: Vec<Message> = Vec::new();
        assert_eq!(apply_summary(&mut messages, "nothing happened", KEEP_MESSAGES), 0);
        assert_eq!(messages.len(), 1, "the summary is all there is");

        let mut messages = vec![Message::user("just the one")];
        apply_summary(&mut messages, "story", KEEP_MESSAGES);
        assert!(messages[0].text().unwrap().starts_with("## The story so far"));
        assert_eq!(messages.len(), 2, "no system prompt to keep, and the tail is the one message");

        assert_eq!(evict_images(&mut [], KEEP_IMAGES), 0);
    }
}
