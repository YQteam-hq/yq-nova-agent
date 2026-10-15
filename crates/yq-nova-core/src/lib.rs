// Recent clippy releases report `clippy::double_must_use` for every
// `#[async_trait]` trait in this crate: the macro stamps `#[must_use]` onto
// the boxed future it returns, which clippy now considers redundant because
// `Pin<Box<dyn Future>>` is already `#[must_use]`. The attribute is injected
// by the macro rather than written here, so silence the lint crate-wide.
#![allow(clippy::double_must_use)]

pub mod background;
pub mod config;
pub mod embedding;
pub mod error;
pub mod graph;
pub mod logging;
pub mod memory;
pub mod storage;

pub use config::Config;
pub use error::{ErrorCode, NovaError, NovaResult};
pub use uuid::Uuid;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn git_sha() -> &'static str {
    option_env!("YQ_NOVA_GIT_SHA").unwrap_or("dev")
}
