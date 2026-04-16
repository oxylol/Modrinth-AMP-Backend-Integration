use thiserror::Error;

/// AMP plugin errors. These bubble up through `TheseusSerializableError::Amp`
/// and are serialized to the frontend as `{ field_name: "Amp", message }`.
#[derive(Error, Debug)]
pub enum AmpError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("URL parse error: {0}")]
    Url(#[from] url::ParseError),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Credential store error: {0}")]
    Keyring(#[from] keyring::Error),

    #[error("AMP API error: {0}")]
    Api(String),

    #[error("Authentication failed: {0}")]
    Auth(String),

    #[error("Connection '{0}' not found")]
    ConnectionNotFound(String),

    #[error("Instance '{0}' not found")]
    InstanceNotFound(String),

    #[error("Invalid server ID '{0}' (expected format: amp_<connection>_<instance>)")]
    InvalidServerId(String),

    #[error("Invalid AMP URL: {0}")]
    InvalidUrl(String),
}
