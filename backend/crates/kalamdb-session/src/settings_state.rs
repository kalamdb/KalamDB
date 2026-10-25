use std::{collections::HashMap, sync::Arc};

/// Copy-on-write snapshots keep savepoints cheap until a setting is changed.
#[derive(Debug, Default)]
pub(crate) struct SettingsState {
    pub parsed_path: Option<(String, Arc<crate::SearchPath>)>,
    /// Resolved schema for the current path text and user. Recomputed only when
    /// either changes, or the previously chosen schema disappears.
    pub resolved_schema: Option<ResolvedSchema>,
    pub session: Arc<HashMap<String, String>>,
    pub local: Arc<HashMap<String, String>>,
    pub baseline: Option<Arc<HashMap<String, String>>>,
    pub savepoints: Vec<(String, Arc<HashMap<String, String>>, Arc<HashMap<String, String>>)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedSchema {
    pub text: String,
    pub user: String,
    pub schema: String,
}
