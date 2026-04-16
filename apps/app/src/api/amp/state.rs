//! Central plugin state: connection registry + live instance handles.
//!
//! Server ID format consumed by all commands:
//!     amp_{connection_uuid}_{instance_id}
//! where `instance_id` is `"default"` for non-ADS panels.

use std::collections::HashMap;
use std::sync::Arc;

use tauri::Runtime;
use uuid::Uuid;

use super::client::AmpClient;
use super::config::{self, AmpConfigStore};
use super::error::AmpError;
use super::poller::PollerHandle;
use super::types::{
    AmpConnection, AmpConnectionDraft, AmpInstanceInfo, AmpServerStatus,
    PowerAction, PowerState,
};

struct LiveInstance {
    client: Arc<AmpClient>,
    subscriber_count: usize,
    poller: Option<PollerHandle>,
}

pub struct AmpPluginState<R: Runtime> {
    store: AmpConfigStore,
    app: tauri::AppHandle<R>,
    instances: HashMap<String, LiveInstance>,
}

impl<R: Runtime> AmpPluginState<R> {
    pub fn new(store: AmpConfigStore, app: tauri::AppHandle<R>) -> Self {
        Self {
            store,
            app,
            instances: HashMap::new(),
        }
    }

    pub fn list_connections(&self) -> Vec<AmpConnection> {
        self.store.list()
    }

    pub async fn add_connection(
        &mut self,
        draft: AmpConnectionDraft,
    ) -> Result<AmpConnection, AmpError> {
        let probe = AmpClient::new(
            &draft.base_url,
            &draft.username,
            &draft.password,
            draft.insecure_tls,
            None,
        )?;
        probe.test_connection().await?;

        let connection_id = Uuid::new_v4();
        let connection = AmpConnection {
            connection_id,
            base_url: draft.base_url,
            username: draft.username,
            friendly_name: draft.friendly_name,
            insecure_tls: draft.insecure_tls,
        };

        config::store_password(&connection_id, &draft.password)?;
        self.store.insert(connection.clone())?;
        Ok(connection)
    }

    pub async fn remove_connection(
        &mut self,
        connection_id: &str,
    ) -> Result<(), AmpError> {
        let id = Uuid::parse_str(connection_id)
            .map_err(|_| AmpError::ConnectionNotFound(connection_id.to_owned()))?;

        let prefix = format!("amp_{id}_");
        let keys: Vec<String> = self
            .instances
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .cloned()
            .collect();
        for k in keys {
            if let Some(mut live) = self.instances.remove(&k)
                && let Some(mut poller) = live.poller.take()
            {
                poller.stop();
            }
        }

        let removed = self.store.remove(&id)?;
        if !removed {
            return Err(AmpError::ConnectionNotFound(id.to_string()));
        }
        let _ = config::delete_password(&id);
        Ok(())
    }

    pub async fn list_instances(
        &mut self,
        connection_id: &str,
    ) -> Result<Vec<AmpInstanceInfo>, AmpError> {
        let id = Uuid::parse_str(connection_id)
            .map_err(|_| AmpError::ConnectionNotFound(connection_id.to_owned()))?;
        let conn = self
            .store
            .get(&id)
            .cloned()
            .ok_or_else(|| AmpError::ConnectionNotFound(id.to_string()))?;

        let password = config::load_password(&conn.connection_id)?;
        let root_client = AmpClient::new(
            &conn.base_url,
            &conn.username,
            &password,
            conn.insecure_tls,
            None,
        )?;
        let mut instances = root_client.list_instances().await?;
        for inst in &mut instances {
            inst.server_id = build_server_id(&id, &inst.instance_id);
        }
        Ok(instances)
    }

    pub async fn power(
        &mut self,
        server_id: &str,
        action: PowerAction,
    ) -> Result<(), AmpError> {
        let client = self.client_for(server_id).await?;
        client.power(action.amp_endpoint()).await
    }

    pub async fn send_command(
        &mut self,
        server_id: &str,
        command: &str,
    ) -> Result<(), AmpError> {
        let client = self.client_for(server_id).await?;
        client.send_console_message(command).await
    }

