use serde::Serialize;

/// A board column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Column {
    Ideas,
    Running,
    NeedsYou,
    Waiting,
    Paused,
    Done,
    Archive,
}

impl Column {
    /// Every column, in board order (the archive last).
    pub const ALL: [Column; 7] = [
        Column::Ideas,
        Column::Running,
        Column::NeedsYou,
        Column::Waiting,
        Column::Paused,
        Column::Done,
        Column::Archive,
    ];

    /// The id used in the API and the database.
    pub fn as_str(&self) -> &'static str {
        match self {
            Column::Ideas => "ideas",
            Column::Running => "running",
            Column::NeedsYou => "needs_you",
            Column::Paused => "paused",
            Column::Waiting => "waiting",
            Column::Done => "done",
            Column::Archive => "archive",
        }
    }

    pub fn parse(s: &str) -> Option<Column> {
        match s.trim().to_lowercase().replace('-', "_").as_str() {
            "ideas" | "idea" => Some(Column::Ideas),
            "running" => Some(Column::Running),
            "needs_you" | "needsyou" => Some(Column::NeedsYou),
            "paused" => Some(Column::Paused),
            "waiting" | "waiting_on_others" => Some(Column::Waiting),
            "done" => Some(Column::Done),
            "archive" | "archived" => Some(Column::Archive),
            _ => None,
        }
    }
}

/// The status a summary gives a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiStatus {
    NeedsYou,
    WaitingOnOthers,
    Done,
    Paused,
}

impl AiStatus {
    pub fn parse(s: &str) -> Option<AiStatus> {
        match s {
            "needs_you" => Some(AiStatus::NeedsYou),
            "waiting_on_others" => Some(AiStatus::WaitingOnOthers),
            "done" => Some(AiStatus::Done),
            "paused" => Some(AiStatus::Paused),
            _ => None,
        }
    }
}

/// One session, as stored.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Session {
    pub id: String,
    /// Id of the agent that ran the session (`claude`); empty counts as the default agent.
    pub agent: String,
    pub project_dir: Option<String>,
    pub cwd: Option<String>,
    pub repo_root: Option<String>,
    pub title: Option<String>,
    pub title_source: Option<String>,
    pub first_prompt: Option<String>,
    pub started_at: Option<i64>,
    pub last_activity_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub kind: Option<String>,
    pub parent_session_id: Option<String>,
    pub idea_id: Option<i64>,
    pub transcript_path: Option<String>,
    pub transcript_state: Option<String>,
    pub archive_path: Option<String>,
    pub git_branch: Option<String>,
    pub prompt_count: i64,
    pub reply_count: i64,
    pub last_user_text: Option<String>,
    pub last_assistant_text: Option<String>,
    pub recap: Option<String>,
    pub recap_at: Option<i64>,
    pub live_status: Option<String>,
    pub live_updated_at: Option<i64>,
    pub waiting_since: Option<i64>,
    /// What a waiting session asks of you ("answer: …", "allow Bash: …"), from the hooks.
    pub waiting_note: Option<String>,
    /// The wait is a prompt (permission, question dialog, plan) that stops the agent mid-task.
    pub waiting_prompt: bool,
    /// Why the live status says `waiting` ("permission prompt", "input needed").
    pub live_wait: Option<String>,
    /// The web link of the session while its remote control is on.
    pub remote_url: Option<String>,
    /// Background tasks and loops the transcript shows.
    pub signals: crate::activity::Signals,
    /// Loops the agent's live state shows (a Codex goal).
    pub live_loops: Vec<crate::activity::LoopInfo>,
    pub column_manual: Option<String>,
    pub column_manual_at: Option<i64>,
    pub ai_status: Option<String>,
    pub state_line: Option<String>,
    pub next_step: Option<String>,
    pub summary_at: Option<i64>,
    pub summary_input_hash: Option<String>,
    pub archived: bool,
    /// Manual override: Some(true) = `eye hide`, Some(false) = `eye unhide`, None = the rule decides.
    pub hidden: Option<bool>,
    /// When you asked to check on this session (Unix ms); see `remind`.
    pub remind_at: Option<i64>,
    /// What to check on, in your words.
    pub remind_note: Option<String>,
    /// When you were told it is due, so the notice goes out once.
    pub remind_notified_at: Option<i64>,
    /// Token use from the transcripts; None until the transcript was read.
    pub tokens: Option<Tokens>,
}

impl Session {
    /// Open, not working, and a hook said it waits on you (the same test that puts it in Needs you).
    fn waiting(&self) -> bool {
        self.waiting_since.is_some() && self.live_status.as_deref().is_some_and(|l| l != "busy")
    }

