//! The interactive picker screen. Drawn on /dev/tty so stdout stays clean for `--print`.

mod input;
mod render;
mod state;
mod terminal;
#[cfg(test)]
mod test_support;

pub use render::draw;
pub use state::App;
pub use terminal::{have_tty, render_text, run};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Quit,
    /// Row id to resume or start.
    Pick(String),
    /// Session id to fork: resume as a new session that branches from it.
    Fork(String),
}