    pub async fn get_status(
        &mut self,
        server_id: &str,
    ) -> Result<AmpServerStatus, AmpError> {
        let client = self.client_for(server_id).await?;
        match client.get_status().await {
            Ok(status) => {
                let (ram_used, ram_total) = status.ram_usage();
                Ok(AmpServerStatus {
                    server_id: server_id.to_owned(),
                    power_state: status.power_state(),
                    cpu_percent: status.cpu_percent(),
                    ram_usage_bytes: ram_used,
                    ram_total_bytes: ram_total,
                    uptime_seconds: status.uptime_seconds(),
                    connection_ok: true,
                    connection_error: None,
                })
            }
            Err(err) => Ok(AmpServerStatus {
                server_id: server_id.to_owned(),
                power_state: PowerState::Unknown,
                cpu_percent: 0.0,
                ram_usage_bytes: 0,
                ram_total_bytes: 0,
                uptime_seconds: 0,
                connection_ok: false,
                connection_error: Some(err.to_string()),
            }),
        }
    }

    pub async fn subscribe(
        &mut self,
        server_id: &str,
    ) -> Result<(), AmpError> {
        let client = self.client_for(server_id).await?;
        let live = self
            .instances
            .get_mut(server_id)
            .expect("client_for just inserted this entry");
        live.subscriber_count += 1;
        if live.poller.is_none() {
            live.poller = Some(PollerHandle::spawn(
                self.app.clone(),
                server_id.to_owned(),
                client,
            ));
        }
        Ok(())
    }

    pub async fn unsubscribe(
        &mut self,
        server_id: &str,
    ) -> Result<(), AmpError> {
        if let Some(live) = self.instances.get_mut(server_id) {
            if live.subscriber_count > 0 {
                live.subscriber_count -= 1;
            }
            if live.subscriber_count == 0
                && let Some(mut poller) = live.poller.take()
            {
                poller.stop();
            }
        }
        Ok(())
    }

    async fn client_for(
        &mut self,
        server_id: &str,
    ) -> Result<Arc<AmpClient>, AmpError> {
        if let Some(live) = self.instances.get(server_id) {
            return Ok(live.client.clone());
        }

        let (connection_id, instance_id) = parse_server_id(server_id)?;
        let conn = self
            .store
            .get(&connection_id)
            .cloned()
            .ok_or_else(|| {
                AmpError::ConnectionNotFound(connection_id.to_string())
            })?;

        let password = config::load_password(&conn.connection_id)?;
        let per_instance = if instance_id == "default" {
            None
        } else {
            Some(instance_id.clone())
        };
        let client = Arc::new(AmpClient::new(
            &conn.base_url,
            &conn.username,
            &password,
            conn.insecure_tls,
            per_instance,
        )?);

        self.instances.insert(
            server_id.to_owned(),
            LiveInstance {
                client: client.clone(),
                subscriber_count: 0,
                poller: None,
            },
        );
        Ok(client)
    }
}

pub fn build_server_id(connection_id: &Uuid, instance_id: &str) -> String {
    format!("amp_{connection_id}_{instance_id}")
}

pub fn parse_server_id(server_id: &str) -> Result<(Uuid, String), AmpError> {
    let rest = server_id
        .strip_prefix("amp_")
        .ok_or_else(|| AmpError::InvalidServerId(server_id.to_owned()))?;
    let (conn_part, inst_part) = rest
        .split_once('_')
        .ok_or_else(|| AmpError::InvalidServerId(server_id.to_owned()))?;
    let conn = Uuid::parse_str(conn_part)
        .map_err(|_| AmpError::InvalidServerId(server_id.to_owned()))?;
    if inst_part.is_empty() {
        return Err(AmpError::InvalidServerId(server_id.to_owned()));
    }
    Ok((conn, inst_part.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_id_roundtrip() {
        let conn = Uuid::new_v4();
        let id = build_server_id(&conn, "foo-bar");
        let (back, inst) = parse_server_id(&id).unwrap();
        assert_eq!(back, conn);
        assert_eq!(inst, "foo-bar");
    }

    #[test]
    fn server_id_default_instance() {
        let conn = Uuid::new_v4();
        let id = build_server_id(&conn, "default");
        let (_, inst) = parse_server_id(&id).unwrap();
        assert_eq!(inst, "default");
    }

    #[test]
    fn server_id_rejects_garbage() {
        assert!(parse_server_id("modrinth-server-id").is_err());
        assert!(parse_server_id("amp_").is_err());
        assert!(parse_server_id("amp_not-a-uuid_default").is_err());
    }
}
