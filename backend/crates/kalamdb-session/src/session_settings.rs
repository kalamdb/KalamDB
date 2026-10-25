//! Bounded connection/request settings with PostgreSQL transaction scopes.
use std::{
    collections::HashSet,
    sync::{Arc, Mutex, MutexGuard},
};

use crate::{
    postgres_settings::{canonical_name, default_value, validate, SETTINGS},
    settings_state::{ResolvedSchema, SettingsState},
};

/// One `pg_settings` row: the effective value and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingEntry {
    pub name: String,
    pub value: String,
    pub source: &'static str,
}

#[derive(Debug, Clone, Default)]
pub struct SessionSettings {
    state: Arc<Mutex<SettingsState>>,
}

impl SessionSettings {
    fn lock(&self) -> MutexGuard<'_, SettingsState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub fn search_path(&self) -> Option<Arc<crate::SearchPath>> {
        let mut state = self.lock();
        let text = state
            .local
            .get("search_path")
            .or_else(|| state.session.get("search_path"))?
            .clone();
        Some(parsed_path(&mut state, &text)?)
    }

    /// Schema selected by `search_path` for `user`.
    ///
    /// The parsed path and the chosen schema are kept. A later call re-walks the
    /// catalog only when the path text, user, or previously chosen schema changed.
    pub fn resolved_schema(
        &self,
        user: &str,
        mut exists: impl FnMut(&str) -> bool,
    ) -> Option<String> {
        let mut state = self.lock();
        let text = state
            .local
            .get("search_path")
            .or_else(|| state.session.get("search_path"))?
            .clone();
        if let Some(cached) = &state.resolved_schema {
            if cached.text == text && cached.user == user {
                return Some(cached.schema.clone());
            }
        }
        let path = parsed_path(&mut state, &text)?;
        let schema = path.first_existing(user, &mut exists).unwrap_or_default();
        state.resolved_schema = Some(ResolvedSchema {
            text,
            user: user.to_string(),
            schema: schema.clone(),
        });
        Some(schema)
    }
    pub fn get(&self, name: &str) -> Option<String> {
        let name = canonical_name(name);
        let state = self.lock();
        state
            .local
            .get(&name)
            .or_else(|| state.session.get(&name))
            .cloned()
            .or_else(|| default_value(&name).map(str::to_owned))
    }
    pub fn set(&self, name: &str, value: &str) -> Result<String, String> {
        self.set_config(name, Some(value), false)
    }
    pub fn set_config(
        &self,
        name: &str,
        value: Option<&str>,
        local: bool,
    ) -> Result<String, String> {
        let name = canonical_name(name);
        let value = validate(&name, value)?;
        let mut state = self.lock();
        if local && state.baseline.is_none() {
            return Ok(value);
        }
        let target = if local { &state.local } else { &state.session };
        let bytes: usize = target
            .iter()
            .filter(|(key, _)| *key != &name)
            .map(|(k, v)| k.len() + v.len())
            .sum();
        if (!target.contains_key(&name) && target.len() >= 128)
            || bytes + name.len() + value.len() > 65536
        {
            return Err("session setting budget exceeded".into());
        }
        if local {
            Arc::make_mut(&mut state.local).insert(name, value.clone());
        } else {
            Arc::make_mut(&mut state.local).remove(&name);
            Arc::make_mut(&mut state.session).insert(name, value.clone());
        }
        Ok(value)
    }
    pub fn reset_all(&self) {
        let mut state = self.lock();
        state.local = Arc::default();
        state.session = Arc::default();
    }
    pub fn entries(&self) -> Vec<(String, String)> {
        self.catalog_entries()
            .into_iter()
            .map(|entry| (entry.name, entry.value))
            .collect()
    }

    /// Known settings plus custom placeholders, with PostgreSQL `source` values.
    pub fn catalog_entries(&self) -> Vec<SettingEntry> {
        let state = self.lock();
        let mut seen = HashSet::with_capacity(SETTINGS.len() + state.session.len());
        let mut rows = Vec::with_capacity(SETTINGS.len() + state.session.len());
        for (name, default) in SETTINGS {
            let (value, source) = effective(&state, name, default);
            seen.insert((*name).to_string());
            rows.push(SettingEntry {
                name: (*name).to_string(),
                value,
                source,
            });
        }
        for (name, value) in state.local.iter() {
            if seen.insert(name.clone()) {
                rows.push(SettingEntry {
                    name: name.clone(),
                    value: value.clone(),
                    source: "override",
                });
            }
        }
        for (name, value) in state.session.iter() {
            if seen.insert(name.clone()) {
                rows.push(SettingEntry {
                    name: name.clone(),
                    value: value.clone(),
                    source: "session",
                });
            }
        }
        rows
    }
    pub fn begin(&self) {
        let mut state = self.lock();
        if state.baseline.is_none() {
            state.baseline = Some(Arc::clone(&state.session));
        }
    }
    pub fn commit(&self) {
        let mut state = self.lock();
        state.baseline = None;
        state.local = Arc::default();
        state.savepoints.clear();
    }
    pub fn rollback(&self) {
        let mut state = self.lock();
        if let Some(baseline) = state.baseline.take() {
            state.session = baseline;
        }
        state.local = Arc::default();
        state.savepoints.clear();
    }
    pub fn savepoint(&self, name: &str) -> Result<(), String> {
        let mut state = self.lock();
        if state.savepoints.len() >= 64 {
            return Err("session setting savepoint limit exceeded".into());
        }
        let snapshot = (name.to_string(), Arc::clone(&state.session), Arc::clone(&state.local));
        state.savepoints.push(snapshot);
        Ok(())
    }
    pub fn rollback_to(&self, name: &str) {
        let mut state = self.lock();
        if let Some(i) = state.savepoints.iter().rposition(|(n, _, _)| n == name) {
            state.session = Arc::clone(&state.savepoints[i].1);
            state.local = Arc::clone(&state.savepoints[i].2);
            state.savepoints.truncate(i + 1);
        }
    }
    pub fn release(&self, name: &str) {
        let mut state = self.lock();
        if let Some(i) = state.savepoints.iter().rposition(|(n, _, _)| n == name) {
            state.savepoints.truncate(i);
        }
    }
}

