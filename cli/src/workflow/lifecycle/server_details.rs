//! Read the server settings used by a managed local process.

use std::{env, fs, path::PathBuf};

use crate::{error::Result, workflow::instance::ManagedLayout};

pub(super) struct ServerDetails {
    pub data_path:          PathBuf,
    pub bootstrap_password: Option<String>,
}

impl ServerDetails {
    pub fn read(layout: &ManagedLayout) -> Result<Self> {
        let config = if layout.config_path.is_file() {
            toml::from_str::<toml::Value>(&fs::read_to_string(&layout.config_path)?)
                .map_err(|error| crate::error::CLIError::ConfigurationError(error.to_string()))?
        } else {
            toml::Value::Table(Default::default())
        };
        let data_path = env::var("KALAMDB_DATA_DIR")
            .ok()
            .or_else(|| config.get("storage")?.get("data_path")?.as_str().map(str::to_owned))
            .map(PathBuf::from)
            .unwrap_or_else(|| layout.data_dir.clone());
        let data_path = if data_path.is_absolute() {
            data_path
        } else {
            layout.working_dir.join(data_path)
        };
        let bootstrap_password = env::var("KALAMDB_ROOT_PASSWORD")
            .ok()
            .filter(|value| !value.is_empty())
            .or_else(|| {
                config
                    .get("auth")?
                    .get("root_password")?
                    .as_str()
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
            });
        Ok(Self {
            data_path,
            bootstrap_password,
        })
    }
}
