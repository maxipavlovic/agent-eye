//! An agent whose judging run is not proven: another agent with `judging_locked_down` saying no.
//! It keeps the other agent's id, so it may still write summaries (as the agents from before 0.5
//! may), and its `ask` works, so a test that counts the calls sees one if it is asked.

use crate::agents::{
    Agent, AgentRef, Delta, Detection, HeadlessReply, HeadlessRequest, Invocation, ParseOpts,
    StartPlan, StartRequest, TailItem, Transcript,
};
use crate::conversation::Message;
use anyhow::Result;
use std::path::Path;

#[derive(Debug)]
pub struct Unproven(pub AgentRef);

impl Agent for Unproven {
    fn id(&self) -> &'static str {
        self.0.id()
    }
    fn name(&self) -> &'static str {
        self.0.name()
    }
    fn short_name(&self) -> &'static str {
        self.0.short_name()
    }
    fn detect(&self) -> Detection {
        self.0.detect()
    }
    fn transcripts(&self) -> Vec<Transcript> {
        self.0.transcripts()
    }
    fn parse_transcript(&self, t: &Transcript, opts: &ParseOpts) -> Result<Delta> {
        self.0.parse_transcript(t, opts)
    }
    fn judging_locked_down(&self) -> bool {
        false
    }
    fn owns_transcript(&self, path: &Path) -> bool {
        self.0.owns_transcript(path)
    }
    fn read_conversation(&self, path: &Path) -> Result<Vec<Message>> {
        self.0.read_conversation(path)
    }
    fn conversation_tail(&self, path: &Path) -> Result<Vec<TailItem>> {
        self.0.conversation_tail(path)
    }
    fn ask(&self, req: &HeadlessRequest) -> Result<HeadlessReply> {
        self.0.ask(req)
    }
    fn executable(&self) -> String {
        self.0.executable()
    }
    fn resume(&self, session_id: &str) -> Invocation {
        self.0.resume(session_id)
    }
    fn start(&self, req: &StartRequest) -> Result<StartPlan> {
        self.0.start(req)
    }
}
