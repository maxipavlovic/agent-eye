//! The turn events an adapter collects while it reads a transcript, and the open turn it carries
//! to its next read.
//!
//! A turn ends with the last thing the agent did before the next prompt, which is only known once
//! that prompt arrives. The read that sees the prompt emits the `End`, so a read that stops in the
//! middle of a turn, or at the end of the file, keeps the open turn (time of its last reply, words
//! so far) in the carry instead of emitting anything; the next read picks it up and the turn is
//! closed once, at its last reply. So a turn that nothing followed yet has no `End`. An agent that
//! says itself when a turn ends closes it at that line (`close_at`), but only a turn a prompt began:
//! a task the user did not type (a compaction, a review) is no turn of the user's.

use super::{TurnEvent, TurnKind};
use serde_json::{json, Value};

/// The agent's turn that has not been closed yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Open {
    /// Time of its last reply.
    ts: i64,
    /// Words of reply text so far.
    words: i64,
}

/// Collects the turn events of the lines it is shown, in file order.
#[derive(Debug, Default)]
pub struct Stream {
    events: Vec<TurnEvent>,
    open: Option<Open>,
    /// A prompt came and the turn it began has not been closed or given up.
    prompted: bool,
}

impl Stream {
    /// Start from the open turn an earlier read left in its carry (None: nothing is open).
    pub fn resume(carry: Option<&Value>) -> Stream {
        let turn = carry.and_then(|c| c.get("turn"));
        let open = turn.and_then(|t| {
            Some(Open {
                ts: t.get("ts")?.as_i64()?,
                words: t.get("words")?.as_i64()?,
            })
        });
        // a carry from before the flag existed: an open turn is one a prompt began
        let prompted = turn
            .and_then(|t| t.get("prompted"))
            .and_then(Value::as_bool)
            .unwrap_or(open.is_some());
        Stream {
            events: vec![],
            open,
            prompted,
        }
    }

    pub fn push(&mut self, ts: i64, kind: TurnKind, n: i64) {
        self.prompted |= kind == TurnKind::Prompt;
        self.events.push(TurnEvent::new(ts, kind, n));
    }

    /// The agent gave up the turn without finishing it (the user interrupted it): it ends with its
    /// last reply when the next prompt arrives, and a task that finishes meanwhile is not its end.
    pub fn abandon(&mut self) {
        self.prompted = false;
    }

    /// The last event pushed.
    pub fn last(&self) -> Option<&TurnEvent> {
        self.events.last()
    }

    /// The agent did something at `ts`, with `words` of reply text: the turn is open until the next
    /// prompt, and its `End` is the time of the last of these.
    pub fn reply(&mut self, ts: i64, words: i64) {
        match &mut self.open {
            Some(o) => {
                o.ts = ts;
                o.words += words;
            }
            None => self.open = Some(Open { ts, words }),
        }
    }

    /// Emit the `End` of the open turn, once (a prompt arrived).
    pub fn close(&mut self) {
        if let Some(o) = self.open.take() {
            self.push(o.ts, TurnKind::End, o.words);
        }
        self.prompted = false;
    }

    /// Emit the `End` of the turn at `ts`, because the agent said it was done (a turn with no
    /// reply ends with no words). A task no prompt began, or a turn that was ended already, adds
    /// nothing.
    pub fn close_at(&mut self, ts: i64) {
        if !self.prompted {
            return;
        }
        let words = self.open.take().map_or(0, |o| o.words);
        self.push(ts, TurnKind::End, words);
        self.prompted = false;
    }

    /// The events, and the turn to carry to the next read.
    pub fn finish(self) -> (Vec<TurnEvent>, Option<Value>) {
        let carry = match (self.open, self.prompted) {
            (Some(o), p) => Some(json!({"ts": o.ts, "words": o.words, "prompted": p})),
            (None, true) => Some(json!({"prompted": true})),
            (None, false) => None,
        };
        (self.events, carry)
    }
}
