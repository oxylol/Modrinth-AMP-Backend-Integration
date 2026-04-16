//! Per-instance background polling task.
//!
//! One `PollerHandle` is spawned per active AMP instance when a frontend
//! subscriber registers. It:
//!   1. Calls `Core/GetUpdates` at ~2Hz.
//!   2. Emits `amp://console/{server_id}` for each console entry.
//!   3. Emits `amp://status/{server_id}` on every poll (cheap + keeps UI alive).
//!   4. Auto-handles session expiry via `AmpClient::call` (which re-logs on 401).
//!
//! When the last subscriber unsubscribes, the handle is dropped and the task
//! exits on the next iteration.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::{Emitter, Runtime};
use tokio::task::JoinHandle;
use tokio::time;

use super::client::AmpClient;
use super::types::{AmpConsoleEvent, AmpStatusEvent};

const POLL_INTERVAL: Duration = Duration::from_millis(500);
const BACKOFF_ON_ERROR: Duration = Duration::from_secs(3);

pub struct PollerHandle {
    server_id: String,
    stop: Arc<AtomicBool>,
    task: Option<JoinHandle<()>>,
}

impl PollerHandle {
    pub fn spawn<R: Runtime>(
        app: tauri::AppHandle<R>,
        server_id: String,
        client: Arc<AmpClient>,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let task = tokio::spawn(run_loop(
            app,
            server_id.clone(),
            client,
            stop.clone(),
        ));
        Self {
            server_id,
            stop,
            task: Some(task),
        }
    }

    pub fn server_id(&self) -> &str {
        &self.server_id
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl Drop for PollerHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

async fn run_loop<R: Runtime>(
    app: tauri::AppHandle<R>,
    server_id: String,
    client: Arc<AmpClient>,
    stop: Arc<AtomicBool>,
) {
    let console_event = format!("amp://console/{server_id}");
    let status_event = format!("amp://status/{server_id}");

    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }

        match client.get_updates().await {
            Ok(updates) => {
                let now_ms = now_ms();
                for entry in updates.console_entries {
                    let level = classify_level(&entry.entry_type);
                    let payload = AmpConsoleEvent {
                        server_id: server_id.clone(),
                        text: entry.contents,
                        level,
                        source: Some(entry.source).filter(|s| !s.is_empty()),
                        timestamp_ms: now_ms,
                    };
                    if let Err(e) = app.emit(&console_event, payload) {
                        tracing::warn!(
                            "AMP poller failed to emit console event: {e}"
                        );
                    }
                }

                if let Some(status) = updates.status {
                    let cpu = status
                        .metrics
                        .as_ref()
                        .and_then(|m| m.cpu_usage.as_ref())
                        .map(|m| m.percent as f32)
                        .unwrap_or(0.0);
                    let (ram_used, ram_total) = status
                        .metrics
                        .as_ref()
                        .and_then(|m| m.memory_usage.as_ref())
                        .map(|m| {
                            let scale = match m.units.as_str() {
                                "MB" => 1024u64 * 1024,
                                "GB" => 1024u64 * 1024 * 1024,
                                "KB" => 1024,
                                _ => 1,
                            };
                            (
                                (m.raw_value * scale as f64) as u64,
                                (m.max_value * scale as f64) as u64,
                            )
                        })
                        .unwrap_or((0, 0));
                    let payload = AmpStatusEvent {
                        server_id: server_id.clone(),
                        power_state:
                            super::types::PowerState::from_amp_state(
                                status.state,
                            ),
                        cpu_percent: cpu,
                        ram_usage_bytes: ram_used,
                        ram_total_bytes: ram_total,
                        uptime_seconds: 0,
                        connection_ok: true,
                        connection_error: None,
                    };
                    if let Err(e) = app.emit(&status_event, payload) {
                        tracing::warn!(
                            "AMP poller failed to emit status event: {e}"
                        );
                    }
                }
                time::sleep(POLL_INTERVAL).await;
            }
            Err(err) => {
                let payload = AmpStatusEvent {
                    server_id: server_id.clone(),
                    power_state: super::types::PowerState::Unknown,
                    cpu_percent: 0.0,
                    ram_usage_bytes: 0,
                    ram_total_bytes: 0,
                    uptime_seconds: 0,
                    connection_ok: false,
                    connection_error: Some(err.to_string()),
                };
                let _ = app.emit(&status_event, payload);
                tracing::debug!(
                    "AMP poll failed for {server_id}: {err}; backing off"
                );
                time::sleep(BACKOFF_ON_ERROR).await;
            }
        }
    }
}

fn classify_level(entry_type: &str) -> String {
    match entry_type.to_ascii_lowercase().as_str() {
        "error" | "err" | "fatal" => "error".to_owned(),
        "warn" | "warning" => "warn".to_owned(),
        "debug" => "debug".to_owned(),
        "trace" => "trace".to_owned(),
        _ => "info".to_owned(),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
