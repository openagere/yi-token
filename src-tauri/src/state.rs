//! Application state shared with every command.

use std::sync::Arc;
use yi_core::services::Services;

pub struct AppState(pub Arc<Services>);