    /// What the session asks of you right now.
    pub fn waiting_for(&self) -> Option<&str> {
        self.waiting_note.as_deref().filter(|_| self.waiting())
    }

    /// What the live status says it waits for ("Permission prompt"), when it is an ask.
    pub fn live_ask(&self) -> Option<String> {
        crate::activity::live_ask(self).map(sentence)
    }

    /// The agent is stopped mid-task until you answer a prompt in the session. A finished reply that
    /// ends in a question waits on you too, but is not blocked.
    pub fn blocked(&self) -> bool {
        self.waiting_prompt && self.waiting()
    }

    /// The next step to show: what a waiting session asks of you, else the written summary's.
    pub fn next_line(&self) -> Option<String> {
        match (self.waiting_for(), self.live_ask()) {
            (Some(w), _) => Some(format!("You: {w}")),
            (None, Some(ask)) => Some(format!("You: {ask}")),
            (None, None) => self.next_step.clone(),
        }
    }
}

/// The text with its first letter in capitals.
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// A cheap change stamp: the size and the modification time (Unix ms) of something eye reads. When
/// it is the same as at the last read, the scanner leaves the thing alone without opening it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub size: i64,
    pub mtime: i64,
}

impl Stamp {
    /// The stamp of a file, from metadata already at hand.
    pub fn of_metadata(md: &std::fs::Metadata) -> Stamp {
        Stamp {
            size: md.len() as i64,
            mtime: crate::util::mtime_ms(md),
        }
    }

    /// The stamp of a file, None when it cannot be read.
    pub fn of_file(path: &std::path::Path) -> Option<Stamp> {
        std::fs::metadata(path)
            .ok()
            .map(|md| Stamp::of_metadata(&md))
    }
}

/// Token counts of a session. `subagents_total` is the sum of all four counts over its
/// sub-agent transcripts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct Tokens {
    pub input: i64,
    pub output: i64,
    pub cache_write: i64,
    pub cache_read: i64,
    pub subagents_total: i64,
}

impl Tokens {
    /// Input, output and cache tokens of the main transcript.
    pub fn main_total(&self) -> i64 {
        self.input + self.output + self.cache_write + self.cache_read
    }
    /// Main transcript plus sub-agents.
    pub fn total(&self) -> i64 {
        self.main_total() + self.subagents_total
    }
    /// Add another count to this one (the sub-agent total stays).
    pub fn add(&mut self, o: &Tokens) {
        self.input += o.input;
        self.output += o.output;
        self.cache_write += o.cache_write;
        self.cache_read += o.cache_read;
    }
    /// The API form, with the totals.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "input": self.input, "output": self.output, "cache_write": self.cache_write,
            "cache_read": self.cache_read, "total": self.total(),
            "subagents_total": self.subagents_total,
        })
    }
}

/// A piece of work inside a session: a child, an output or a topic.
#[derive(Debug, Clone, Serialize)]
pub struct Stream {
    pub id: i64,
    pub session_id: String,
    pub kind: String,
    pub key: String,
    pub title: Option<String>,
    pub status: Option<String>,
    pub detail: Option<String>,
    pub link: Option<String>,
    pub sort: Option<i64>,
    pub updated_at: Option<i64>,
}

/// Something a session produced: a pull request, an artifact, a Jira key, a commit, a folder.
#[derive(Debug, Clone, Serialize)]
pub struct Output {
    pub id: i64,
    pub session_id: String,
    pub kind: String,
    pub value: String,
    pub url: Option<String>,
    pub first_seen_at: Option<i64>,
    /// Artifact title (or its fallback) from the stream row; None when only the value is known.
    pub title: Option<String>,
}

/// A parked idea.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Idea {
    pub id: i64,
    /// The agent the idea starts in; None is the default agent.
    pub agent: Option<String>,
    pub title: String,
    /// Yours; never sent to the agent.
    pub notes: Option<String>,
    /// The first message of the session the idea starts; None opens the agent with nothing sent.
    pub prompt: Option<String>,
    pub folder: Option<String>,
    pub created_at: Option<i64>,
    pub from_session_id: Option<String>,
    pub session_id: Option<String>,
    pub sort: Option<i64>,
    pub archived: bool,
    /// When you asked to check on this idea (Unix ms); see `remind`.
    pub remind_at: Option<i64>,
    pub remind_note: Option<String>,
    pub remind_notified_at: Option<i64>,
}
