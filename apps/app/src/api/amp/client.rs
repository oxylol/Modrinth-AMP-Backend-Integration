//! AMP HTTP client.
//!
//! AMP exposes all API calls as `POST {base}/API/{Module}/{Method}` with a
//! JSON body that includes `{ SESSIONID, ... }`. Sessions are obtained via
//! `Core/Login` and expire silently — we detect expiry by receiving
//! `{ Status: false, Title: "Session expired" }` or HTTP 401 and re-auth.
//!
//! ADS (AMP's controller mode) exposes a per-instance proxy at
//! `{base}/API/ADSModule/Servers/{instance_id}/API/{Module}/{Method}` that
//! transparently routes to the instance's own API.

use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::RwLock;
use url::Url;

use super::error::AmpError;
use super::types::{AmpInstanceInfo, AmpTestResult, PowerState};

const AMP_API_TIMEOUT: Duration = Duration::from_secs(20);

/// Stateful AMP client — holds the session token and rebuilds it on expiry.
pub struct AmpClient {
    http: Client,
    base_url: Url,
    username: String,
    password: String,
    /// Optional instance ID — if set, all Core/* calls are routed through the
    /// ADS instance proxy. If None, we talk to the top-level panel (which for
    /// standalone AMP IS the game server, and for ADS is the controller).
    instance_id: Option<String>,
    session_id: RwLock<Option<String>>,
}

impl AmpClient {
    pub fn new(
        base_url: &str,
        username: &str,
        password: &str,
        insecure_tls: bool,
        instance_id: Option<String>,
    ) -> Result<Self, AmpError> {
        let base_url = parse_base_url(base_url)?;
        let http = Client::builder()
            .danger_accept_invalid_certs(insecure_tls)
            .timeout(AMP_API_TIMEOUT)
            .build()?;
        Ok(Self {
            http,
            base_url,
            username: username.to_owned(),
            password: password.to_owned(),
            instance_id,
            session_id: RwLock::new(None),
        })
    }

    pub fn with_instance(&self, instance_id: String) -> Result<Self, AmpError> {
        let http = self.http.clone();
        Ok(Self {
            http,
            base_url: self.base_url.clone(),
            username: self.username.clone(),
            password: self.password.clone(),
            instance_id: Some(instance_id),
            session_id: RwLock::new(None),
        })
    }

    async fn login(&self) -> Result<String, AmpError> {
        #[derive(Serialize)]
        #[serde(rename_all = "PascalCase")]
        struct LoginBody<'a> {
            username: &'a str,
            password: &'a str,
            token: &'a str,
            remember_me: bool,
        }
        #[derive(Deserialize)]
        struct LoginResponse {
            #[serde(rename = "sessionID", default)]
            session_id: Option<String>,
            #[serde(default)]
            success: Option<bool>,
            #[serde(rename = "resultReason", default)]
            result_reason: Option<String>,
        }

