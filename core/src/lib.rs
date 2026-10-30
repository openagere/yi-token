//! yi-token core library.
//!
//! Further module declarations are added by later tasks.

pub mod backup;
pub mod clock;
pub mod catalog;
pub mod domain;
pub mod error;
pub mod lock;
pub mod logging;
pub mod paths;
pub mod protocol;
pub mod proxy;
pub mod services;
pub mod storage;
pub mod terminal;
pub mod trajectory;

pub use error::{CoreError, CoreResult, ErrorCode, UpstreamError};
pub use paths::AppPaths;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
