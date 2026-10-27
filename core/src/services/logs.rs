use super::{LogsView, Services};
use crate::error::CoreResult;
use crate::logging;
use std::sync::Arc;

const MAX_TAIL_BYTES: u64 = 512 * 1024;
const MAX_TAIL_LINES: usize = 1000;

impl Services {
    pub async fn get_logs(self: &Arc<Self>) -> CoreResult<LogsView> {
        self.run(|s| {
            let lines = logging::tail(&s.paths.log_dir, MAX_TAIL_BYTES, MAX_TAIL_LINES)?;
            Ok(LogsView { lines })
        })
        .await
    }
}
