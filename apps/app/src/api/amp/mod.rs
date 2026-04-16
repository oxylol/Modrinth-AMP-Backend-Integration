//! AMP (CubeCoders) backend plugin.
//!
//! Exposes a Tauri plugin `amp` that lets the frontend manage user-added AMP
//! panels alongside Modrinth's hosted servers. See `AMP_INTEGRATION_DESIGN.md`
//! at the repo root for the architecture.

mod client;
mod config;
mod error;
mod poller;
mod state;
mod types;

pub use error::AmpError;

use std::sync::Arc;

use tauri::plugin::TauriPlugin;
use tauri::{Manager, Runtime};
use tokio::sync::RwLock;

use crate::api::Result;

use self::client::AmpClient;
use self::config::AmpConfigStore;
use self::state::AmpPluginState;
use self::types::{
    AmpConnection, AmpConnectionDraft, AmpInstanceInfo, AmpServerStatus,
    AmpTestResult, PowerAction,
};

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    tauri::plugin::Builder::<R>::new("amp")
        .setup(|app, _api| {
            let app_handle = app.clone();
            let app_data_dir = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("Failed to resolve app data dir: {e}"))?;

            let store = AmpConfigStore::load(&app_data_dir)
                .map_err(|e| format!("Failed to load AMP config: {e}"))?;

            app.manage(Arc::new(RwLock::new(AmpPluginState::new(
                store,
                app_handle,
            ))));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            amp_test_connection,
            amp_add_connection,
            amp_remove_connection,
            amp_list_connections,
            amp_list_instances,
            amp_power,
            amp_send_command,
            amp_get_status,
            amp_subscribe,
            amp_unsubscribe,
        ])
        .build()
}

fn state_arc<R: Runtime>(
    app: &tauri::AppHandle<R>,
) -> Arc<RwLock<AmpPluginState<R>>> {
    let handle = app.state::<Arc<RwLock<AmpPluginState<R>>>>();
    handle.inner().clone()
}

#[tauri::command]
pub async fn amp_test_connection(
    url: String,
    username: String,
    password: String,
    insecure_tls: bool,
) -> Result<AmpTestResult> {
    let client =
        AmpClient::new(&url, &username, &password, insecure_tls, None)?;
    let result = client.test_connection().await?;
    Ok(result)
}

#[tauri::command]
pub async fn amp_add_connection<R: Runtime>(
    app: tauri::AppHandle<R>,
    draft: AmpConnectionDraft,
) -> Result<AmpConnection> {
    let state = state_arc(&app);
    let connection = {
        let mut state = state.write().await;
        state.add_connection(draft).await?
    };
    Ok(connection)
}

#[tauri::command]
pub async fn amp_remove_connection<R: Runtime>(
    app: tauri::AppHandle<R>,
    connection_id: String,
) -> Result<()> {
    let state = state_arc(&app);
    let mut state = state.write().await;
    state.remove_connection(&connection_id).await?;
    Ok(())
}

#[tauri::command]
pub async fn amp_list_connections<R: Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<Vec<AmpConnection>> {
    let state = state_arc(&app);
    let state = state.read().await;
    Ok(state.list_connections())
}

#[tauri::command]
pub async fn amp_list_instances<R: Runtime>(
    app: tauri::AppHandle<R>,
    connection_id: String,
) -> Result<Vec<AmpInstanceInfo>> {
    let state = state_arc(&app);
    let instances = {
        let mut state = state.write().await;
        state.list_instances(&connection_id).await?
    };
    Ok(instances)
}

#[tauri::command]
pub async fn amp_power<R: Runtime>(
    app: tauri::AppHandle<R>,
    server_id: String,
    action: PowerAction,
) -> Result<()> {
    let state = state_arc(&app);
    let mut state = state.write().await;
    state.power(&server_id, action).await?;
    Ok(())
}

#[tauri::command]
pub async fn amp_send_command<R: Runtime>(
    app: tauri::AppHandle<R>,
    server_id: String,
    command: String,
) -> Result<()> {
    let state = state_arc(&app);
    let mut state = state.write().await;
    state.send_command(&server_id, &command).await?;
    Ok(())
}

#[tauri::command]
pub async fn amp_get_status<R: Runtime>(
    app: tauri::AppHandle<R>,
    server_id: String,
) -> Result<AmpServerStatus> {
    let state = state_arc(&app);
    let mut state = state.write().await;
    let status = state.get_status(&server_id).await?;
    Ok(status)
}

#[tauri::command]
pub async fn amp_subscribe<R: Runtime>(
    app: tauri::AppHandle<R>,
    server_id: String,
) -> Result<()> {
    let state = state_arc(&app);
    let mut state = state.write().await;
    state.subscribe(&server_id).await?;
    Ok(())
}

#[tauri::command]
pub async fn amp_unsubscribe<R: Runtime>(
    app: tauri::AppHandle<R>,
    server_id: String,
) -> Result<()> {
    let state = state_arc(&app);
    let mut state = state.write().await;
    state.unsubscribe(&server_id).await?;
    Ok(())
}
