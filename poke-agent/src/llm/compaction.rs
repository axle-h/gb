//! Keeping the history inside the context window.

use crate::llm::protocol::{Content, Message, Role};

/// The complete turns a compaction keeps as they were, pictures and all.
pub const KEEP_TURNS: usize = 2;

/// What an evicted picture leaves behind, so the turn does not read as if the model imagined it.
pub const EVICTED: &str = "[image removed to save context]";

/// The rule every summary restates. It names no menu, because
/// [`prompt::contract`](crate::llm::prompt::contract) closes every turn with that.
pub const CONTRACT_REMINDER: &str = "\
Whatever else has changed, the rule for every turn is the same as it was: end each turn with \
`choose_action` and an id from the menu at the bottom of the turn request. `read_file`, \
`write_file` and `edit_file` do not end the turn, and a reply with no tool call at all does \
nothing in the game.";

/// What the summary request asks the model to write about its own history. Its first line is how
/// a scripted endpoint tells the request apart.
pub const SUMMARY_INSTRUCTION: &str = "\
Your context is nearly full, so this conversation is about to be replaced by a summary of it.

Write that summary now, as a note to your future self. Cover, in prose and in this order:

1. Where you are and what is happening right now.
2. What you have achieved so far — badges, key items, the party and roughly how strong it is.
3. What you were trying to do next, and why.
4. What you have learned about this world that you would otherwise have to rediscover: routes that \
were blocked, people who wanted something, places you have not been yet, things that did not work.

Be specific — names, places, levels — and do not pad it. Nothing else from this conversation is \
kept except `plan.md`, which survives as it is, so anything in neither is forgotten. Reply with the \
summary itself and nothing else; do not call a tool.";

/// Added to [`SUMMARY_INSTRUCTION`] when the oldest turns were left out of the request.
const START_MISSING: &str = "\
The start of this conversation has been left out of this request, because a summary of the whole \
of it could not be had. Summarise what is here, and do not guess at what came before it.";

/// What a compaction that could get no summary puts where the summary would have gone.
const DROPPED: &str = "\
The earlier conversation was dropped without a summary, because none could be written. `plan.md` \
survived it.";

/// The same, after an earlier summary that is kept: what went is only what came after it.
pub const DROPPED_SINCE: &str = "\
The conversation after that summary was dropped without a new one, because none could be written. \
`plan.md` survived it.";

/// A summary request's answer to `choose_action`.
pub const NOT_AN_ACTION: &str = "\
Not taken: this request asks for the summary, not an action, and nothing happens in the game until \
it is written. Reply with the summary as text.";

/// A summary request's answer to a write of anything but `plan.md`.
pub const PLAN_ONLY: &str = "\
Nothing was written: while the summary is being written only `plan.md` can be changed. Reply with \
the summary as text.";

/// What a summary request is told when it has one request left.
pub const OUT_OF_READS: &str = "You have used every read. Reply with the summary now, as text, and call no tool.";

/// What a summary request ends with.
pub fn instruction(start_missing: bool) -> Message {
    Message::user(match start_missing {
        false => SUMMARY_INSTRUCTION.to_string(),
        true => SUMMARY_INSTRUCTION.replacen("\n\n", &format!("\n\n{START_MISSING}\n\n"), 1),
    })
}

/// Replace every image but the `keep` newest with [`EVICTED`], keeping each caption.
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

/// What [`summary_message`] opens with, and therefore how [`is_summary`] recognises one.
pub const SUMMARY_HEADING: &str = "## The story so far";

/// The summary as a user message, under [`SUMMARY_HEADING`] and ending in [`CONTRACT_REMINDER`].
pub fn summary_message(summary: &str) -> Message {
    Message::user(format!("{SUMMARY_HEADING}\n\n{}\n\n{CONTRACT_REMINDER}", summary.trim()))
}

/// [`summary_message`]'s place, held when no summary could be had.
pub fn dropped_message() -> Message {
    summary_message(DROPPED)
}

/// What a compaction that could get no summary opens with: an earlier summary and a note that
/// what followed it went, or, with none to keep, [`dropped_message`].
pub fn dropped_opening(messages: &[Message]) -> Vec<Message> {
    match earlier_summary(messages).filter(|summary| !is_dropped(summary)) {
        Some(summary) => vec![summary.clone(), Message::user(DROPPED_SINCE)],
        None => vec![dropped_message()],
    }
}

