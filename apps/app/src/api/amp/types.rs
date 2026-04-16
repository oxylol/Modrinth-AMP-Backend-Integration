use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A user-registered AMP panel. Password is NOT stored here; it lives in the
/// OS credential manager keyed by `connection_id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AmpConnection {
    pub connection_id: Uuid,
    pub base_url: String,
    pub username: String,
    pub friendly_name: String,
    #[serde(default)]
    pub insecure_tls: bool,
}

/// Inputs for creating a new connection. Password goes straight to the keyring
/// and is never persisted elsewhere.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AmpConnectionDraft {
    pub base_url: String,
    pub username: String,
    pub password: String,
    pub friendly_name: String,
    #[serde(default)]
    pub insecure_tls: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AmpTestResult {
    pub ok: bool,
    pub message: String,
    pub instance_count: usize,
    /// AMP panel's self-reported name, useful for pre-filling friendly_name.
    pub panel_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AmpInstanceInfo {
    pub instance_id: String,
    pub friendly_name: String,
    pub module: String,
    pub running: bool,
    /// Synthetic ID consumed by frontend routing: `amp_{conn}_{inst}`.
    pub server_id: String,
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum PowerAction {
    Start,
    Stop,
    Restart,
    Kill,
}

impl PowerAction {
    pub fn amp_endpoint(self) -> &'static str {
        match self {
            Self::Start => "Core/Start",
            Self::Stop => "Core/Stop",
            Self::Restart => "Core/Restart",
            Self::Kill => "Core/Kill",
        }
    }
}

/// Normalized power state exposed to the frontend. Matches the string union
/// `Archon.Websocket.v0.PowerState` so the existing console/server UI can
/// reuse it.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PowerState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Crashed,
    Unknown,
}

impl PowerState {
    /// AMP `Core/GetStatus` returns `State` as an integer. Map to our enum.
    ///
    /// AMP states (from AMP docs):
    ///    0 = Undefined, 5 = PreStart, 7 = Configuring, 10 = Starting,
    ///   15 = Ready (not used), 20 = Restarting, 25 = Stopping, 30 = PreparingForSleep,
    ///   35 = Sleeping, 40 = Waiting, 45 = Installing, 50 = Updating, 55 = AwaitingUserInput,
    ///   60 = Failed, 65 = Suspended, 70 = Maintenance, 75 = Indeterminate
    /// The canonical "running" value in practice is 10 when RunningMainTask is true,
    /// but most panels report `Running = 2` via the higher-level status. Empirically
    /// we map both forms. Values we can't identify become `Unknown`.
    pub fn from_amp_state(state: i32) -> Self {
        match state {
            0 => Self::Stopped,
            5 | 7 | 10 => Self::Starting,
            2 | 20 => Self::Running,
            25 | 30 => Self::Stopping,
            60 => Self::Crashed,
            _ => Self::Unknown,
        }
    }
}

/// Lightweight status snapshot consumed by the server list + detail page.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AmpServerStatus {
    pub server_id: String,
    pub power_state: PowerState,
    pub cpu_percent: f32,
    pub ram_usage_bytes: u64,
    pub ram_total_bytes: u64,
    pub uptime_seconds: u64,
    pub connection_ok: bool,
    pub connection_error: Option<String>,
}

/// Event payload for `amp://console/{server_id}`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AmpConsoleEvent {
    pub server_id: String,
    pub text: String,
    pub level: String,
    pub source: Option<String>,
    pub timestamp_ms: u64,
}

/// Event payload for `amp://status/{server_id}`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AmpStatusEvent {
    pub server_id: String,
    pub power_state: PowerState,
    pub cpu_percent: f32,
    pub ram_usage_bytes: u64,
    pub ram_total_bytes: u64,
    pub uptime_seconds: u64,
    pub connection_ok: bool,
    pub connection_error: Option<String>,
}
