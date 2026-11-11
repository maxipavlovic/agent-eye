//! The picker's state: the rows, the query, the selection and the conversation cache.

use crate::agents::Agents;
use crate::config::UserCfg;
use crate::conversation::{self, Message};
use crate::db::Db;
use crate::folder::{self, FolderState};
use crate::paths::Paths;
use crate::pick::{self, Detail, Index, Kind, Row};
use anyhow::Result;
use ratatui::widgets::ListState;
use std::collections::HashMap;
use std::path::PathBuf;

/// Messages kept per session in the conversation view (the newest ones).
pub(super) const TALK_KEEP: usize = 200;

/// A conversation as loaded: the number of messages in the transcript and the newest ones.
pub(super) type Talk = Result<(usize, Vec<Message>), String>;

/// The picker's state.
pub struct App {
    pub db: Db,
    pub idx: Index,
    pub query: String,
    pub all: bool,
    pub show_preview: bool,
    pub results: Vec<usize>,
    pub sel: usize,
    pub list_state: ListState,
    pub detail: Detail,
    pub(super) detail_id: Option<String>,
    pub msg: Option<String>,
    /// Row id of a live session whose second Enter is awaited.
    pub(super) confirm: Option<String>,
    pub now: i64,
    pub color: bool,
    pub page: usize,
    /// Last search time, shown in the corner when EYE_PICK_TIMING is set.
    pub last_ms: f64,
    /// Where transcripts are read from for the conversation view (None: the view stays empty).
    pub paths: Option<Paths>,
    /// Who "you" is in next lines.
    pub user: UserCfg,
    /// The enabled agents: their names label the preview, and they read the conversations.
    pub agents: Agents,
    /// The preview pane shows the conversation instead of the summary (Ctrl-T).
    pub talk: bool,
    /// Lines the conversation is scrolled up from its end.
    pub(super) talk_scroll: usize,
    /// Conversations read so far, per row id.
    pub(super) talk_cache: HashMap<String, Talk>,
    /// The folder eye may create idea folders inside (None: it creates none).
    pub folder_root: Option<PathBuf>,
    /// The one-line input for a reminder, while it is open (Ctrl-R).
    pub(super) remind: Option<super::remind::Input>,
}

impl App {
    /// A picker over `idx`, searched for `query`.
    pub fn new(db: Db, idx: Index, query: &str, color: bool, now: i64) -> App {
        let mut a = App {
            db,
            idx,
            query: query.to_string(),
            all: false,
            show_preview: true,
            results: vec![],
            sel: 0,
            list_state: ListState::default(),
            detail: Detail::default(),
            detail_id: None,
            msg: None,
            confirm: None,
            now,
            color,
            page: 10,
            last_ms: 0.0,
            paths: None,
            user: UserCfg::default(),
            agents: Agents::default(),
            talk: false,
            talk_scroll: 0,
            talk_cache: HashMap::new(),
            folder_root: None,
            remind: None,
        };
        a.research();
        a
    }

    /// Run the query again.
    pub fn research(&mut self) {
        let t = std::time::Instant::now();
        self.results = pick::search(&self.db, &self.idx, &self.query, self.all, self.now);
        self.last_ms = t.elapsed().as_secs_f64() * 1000.0;
        self.sel = 0;
        self.msg = None;
        self.confirm = None;
        self.refresh_detail();
    }

    /// What the folder of an idea row is; None for a session or an idea without a folder.
    pub(super) fn idea_folder(&self, r: &Row) -> Option<FolderState> {
        let dir = r.idea.as_ref()?.folder.as_deref()?;
        Some(folder::classify(self.folder_root.as_deref(), dir))
    }

    /// The selected row.
    pub fn current(&self) -> Option<&Row> {
        self.results.get(self.sel).map(|&i| &self.idx.rows[i])
    }

    /// Hide the selected session (or show it again when it is hidden), keeping the place in the list.
    pub(super) fn toggle_hidden(&mut self) {
        let Some(&i) = self.results.get(self.sel) else {
            return;
        };
        let r = &self.idx.rows[i];
        if r.kind != Kind::Session {
            self.msg = Some("only sessions can be hidden".into());
            return;
        }
        let (id, hide, title) = (r.id.clone(), !r.hidden, r.title.clone());
        if let Err(e) = self.db.set_hidden(&id, Some(hide)) {
            self.msg = Some(format!("could not save: {e:#}"));
            return;
        }
        self.idx.rows[i].hidden = hide;
        let sel = self.sel;
        self.research();
        self.sel = sel.min(self.results.len().saturating_sub(1));
        self.refresh_detail();
        self.msg = Some(if hide {
            format!("hidden: {title}  (Ctrl-A shows hidden sessions, Ctrl-X there shows it again)")
        } else {
            format!("shown again: {title}")
        });
    }

    pub(super) fn refresh_detail(&mut self) {
        let want = self.current().map(|r| r.id.clone());
        if want != self.detail_id {
            self.detail = match self.current() {
                Some(r) => pick::detail(&self.db, r),
                None => Detail::default(),
            };
            self.detail_id = want;
            self.talk_scroll = 0;
        }
        self.list_state.select(if self.results.is_empty() {
            None
        } else {
            Some(self.sel)
        });
    }

    /// Read the conversation of the selected row once; called when the pane is drawn, so keys that
    /// are already queued are applied first.
    pub(super) fn ensure_talk(&mut self) {
        let Some(r) = self.current() else {
            return;
        };
        if self.talk_cache.contains_key(&r.id) {
            return;
        }
        let loaded: Talk = match (&r.session, &self.paths) {
            _ if r.kind != Kind::Session => Err("an idea has no conversation yet".into()),
            (Some(s), Some(p)) => match self.agents.of_session(s) {
                Some(agent) => conversation::read(s, p, agent.as_ref())
                    .map(|mut all| {
                        let n = all.len();
                        all.drain(..n.saturating_sub(TALK_KEEP));
                        (n, all)
                    })
                    .map_err(|e| format!("{e:#}")),
                None => Err("the agent of this session is not enabled".into()),
            },
            _ => Err("no transcript to read".into()),
        };
        let id = r.id.clone();
        self.talk_cache.insert(id, loaded);
    }

    pub(super) fn move_by(&mut self, d: isize) {
        if self.results.is_empty() {
            return;
        }
        let n = self.results.len() as isize;
        self.sel = (self.sel as isize + d).clamp(0, n - 1) as usize;
        self.msg = None;
        self.confirm = None;
        self.refresh_detail();
    }
}
