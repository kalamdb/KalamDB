use std::{env, path::PathBuf};

use crate::history::get_kalam_config_dir;

pub(super) fn default_credentials_path() -> PathBuf {
    resolve_default_credentials_path(
        env::var("KALAMDB_CREDENTIALS_PATH").ok().as_deref(),
        get_kalam_config_dir(),
    )
}

fn resolve_default_credentials_path(env_path: Option<&str>, config_dir: PathBuf) -> PathBuf {
    if let Some(path) = env_path {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    config_dir.join("credentials.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_path_wins_when_set() {
        let resolved = resolve_default_credentials_path(
            Some(" /tmp/kalam/credentials.toml "),
            PathBuf::from("/home/test/.kalam"),
        );

        assert_eq!(resolved, PathBuf::from("/tmp/kalam/credentials.toml"));
    }

    #[test]
    fn blank_env_path_falls_back_to_config_dir() {
        let resolved =
            resolve_default_credentials_path(Some("  "), PathBuf::from("/home/test/.kalam"));

        assert_eq!(resolved, PathBuf::from("/home/test/.kalam/credentials.toml"));
    }
}
