//! When the user and the agent took their turns. An adapter reports a stream of turn events per
//! session (`Delta::turns`); the core stores them (`store`) for the steering clock, which asks
//! whether the user steers worse at some hours. Sessions read before eye kept turns get theirs
//! from `backfill`, which reads the transcripts again and touches nothing else.

pub mod backfill;
pub mod store;
pub mod stream;

/// What happened at one moment of a session. The wording is the same for every agent; how an agent's
/// own records map to it is in `docs/agents.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnKind {
    /// The user typed a prompt. `n` is its length in characters.
    Prompt,
    /// The agent finished a turn: the last thing it wrote before the next prompt. `n` is the
    /// number of words of reply text in that turn.
    End,
    /// The user stopped the agent while it was working.
    Interrupt,
    /// The agent made one tool call.
    Tool,
    /// A tool call failed (a tool result that reports an error, not a refusal).
    ToolError,
    /// A tool call was refused by the user or by a rule.
    Refusal,
    /// The agent thought. `n` is the agent's own measure of that thinking, which differs by agent
    /// (a length, a token count) and is only compared with the same agent's other values.
    Think,
    /// The context of the session was compacted.
    Compact,
}

impl TurnKind {
    /// The word stored in the `turn` table.
    pub fn as_str(self) -> &'static str {
        match self {
            TurnKind::Prompt => "prompt",
            TurnKind::End => "end",
            TurnKind::Interrupt => "interrupt",
            TurnKind::Tool => "tool",
            TurnKind::ToolError => "tool_error",
            TurnKind::Refusal => "refusal",
            TurnKind::Think => "think",
            TurnKind::Compact => "compact",
        }
    }
}

/// One turn event of a session: when it happened (Unix ms), what it was, and its size (`n`, which
/// each kind explains above; 0 when it has none).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnEvent {
    pub ts: i64,
    pub kind: TurnKind,
    pub n: i64,
}

impl TurnEvent {
    pub fn new(ts: i64, kind: TurnKind, n: i64) -> TurnEvent {
        TurnEvent { ts, kind, n }
    }
}
