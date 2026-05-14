// T-000097: sync.rs split into per-domain sub-modules. Each child file owns
// its free fns; this file is a pure barrel — declarations + flat re-exports
// so existing `use crate::sync::*` and `crate::sync::foo()` call-sites in
// lib.rs and db/*.rs compile unchanged.

pub mod bugs;
pub mod claude_md;
pub mod fs;
pub mod gitignore;
pub mod project_md;
pub mod requirements;
pub mod tasks;

pub use bugs::*;
pub use claude_md::*;
pub use fs::*;
pub use gitignore::*;
pub use project_md::*;
pub use requirements::*;
pub use tasks::*;