/// An earlier summary as a compaction that asks for none keeps it: with the note after it, if a
/// failed compaction left one.
pub fn kept_opening(messages: &[Message]) -> Option<Vec<Message>> {
    let summary = earlier_summary(messages)?;
    let system = usize::from(messages.first().is_some_and(|first| first.role == Role::System));
    let note = messages.get(system + 1).filter(|message| message.text() == Some(DROPPED_SINCE));
    Some(std::iter::once(summary).chain(note).cloned().collect())
}

/// [`dropped_message`], which holds a summary's place and says nothing a summary would.
fn is_dropped(message: &Message) -> bool {
    message.text().is_some_and(|text| text.contains(DROPPED))
}

pub fn is_summary(message: &Message) -> bool {
    message.role == Role::User && message.text().is_some_and(|text| text.starts_with(SUMMARY_HEADING))
}

/// A `user` message that is not a picture: where a provider's turn begins.
pub fn is_turn_start(message: &Message) -> bool {
    message.role == Role::User && !message.has_image() && !is_evicted_image(message)
}

fn is_evicted_image(message: &Message) -> bool {
    message.text().is_some_and(|text| text.ends_with(EVICTED))
}

/// The situation that opens one of the game's turns; nudges and notes inside a turn are not one.
fn opens_turn(message: &Message) -> bool {
    message.role == Role::User && message.text().is_some_and(|text| text.starts_with(crate::llm::prompt::DECISION))
}

/// How many leading messages a compaction never removes: the system prompt and an earlier summary.
fn head_len(messages: &[Message]) -> usize {
    let system = usize::from(messages.first().is_some_and(|first| first.role == Role::System));
    system + usize::from(messages.get(system).is_some_and(is_summary))
}

/// Where each turn after the head starts.
fn turn_starts(messages: &[Message]) -> Vec<usize> {
    let head = head_len(messages);
    (head..messages.len()).filter(|&index| opens_turn(&messages[index])).collect()
}

/// The summary an earlier compaction left, or the note held in its place.
pub fn earlier_summary(messages: &[Message]) -> Option<&Message> {
    let system = usize::from(messages.first().is_some_and(|first| first.role == Role::System));
    messages.get(system).filter(|message| is_summary(message))
}

/// How many turns there are since the head.
pub fn turns_after_head(messages: &[Message]) -> usize {
    turn_starts(messages).len()
}

/// Whether there is anything past the head to summarise or drop.
pub fn has_body(messages: &[Message]) -> bool {
    messages.len() > head_len(messages)
}

/// The history with the oldest half of its turns removed, cut at turn boundaries. The head and the
/// last [`KEEP_TURNS`] turns are never removed. The flag says whether anything was.
pub fn without_oldest_half(messages: &[Message]) -> (Vec<Message>, bool) {
    let head = head_len(messages);
    let starts = turn_starts(messages);
    let removable = starts.len().saturating_sub(KEEP_TURNS);
    let removed = removable.div_ceil(2);
    if removed == 0 {
        return (messages.to_vec(), false);
    }
    let mut kept = messages[..head].to_vec();
    kept.extend_from_slice(&messages[starts[removed]..]);
    (kept, true)
}

/// Keep the system prompt, put `opening` after it and then the last `keep` turns, and drop the
/// rest. Returns how many messages left; `opening` is not counted against them.
pub fn replace_with(messages: &mut Vec<Message>, opening: &[Message], keep: usize) -> usize {
    let before = messages.len();
    let system = messages.first().filter(|first| first.role == Role::System).cloned();
    let starts = turn_starts(messages);
    let from = match (keep, starts.len().checked_sub(keep)) {
        (0, _) => messages.len(),
        (_, Some(index)) => starts[index],
        (_, None) => starts.first().copied().unwrap_or(messages.len()),
    };

    let mut kept = Vec::with_capacity(messages.len() - from + 1 + opening.len());
    kept.extend(system);
    kept.extend_from_slice(opening);
    kept.extend_from_slice(&messages[from..]);
    *messages = kept;

    before.saturating_sub(messages.len().saturating_sub(opening.len()))
}

