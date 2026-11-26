//! The turn events an adapter collects while it reads a transcript, and the open turn it carries
//! to its next read.
//!
//! A turn ends with the last thing the agent did before the next prompt, which is only known once
//! that prompt arrives. The read that sees the prompt emits the `End`, so a read that stops in the
//! middle of a turn keeps the open turn (time of its last reply, words so far) in the carry instead
//! of emitting anything; the next read picks it up and the turn is closed once. A read of the whole
//! transcript also closes the last turn at its end, as nothing more is known about it, and marks it
//! closed in the carry, so a continuation of that same turn is not closed a second time. An agent
//! that says itself when a turn ends closes it at that line (`close_at`).

use super::{TurnEvent, TurnKind};
use serde_json::{json, Value};

/// The agent's turn that has not been closed yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Open {
    /// Time of its last reply.
    ts: i64,
    /// Words of reply text so far.
    words: i64,
    /// Its `End` has been emitted (a whole read stopped inside it).
    closed: bool,
}

/// Collects the turn events of the lines it is shown, in file order.
#[derive(Debug, Default)]
pub struct Stream {
    events: Vec<TurnEvent>,
    open: Option<Open>,
}

impl Stream {
    /// Start from the open turn an earlier read left in its carry (None: nothing is open).
    pub fn resume(carry: Option<&Value>) -> Stream {
        let open = carry.and_then(|c| c.get("turn")).and_then(|t| {
            Some(Open {
                ts: t.get("ts")?.as_i64()?,
                words: t.get("words")?.as_i64()?,
                closed: t.get("closed")?.as_bool()?,
            })
        });
        Stream {
            events: vec![],
            open,
        }
    }

    pub fn push(&mut self, ts: i64, kind: TurnKind, n: i64) {
        self.events.push(TurnEvent::new(ts, kind, n));
    }

    /// The last event pushed.
    pub fn last(&self) -> Option<&TurnEvent> {
        self.events.last()
    }

    /// The agent did something at `ts`, with `words` of reply text: the turn is open until the next
    /// prompt, and its `End` is the time of the last of these.
    pub fn reply(&mut self, ts: i64, words: i64) {
        match &mut self.open {
            Some(o) if o.closed => {}
            Some(o) => {
                o.ts = ts;
                o.words += words;
            }
            None => {
                self.open = Some(Open {
                    ts,
                    words,
                    closed: false,
                });
            }
        }
    }

    /// Emit the `End` of the open turn, once (a prompt arrived).
    pub fn close(&mut self) {
        if let Some(o) = self.open.take().filter(|o| !o.closed) {
            self.push(o.ts, TurnKind::End, o.words);
        }
    }

    /// Emit the `End` of the turn at `ts`, because the agent said it was done (a turn with no
    /// reply ends with no words).
    pub fn close_at(&mut self, ts: i64) {
        let words = self
            .open
            .take()
            .filter(|o| !o.closed)
            .map_or(0, |o| o.words);
        self.push(ts, TurnKind::End, words);
    }

    /// The events, and the turn to carry to the next read. A `whole` read closes the last turn.
    pub fn finish(mut self, whole: bool) -> (Vec<TurnEvent>, Option<Value>) {
        if whole {
            if let Some(o) = self.open.as_mut().filter(|o| !o.closed) {
                let end = TurnEvent::new(o.ts, TurnKind::End, o.words);
                o.closed = true;
                self.events.push(end);
            }
        }
        let carry = self
            .open
            .map(|o| json!({"ts": o.ts, "words": o.words, "closed": o.closed}));
        (self.events, carry)
    }
}
