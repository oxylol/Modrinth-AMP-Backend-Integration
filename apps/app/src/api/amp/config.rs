//! On-disk + keyring persistence for AMP connections.
//!
//! Connection metadata (URL, username, friendly name, insecure_tls flag) lives
//! in `{app_data_dir}/amp_connections.json`. Passwords go to the OS credential
//! manager keyed by the connection UUID.

use std::fs;
use std::path::{Path, PathBuf};

use keyring::Entry;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::error::AmpError;
use super::types::AmpConnection;

const KEYRING_SERVICE: &str = "modrinth-app-amp";
const CONFIG_FILE: &str = "amp_connections.json";

#[derive(Default, Serialize, Deserialize)]
struct OnDiskConfig {
    #[serde(default)]
    connections: Vec<AmpConnection>,
}

pub struct AmpConfigStore {
    path: PathBuf,
    connections: Vec<AmpConnection>,
}

impl AmpConfigStore {
    pub fn load(app_data_dir: &Path) -> Result<Self, AmpError> {
        fs::create_dir_all(app_data_dir)?;
        let path = app_data_dir.join(CONFIG_FILE);
        let connections = if path.exists() {
            let raw = fs::read_to_string(&path)?;
            let disk: OnDiskConfig = serde_json::from_str(&raw)?;
            disk.connections
        } else {
            Vec::new()
        };
        Ok(Self { path, connections })
    }

    pub fn list(&self) -> Vec<AmpConnection> {
        self.connections.clone()
    }

    pub fn get(&self, id: &Uuid) -> Option<&AmpConnection> {
        self.connections
            .iter()
            .find(|c| &c.connection_id == id)
    }

    pub fn insert(&mut self, connection: AmpConnection) -> Result<(), AmpError> {
        if let Some(existing) = self
            .connections
            .iter_mut()
            .find(|c| c.connection_id == connection.connection_id)
        {
            *existing = connection;
        } else {
            self.connections.push(connection);
        }
        self.persist()
    }

    pub fn remove(&mut self, id: &Uuid) -> Result<bool, AmpError> {
        let before = self.connections.len();
        self.connections.retain(|c| &c.connection_id != id);
        let removed = self.connections.len() != before;
        if removed {
            self.persist()?;
        }
        Ok(removed)
    }

    fn persist(&self) -> Result<(), AmpError> {
        let disk = OnDiskConfig {
            connections: self.connections.clone(),
        };
        let serialized = serde_json::to_string_pretty(&disk)?;
        fs::write(&self.path, serialized)?;
        Ok(())
    }
}

pub fn store_password(id: &Uuid, password: &str) -> Result<(), AmpError> {
    let entry = Entry::new(KEYRING_SERVICE, &id.to_string())?;
    entry.set_password(password)?;
    Ok(())
}

pub fn load_password(id: &Uuid) -> Result<String, AmpError> {
    let entry = Entry::new(KEYRING_SERVICE, &id.to_string())?;
    entry.get_password().map_err(Into::into)
}

pub fn delete_password(id: &Uuid) -> Result<(), AmpError> {
    let entry = Entry::new(KEYRING_SERVICE, &id.to_string())?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}