        let url = self.endpoint_url("Core/Login", true)?;
        let body = LoginBody {
            username: &self.username,
            password: &self.password,
            token: "",
            remember_me: false,
        };
        let resp = self.http.post(url).json(&body).send().await?;
        if !resp.status().is_success() {
            return Err(AmpError::Auth(format!(
                "HTTP {} during login",
                resp.status()
            )));
        }
        let parsed: LoginResponse = resp.json().await?;
        if parsed.success == Some(false) || parsed.session_id.is_none() {
            return Err(AmpError::Auth(
                parsed
                    .result_reason
                    .unwrap_or_else(|| "Invalid credentials".to_owned()),
            ));
        }
        let session = parsed
            .session_id
            .ok_or_else(|| AmpError::Auth("No sessionID in response".to_owned()))?;
        *self.session_id.write().await = Some(session.clone());
        Ok(session)
    }

    async fn current_session(&self) -> Result<String, AmpError> {
        if let Some(s) = self.session_id.read().await.clone() {
            return Ok(s);
        }
        self.login().await
    }

    /// Build the full URL for a given AMP module/method. When `skip_instance`
    /// is true we address the panel itself (used for Login + ADS discovery).
    fn endpoint_url(
        &self,
        path: &str,
        skip_instance: bool,
    ) -> Result<Url, AmpError> {
        let path = if !skip_instance
            && let Some(instance) = &self.instance_id
        {
            format!("API/ADSModule/Servers/{instance}/API/{path}")
        } else {
            format!("API/{path}")
        };
        Ok(self.base_url.join(&path)?)
    }

    /// Invoke an AMP API method. The argument body should be a JSON object
    /// (NOT wrapped in SESSIONID — we add that ourselves). Session expiry
    /// triggers a single silent re-login + retry.
    pub async fn call<T>(
        &self,
        method_path: &str,
        args: Value,
    ) -> Result<T, AmpError>
    where
        T: for<'de> Deserialize<'de>,
    {
        self.call_inner(method_path, args, false).await
    }

    async fn call_inner<T>(
        &self,
        method_path: &str,
        args: Value,
        is_retry: bool,
    ) -> Result<T, AmpError>
    where
        T: for<'de> Deserialize<'de>,
    {
        let session = self.current_session().await?;
        let mut body = args.clone();
        if let Value::Object(map) = &mut body {
            map.insert(
                "SESSIONID".to_owned(),
                Value::String(session.clone()),
            );
        } else {
            return Err(AmpError::Api(
                "AMP API arg body must be a JSON object".to_owned(),
            ));
        }

        let url = self.endpoint_url(method_path, false)?;
        let resp = self.http.post(url).json(&body).send().await?;
        let status = resp.status();

        if status == StatusCode::UNAUTHORIZED && !is_retry {
            *self.session_id.write().await = None;
            return Box::pin(self.call_inner(method_path, args, true)).await;
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(AmpError::Api(format!("HTTP {status}: {text}")));
        }

        let raw: Value = resp.json().await?;

        if let Some(obj) = raw.as_object()
            && let Some(Value::Bool(false)) = obj.get("Status")
        {
            let reason = obj
                .get("Title")
                .and_then(Value::as_str)
                .unwrap_or("Unknown AMP API error");
            if !is_retry
                && (reason.eq_ignore_ascii_case("Session expired")
                    || reason.to_lowercase().contains("session"))
            {
                *self.session_id.write().await = None;
                return Box::pin(self.call_inner(method_path, args, true))
                    .await;
            }
            return Err(AmpError::Api(reason.to_owned()));
        }

        let parsed: T = serde_json::from_value(raw)?;
        Ok(parsed)
    }

    pub async fn test_connection(&self) -> Result<AmpTestResult, AmpError> {
        self.login().await?;
        let status: Value =
            self.call("Core/GetStatus", json!({})).await?;
        let panel_name = status
            .get("InstanceName")
            .and_then(Value::as_str)
            .map(|s| s.to_owned());

        let instance_count = match self
            .call::<Vec<AmpInstanceRaw>>("ADSModule/GetInstances", json!({}))
            .await
        {
            Ok(list) => list.iter().map(|c| c.available_instances.len()).sum(),
            Err(_) => 1, // not ADS — treat as one instance
        };

        Ok(AmpTestResult {
            ok: true,
            message: "Connected".to_owned(),
            instance_count,
            panel_name,
        })
    }

    pub async fn list_instances(
        &self,
    ) -> Result<Vec<AmpInstanceInfo>, AmpError> {
        match self
            .call::<Vec<AmpInstanceRaw>>("ADSModule/GetInstances", json!({}))
            .await
        {
            Ok(groups) => {
                let mut out = Vec::new();
                for group in groups {
                    for inst in group.available_instances {
                        out.push(AmpInstanceInfo {
                            instance_id: inst.instance_id.clone(),
                            friendly_name: inst
                                .friendly_name
                                .unwrap_or(inst.instance_id.clone()),
                            module: inst.module.unwrap_or_default(),
                            running: inst.running.unwrap_or(false),
                            server_id: String::new(), // filled in by state layer
                        });
                    }
                }
                Ok(out)
            }
            Err(_) => {
                let status: Value =
                    self.call("Core/GetStatus", json!({})).await?;
                let name = status
                    .get("InstanceName")
                    .and_then(Value::as_str)
                    .unwrap_or("AMP Instance")
                    .to_owned();
                Ok(vec![AmpInstanceInfo {
                    instance_id: "default".to_owned(),
                    friendly_name: name,
                    module: status
                        .get("Module")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                    running: true,
                    server_id: String::new(),
                }])
            }
        }
    }

    pub async fn power(&self, endpoint: &str) -> Result<(), AmpError> {
        let _: Value = self.call(endpoint, json!({})).await?;
        Ok(())
    }

    pub async fn send_console_message(
        &self,
        message: &str,
    ) -> Result<(), AmpError> {
        let _: Value = self
            .call("Core/SendConsoleMessage", json!({ "message": message }))
            .await?;
        Ok(())
    }

    pub async fn get_updates(
        &self,
    ) -> Result<GetUpdatesResponse, AmpError> {
        self.call("Core/GetUpdates", json!({})).await
    }

    pub async fn get_status(&self) -> Result<CoreStatus, AmpError> {
        self.call("Core/GetStatus", json!({})).await
    }
}

