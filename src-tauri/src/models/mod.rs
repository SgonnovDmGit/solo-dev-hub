// T-000102: split by domain. Each sub-module owns its DTOs; this file is a
// pure barrel — declarations + flat re-export so existing `use crate::models::*`
// and `use crate::models::{Foo, Bar}` call-sites compile unchanged.

pub mod bugs;
pub mod bundle;
pub mod core;
pub mod dashboard;
pub mod deploy;
pub mod graph;
pub mod stats;
pub mod sync;
pub mod tasks;
pub mod templates;
pub mod timeline;

pub use bugs::*;
pub use bundle::*;
pub use core::*;
pub use dashboard::*;
pub use deploy::*;
pub use graph::*;
pub use stats::*;
pub use sync::*;
pub use tasks::*;
pub use templates::*;
pub use timeline::*;
