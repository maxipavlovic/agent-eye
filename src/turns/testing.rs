//! Helpers the four adapters' turn tests share.

use super::{TurnEvent, TurnKind};
use serde_json::Value;

/// The kind and size of each event.
pub fn kinds(events: &[TurnEvent]) -> Vec<(TurnKind, i64)> {
    events.iter().map(|e| (e.kind, e.n)).collect()
}

/// The times of the `End` events.
pub fn end_times(events: &[TurnEvent]) -> Vec<i64> {
    events
        .iter()
        .filter(|e| e.kind == TurnKind::End)
        .map(|e| e.ts)
        .collect()
}

/// A carry as the adapter's transcript reader stores it: the open turn under `turn`.
pub fn wrapped(carry: &Option<Value>) -> Option<Value> {
    Some(serde_json::json!({ "turn": carry }))
}

/// Read `text` in two goes, split after its first `at` lines, as the scanner does when a file grows
/// between two reads. `read` turns some lines into events and the carry, given the carry of the
/// read before (None for the first). Returns the events of each read.
pub fn read_in_two(
    text: &str,
    at: usize,
    read: impl Fn(Option<&Value>, &str) -> (Vec<TurnEvent>, Option<Value>),
) -> (Vec<TurnEvent>, Vec<TurnEvent>) {
    let lines: Vec<&str> = text.lines().collect();
    let (head, tail) = lines.split_at(at);
    let (first, carry) = read(None, &head.join("\n"));
    let (second, _) = read(carry.as_ref(), &tail.join("\n"));
    (first, second)
}

/// A read that stops at any line, and goes on from there, reports what one read of the whole text
/// does: the same events in the same order, whether the text is read in two goes at any of its
/// line boundaries or one line at a time. This is the rule a growing transcript needs: no turn
/// closed twice, none lost, no compaction counted again.
pub fn assert_every_split_matches_whole(
    text: &str,
    read: impl Fn(Option<&Value>, &str) -> (Vec<TurnEvent>, Option<Value>),
) {
    let lines: Vec<&str> = text.lines().collect();
    let (whole, _) = read(None, text);
    for at in 1..lines.len() {
        let (mut a, b) = read_in_two(text, at, &read);
        a.extend(b);
        assert_eq!(a, whole, "read in two goes, split after line {at}");
    }
    let mut each = vec![];
    let mut carry: Option<Value> = None;
    for line in &lines {
        let (events, next) = read(carry.as_ref(), line);
        each.extend(events);
        carry = next;
    }
    assert_eq!(each, whole, "read one line at a time");
}