fn parse_base_url(raw: &str) -> Result<Url, AmpError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AmpError::InvalidUrl("empty URL".to_owned()));
    }
    let with_scheme = if trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
    {
        trimmed.to_owned()
    } else {
        format!("http://{trimmed}")
    };
    let mut url = Url::parse(&with_scheme)
        .map_err(|e| AmpError::InvalidUrl(e.to_string()))?;
    if !url.path().ends_with('/') {
        url.set_path(&format!("{}/", url.path()));
    }
    Ok(url)
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct AmpInstanceRaw {
    #[serde(rename = "AvailableInstances", default)]
    available_instances: Vec<AmpInstanceDetailRaw>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct AmpInstanceDetailRaw {
    #[serde(rename = "InstanceID")]
    instance_id: String,
    #[serde(rename = "FriendlyName", default)]
    friendly_name: Option<String>,
    #[serde(rename = "Module", default)]
    module: Option<String>,
    #[serde(rename = "Running", default)]
    running: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CoreStatus {
    #[serde(default)]
    pub state: i32,
    #[serde(default)]
    pub uptime: String,
    pub metrics: Option<CoreStatusMetrics>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CoreStatusMetrics {
    #[serde(rename = "CPU Usage", default)]
    pub cpu_usage: Option<CoreStatusMetric>,
    #[serde(rename = "Memory Usage", default)]
    pub memory_usage: Option<CoreStatusMetric>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CoreStatusMetric {
    #[serde(rename = "RawValue", default)]
    pub raw_value: f64,
    #[serde(rename = "MaxValue", default)]
    pub max_value: f64,
    #[serde(rename = "Percent", default)]
    pub percent: f64,
    #[serde(default)]
    pub units: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetUpdatesResponse {
    #[serde(default)]
    pub console_entries: Vec<ConsoleEntry>,
    pub status: Option<UpdatesStatus>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ConsoleEntry {
    #[serde(default)]
    pub contents: String,
    #[serde(default)]
    pub source: String,
    #[serde(rename = "Type", default)]
    pub entry_type: String,
    #[serde(default)]
    pub timestamp: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UpdatesStatus {
    #[serde(default)]
    pub state: i32,
    pub metrics: Option<CoreStatusMetrics>,
}

impl CoreStatus {
    pub fn power_state(&self) -> PowerState {
        PowerState::from_amp_state(self.state)
    }
    pub fn cpu_percent(&self) -> f32 {
        self.metrics
            .as_ref()
            .and_then(|m| m.cpu_usage.as_ref())
            .map(|m| m.percent as f32)
            .unwrap_or(0.0)
    }
    pub fn ram_usage(&self) -> (u64, u64) {
        let mem = self
            .metrics
            .as_ref()
            .and_then(|m| m.memory_usage.as_ref());
        match mem {
            Some(m) => {
                let scale = match m.units.as_str() {
                    "MB" => 1024u64 * 1024,
                    "GB" => 1024u64 * 1024 * 1024,
                    "KB" => 1024,
                    _ => 1,
                };
                ((m.raw_value * scale as f64) as u64,
                 (m.max_value * scale as f64) as u64)
            }
            None => (0, 0),
        }
    }
    pub fn uptime_seconds(&self) -> u64 {
        parse_uptime(&self.uptime)
    }
}

/// AMP reports uptime as `"D.HH:MM:SS"` (or `"HH:MM:SS"` for <24h).
/// Returns 0 on any parse failure.
fn parse_uptime(s: &str) -> u64 {
    let mut days = 0u64;
    let rest = if let Some((d, r)) = s.split_once('.') {
        days = d.parse().unwrap_or(0);
        r
    } else {
        s
    };
    let parts: Vec<&str> = rest.split(':').collect();
    let (h, m, sec) = match parts.as_slice() {
        [h, m, s] => (h, m, s),
        _ => return 0,
    };
    let h: u64 = h.parse().unwrap_or(0);
    let m: u64 = m.parse().unwrap_or(0);
    let sec: u64 = sec.parse().unwrap_or(0);
    days * 86400 + h * 3600 + m * 60 + sec
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_uptime_under_day() {
        assert_eq!(parse_uptime("01:30:45"), 3600 + 30 * 60 + 45);
    }

    #[test]
    fn parses_uptime_with_days() {
        assert_eq!(parse_uptime("2.12:00:00"), 2 * 86400 + 12 * 3600);
    }

    #[test]
    fn parse_uptime_bad_input_is_zero() {
        assert_eq!(parse_uptime(""), 0);
        assert_eq!(parse_uptime("garbage"), 0);
    }

    #[test]
    fn normalizes_base_url_without_scheme() {
        let u = parse_base_url("192.168.1.1:8080").unwrap();
        assert_eq!(u.scheme(), "http");
        assert_eq!(u.host_str(), Some("192.168.1.1"));
        assert_eq!(u.port(), Some(8080));
        assert_eq!(u.path(), "/");
    }

    #[test]
    fn preserves_https_scheme() {
        let u = parse_base_url("https://panel.example.com").unwrap();
        assert_eq!(u.scheme(), "https");
    }

    #[test]
    fn power_state_mapping() {
        assert_eq!(PowerState::from_amp_state(0), PowerState::Stopped);
        assert_eq!(PowerState::from_amp_state(10), PowerState::Starting);
        assert_eq!(PowerState::from_amp_state(20), PowerState::Running);
        assert_eq!(PowerState::from_amp_state(25), PowerState::Stopping);
        assert_eq!(PowerState::from_amp_state(60), PowerState::Crashed);
        assert_eq!(PowerState::from_amp_state(999), PowerState::Unknown);
    }
}
