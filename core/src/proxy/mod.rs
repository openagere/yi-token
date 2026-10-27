//! Local proxy: lifecycle (server), error responses and the request pipeline.

mod errors;
pub mod headers;
pub mod monitor;
pub mod pipeline;
pub mod request_log;
pub mod resolve;
pub mod router;
mod server;
pub mod upstream;

pub use monitor::Monitor;
pub use server::{ProxyHandle, ProxyPhase, ProxyStatus};