fn effective(state: &SettingsState, name: &str, default: &str) -> (String, &'static str) {
    if let Some(value) = state.local.get(name) {
        (value.clone(), "override")
    } else if let Some(value) = state.session.get(name) {
        (value.clone(), "session")
    } else {
        (default.to_string(), "default")
    }
}

fn parsed_path(state: &mut SettingsState, text: &str) -> Option<Arc<crate::SearchPath>> {
    if let Some((cached, path)) = &state.parsed_path {
        if cached == text {
            return Some(Arc::clone(path));
        }
    }
    let path = Arc::new(crate::SearchPath::parse(text).ok()?);
    state.parsed_path = Some((text.to_string(), Arc::clone(&path)));
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transaction_and_savepoint_settings() {
        let s = SessionSettings::default();
        s.set("application_name", "before").unwrap();
        s.begin();
        s.set_config("application_name", Some("local"), true).unwrap();
        s.savepoint("a").unwrap();
        s.set("application_name", "session").unwrap();
        s.rollback_to("a");
        assert_eq!(s.get("application_name").as_deref(), Some("local"));
        s.commit();
        assert_eq!(s.get("application_name").as_deref(), Some("before"));
        s.begin();
        s.set("application_name", "discard").unwrap();
        s.rollback();
        assert_eq!(s.get("application_name").as_deref(), Some("before"));
    }
    #[test]
    fn validation_and_isolation() {
        let s = SessionSettings::default();
        assert!(s.set("statement_timeout", "5s").is_err());
        assert!(s.set("unknown", "x").is_err());
        assert!(s.set("search_path", "\"broken").is_err());
        s.set("app.test", "ok").unwrap();
        assert_eq!(s.get("app.test").as_deref(), Some("ok"));
        assert_eq!(SessionSettings::default().get("app.test"), None);
    }

    #[test]
    fn resolved_schema_is_reused_until_the_path_changes() {
        let settings = SessionSettings::default();
        settings.set("search_path", "public, default").unwrap();
        let mut lookups = 0;
        let schema = settings
            .resolved_schema("alice", |name| {
                lookups += 1;
                name == "public"
            })
            .expect("schema");
        assert_eq!(schema, "public");
        let again = settings
            .resolved_schema("alice", |_| panic!("catalog must not be walked on a cache hit"))
            .expect("cached schema");
        assert_eq!(again, "public");
        assert_eq!(lookups, 1);

        settings.set("search_path", "default").unwrap();
        let changed =
            settings.resolved_schema("alice", |name| name == "default").expect("new schema");
        assert_eq!(changed, "default");
    }

    #[test]
    fn catalog_entries_include_custom_values_and_sources() {
        let settings = SessionSettings::default();
        settings.set("application_name", "tabularis").unwrap();
        settings.set("app.tabularis_base_search_path", "public").unwrap();
        let rows = settings.catalog_entries();
        let app = rows
            .iter()
            .find(|row| row.name == "application_name")
            .expect("application_name");
        assert_eq!(app.value, "tabularis");
        assert_eq!(app.source, "session");
        let custom = rows
            .iter()
            .find(|row| row.name == "app.tabularis_base_search_path")
            .expect("custom guc");
        assert_eq!(custom.value, "public");
        assert_eq!(custom.source, "session");
        let version = rows.iter().find(|row| row.name == "server_version_num").expect("version");
        assert_eq!(version.source, "default");
    }
}
