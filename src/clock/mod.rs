//! The steering clock: when the user steers an agent worse, and whether a bad stretch was the user
//! or the model. Turn events (`crate::turns`) say when prompts, replies, interrupts, tool calls and
//! thinking happened; `detect` asks a model, once per prompt, which prompts were pushback on
//! something the agent did wrong (the corrections); `bands`, `verdict` and `report` count them by
//! hour of the day and by time since a break and put the user's side next to the model's; `permute`
//! tests the picked hours (and the model's thinking) against random dealing, and `judging` says who
//! may judge corrections and why not. `idle`
//! keeps the idle spans that make breaks exact. See `docs/clock.md`.

pub mod bands;
pub mod detect;
pub mod idle;
pub mod judging;
pub mod nightly;
pub mod pass;
pub mod permute;
pub mod report;
pub mod signals;
pub mod verdict;
