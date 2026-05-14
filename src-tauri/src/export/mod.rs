//! Export / parse module — bug-reports.md, todo.md, done.md formats.
//!
//! Split (v0.31.0) into focused sub-modules:
//! - `util`         — shared pipe-respecting parser + escape/unescape helpers
//! - `bugs`         — v2 (8-field) bug-reports generate + parse
//! - `bugs_legacy`  — pre-v2 "# Bug List:" import path
//! - `todo_done`    — F-021 todo.md + done.md parsers
//!
//! Public surface is re-exported flat here so `crate::export::foo` keeps working.

mod bugs;
mod bugs_legacy;
mod todo_done;
mod util;

pub use bugs::{generate_bug_reports, parse_bug_reports};
pub use bugs_legacy::parse_markdown_legacy;
pub use todo_done::{parse_done_entries_in_period, parse_done_tasks, parse_todo_tasks};
// Kept exported for parity with pre-split surface (was `pub fn` in flat export.rs).
#[allow(unused_imports)]
pub use util::split_pipe_respecting_escape;
