//! Runs in its own process: the global tracing subscriber is installed once.

use std::time::Duration;
use yi_core::domain::LogLevel;
use yi_core::logging;
use yi_core::services::Services;

fn logged(dir: &std::path::Path) -> String {
    logging::tail(dir, 512 * 1024, 1000).unwrap().join("\n")
}

#[tokio::test]
async fn save_settings_changes_log_level() {
    let home = tempfile::tempdir().unwrap();
    let log_dir = home.path().join("logs");
    let svc = Services::for_test(home.path());

    tracing::debug!("marker-before-debug-level");
    svc.save_settings("127.0.0.1".into(), 12000, LogLevel::Debug)
        .await
        .unwrap();
    tracing::debug!("marker-after-debug-level");

    // Dropping the services drops the logging guard, which flushes the writer.
    drop(svc);
    let mut text = logged(&log_dir);
    for _ in 0..50 {
        if text.contains("marker-after-debug-level") {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
        text = logged(&log_dir);
    }
    assert!(text.contains("marker-after-debug-level"), "{text}");
    assert!(!text.contains("marker-before-debug-level"), "{text}");
}