/// Some endpoints take only a plain string on some roles, so eviction must not leave parts.
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

    /// One turn as the worker builds it: situation, read call and result, picture, action.
    fn turn(n: usize, with_picture: bool) -> Vec<Message> {
        let mut messages = vec![
            Message::user(format!("## Decision: what to do next in the overworld\n\nturn {n}")),
            Message::assistant(String::new(), vec![call(&format!("r{n}"), "read_file")]),
            Message::tool_result(format!("r{n}"), "map.png"),
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

    fn says(messages: &[Message], needle: &str) -> bool {
        messages.iter().any(|m| m.text().is_some_and(|t| t.contains(needle)))
    }

    /// A `tool` message whose call was dropped is a 400.
    fn well_formed(messages: &[Message]) {
        let mut outstanding: Vec<&str> = Vec::new();
        for message in messages {
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
    }

    #[test]
    fn eviction_replaces_pictures_with_their_captions_and_costs_nothing_else() {
        let mut messages = history(5, true);
        let before = messages.len();
        let heavy: u64 = messages.iter().map(Message::approximate_tokens).sum();

        assert_eq!(evict_images(&mut messages, 0), 5);
        assert_eq!(messages.len(), before, "eviction replaces messages, it does not remove them");
        assert!(!messages.iter().any(Message::has_image));

        // The caption stays, so the model can see that it did look.
        let evicted = messages.iter().find(|m| m.text().is_some_and(|t| t.ends_with(EVICTED))).expect("evicted");
        assert!(evicted.text().unwrap().contains("Screenshot of the Game Boy screen (frame 0)"));
        assert!(!is_multi_part(evicted), "an evicted picture is a plain string again");

        let after: u64 = messages.iter().map(Message::approximate_tokens).sum();
        assert!(heavy - after >= 2 * crate::llm::protocol::IMAGE_TOKENS, "{heavy} → {after}");
        assert_eq!(evict_images(&mut messages, 0), 0, "idempotent");
    }

    /// An evicted picture is a plain-string `user` message mid-turn: a turn boundary's shape.
    #[test]
    fn an_evicted_picture_is_never_a_cut_point() {
        let mut messages = history(3, true);
        evict_images(&mut messages, 0);
        for (index, message) in messages.iter().enumerate() {
            if message.text().is_some_and(|text| text.ends_with(EVICTED)) {
                assert!(!is_turn_start(message), "message {index} became a legal cut point");
            }
        }
        assert_eq!(turn_starts(&messages).len(), 3, "one per turn, no more");
    }

    #[test]
    fn a_summary_keeps_the_last_two_turns_untouched_pictures_and_all() {
        let mut messages = history(6, true);
        let tail: Vec<Message> = messages[messages.len() - 2 * turn(0, true).len()..].to_vec();
        let before = messages.len();

        let dropped = replace_with(&mut messages, &[summary_message("I am in Pallet Town with a Squirtle.")], KEEP_TURNS);

        assert_eq!(messages[0].role, Role::System, "the system prompt is never compacted");
        assert!(messages[0].text().unwrap().contains("Pokémon Red"));
        assert!(is_summary(&messages[1]));
        assert!(messages[1].text().unwrap().contains("I am in Pallet Town with a Squirtle."));
        assert_eq!(&messages[2..], &tail[..], "the last two turns, exactly as they were");
        assert_eq!(messages.iter().filter(|m| m.has_image()).count(), 2, "their pictures with them");
        assert_eq!(dropped, before - 1 - tail.len(), "every message that left, and not the summary");
        well_formed(&messages);
        assert!(!says(&messages, "turn 3"));
        assert!(says(&messages, "turn 4") && says(&messages, "turn 5"));
    }

    #[test]
    fn a_compaction_can_keep_one_turn_or_none() {
        let mut one = history(4, false);
        replace_with(&mut one, &[dropped_message()], 1);
        assert!(says(&one, "turn 3") && !says(&one, "turn 2"));
        well_formed(&one);

        let mut none = history(4, false);
        replace_with(&mut none, &[dropped_message()], 0);
        assert_eq!(none.len(), 2, "the system prompt and the note");
        assert!(none[1].text().unwrap().contains("dropped without a summary"));
        assert!(is_summary(&none[1]), "the note holds the summary's place");

        // Fewer turns than asked for keeps them all.
        let mut short = history(1, false);
        let was = short.len();
        assert_eq!(replace_with(&mut short, &[dropped_message()], KEEP_TURNS), 0);
        assert_eq!(short.len(), was + 1);
    }

    /// A failed compaction keeps a summary an earlier one wrote, but not a note held in its place.
    #[test]
    fn a_compaction_without_a_summary_keeps_an_earlier_one_and_says_what_went() {
        let mut messages = history(4, false);
        replace_with(&mut messages, &[summary_message("an earlier story")], 4);
        let opening = dropped_opening(&messages);
        replace_with(&mut messages, &opening, 1);
        assert!(messages[1].text().unwrap().contains("an earlier story"));
        assert_eq!(messages[2].text(), Some(DROPPED_SINCE));
        assert!(says(&messages, "turn 3") && !says(&messages, "turn 2"));
        well_formed(&messages);
        assert_eq!(kept_opening(&messages).expect("a summary").len(), 2, "the note travels with it");

        let mut messages = history(4, false);
        replace_with(&mut messages, &[dropped_message()], 4);
        assert_eq!(dropped_opening(&messages), vec![dropped_message()], "a note is not a summary to keep");
        assert_eq!(dropped_opening(&history(4, false)), vec![dropped_message()]);
    }

    /// A nudge or a resume note inside the history is not a turn of its own.
    #[test]
    fn only_a_situation_opens_a_turn() {
        let mut messages = history(3, false);
        messages.insert(6, Message::user(crate::llm::prompt::RESUMED_NOTE));
        messages.push(Message::user("That reply contained no tool call, so nothing happened in the game."));
        messages.push(Message::assistant("sorry".into(), vec![]));
        assert_eq!(turn_starts(&messages).len(), 3);

        replace_with(&mut messages, &[summary_message("story")], KEEP_TURNS);
        assert!(says(&messages, "turn 1") && says(&messages, "turn 2") && !says(&messages, "turn 0"));
    }

    #[test]
    fn halving_removes_the_oldest_turns_and_never_the_head_or_the_last_two() {
        let mut messages = history(6, false);
        replace_with(&mut messages, &[summary_message("an earlier story")], 6);
        assert_eq!(turn_starts(&messages).len(), 6);

        let (halved, removed) = without_oldest_half(&messages);
        assert!(removed);
        assert_eq!(halved[0].role, Role::System);
        assert!(halved[1].text().unwrap().contains("an earlier story"), "an earlier summary stays");
        assert_eq!(turn_starts(&halved).len(), 4, "two of the four removable turns went");
        assert!(!says(&halved, "turn 1") && says(&halved, "turn 2"));
        well_formed(&halved);

        // Nothing to remove when only the kept turns are left.
        let (same, removed) = without_oldest_half(&history(2, false));
        assert!(!removed);
        assert_eq!(same.len(), history(2, false).len());
    }

    #[test]
    fn the_instruction_says_when_the_start_is_missing_and_keeps_its_first_line() {
        let whole = instruction(false);
        let halved = instruction(true);
        assert_eq!(whole.text(), Some(SUMMARY_INSTRUCTION));
        let first_line = SUMMARY_INSTRUCTION.lines().next().unwrap();
        assert!(halved.text().unwrap().starts_with(first_line));
        assert!(halved.text().unwrap().contains("left out of this request"));
        assert!(SUMMARY_INSTRUCTION.contains("`plan.md`"), "the model is told its plan survives");
    }

    #[test]
    fn summary_restates_turn_contract() {
        let message = summary_message("I am somewhere doing something.");
        let text = message.text().expect("the summary is prose");

        assert!(text.ends_with(CONTRACT_REMINDER), "the reminder is the last thing the model reads");
        assert!(CONTRACT_REMINDER.contains("`choose_action` and an id from the menu"));
        // The wording has to stay recognisably the same rule as the one every turn ends with.
        let real = crate::llm::prompt::contract(crate::llm::tools::DecisionKind::Overworld);
        for phrase in ["`choose_action`", "do not end the turn"] {
            assert!(real.contains(phrase) && CONTRACT_REMINDER.contains(phrase), "`{phrase}` has drifted");
        }
        for tool in crate::llm::tools::non_terminal_names() {
            assert!(CONTRACT_REMINDER.contains(&format!("`{tool}`")), "`{tool}` is missing");
        }
        assert!(crate::llm::prompt::SYSTEM_PROMPT.contains("do not end the turn"), "and so does the system prompt's copy");
    }

    /// A history with no system message, or no turns, must not panic.
    #[test]
    fn degenerate_histories_survive_a_compaction() {
        let mut messages: Vec<Message> = Vec::new();
        assert!(!has_body(&messages));
        assert_eq!(replace_with(&mut messages, &[summary_message("nothing happened")], KEEP_TURNS), 0);
        assert_eq!(messages.len(), 1, "the summary is all there is");
        assert!(!has_body(&messages), "a lone summary is a head and nothing more");

        let mut messages = vec![Message::user("just the one")];
        assert!(has_body(&messages));
        replace_with(&mut messages, &[summary_message("story")], KEEP_TURNS);
        assert!(messages[0].text().unwrap().starts_with(SUMMARY_HEADING));
        assert_eq!(messages.len(), 1, "a message that opens no turn is summarised away");

        assert!(!without_oldest_half(&[]).1);
        assert_eq!(evict_images(&mut [], 0), 0);
    }
}
