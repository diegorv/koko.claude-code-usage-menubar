use std::sync::Mutex;
use tauri::AppHandle;

#[derive(Default)]
pub struct UsagePoller {
    handle: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

impl UsagePoller {
    pub fn new() -> Self {
        Self::default()
    }

    /// Aborts any previous poll task and starts a new one. The first tick runs
    /// immediately, then every `interval_secs`. Each tick drives the refresh
    /// cycle in `commands::do_refresh_cycle`.
    pub fn restart(&self, app: AppHandle, interval_secs: u64) {
        if let Ok(mut handle) = self.handle.lock() {
            if let Some(h) = handle.take() {
                h.abort();
            }

            let jh = tauri::async_runtime::spawn(async move {
                crate::commands::do_refresh_cycle(&app).await;

                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;
                    crate::commands::do_refresh_cycle(&app).await;
                }
            });

            *handle = Some(jh);
        }
    }
}
