//! Pure `search_path` resolution.
//!
//! Callers pass a catalog view. This module does not read the namespace catalog.
//! [`catalog_entries_for_stored_keys`] is the only mapping from a stored key to
//! the name a path searches. The stored key `default` stays `default`.

use std::fmt;

/// What kind of object a lookup is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectClass {
    /// Tables, views, and other relations.
    Relation,
    /// Types and enums.
    Type,
    /// Functions and procedures.
    Routine,
    /// Operators.
    Operator,
}

/// One schema the resolver can see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaEntry {
    /// Name a path entry matches. This is the display name, not always the stored key.
    pub name:       String,
    /// Catalog key written for this schema. Equals [`Self::name`] except for the
    /// legacy `default` key, which is also visible as `public`.
    pub stored_key: String,
    /// `false` when the session lacks `USAGE`.
    pub accessible: bool,
    /// Objects in this schema.
    pub objects:    Vec<(ObjectClass, String)>,
}

/// Schemas visible to one resolution.
#[derive(Debug, Clone, Copy)]
pub struct Catalog<'a> {
    /// Session user, used by `"$user"`.
    pub user:        &'a str,
    /// Session temporary schema, such as `pg_temp_1`.
    pub temp_schema: Option<&'a str>,
    /// Existing schemas. Missing path entries are simply absent.
    pub schemas:     &'a [SchemaEntry],
}

/// An object found in one schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolved<'a> {
    /// Schema that held the object.
    pub schema: &'a str,
}

/// Failure while parsing a `search_path` value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathError {
    message: &'static str,
}

impl PathError {
    fn new(message: &'static str) -> Self {
        Self { message }
    }

    /// Static error text.
    #[must_use]
    pub fn message(&self) -> &'static str {
        self.message
    }
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message)
    }
}

impl std::error::Error for PathError {}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PathToken {
    User,
    Temp,
    Catalog,
    Schema(String),
}

/// Explicit `search_path` entries, before implicit `pg_temp` and `pg_catalog`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchPath {
    tokens: Vec<PathToken>,
}

impl SearchPath {
    /// PostgreSQL's default path: `"$user", public`.
    #[must_use]
    pub fn postgres_default() -> Self {
        Self {
            tokens: vec![PathToken::User, PathToken::Schema("public".to_string())],
        }
    }

    /// Parse a `search_path` value. An empty string is an empty path.
    ///
    /// # Errors
    ///
    /// Returns an error when an entry is empty or a quoted identifier is broken.
    pub fn parse(text: &str) -> Result<Self, PathError> {
        if text.trim().is_empty() {
            return Ok(Self { tokens: Vec::new() });
        }
        let mut tokens = Vec::new();
        for entry in split_entries(text)? {
            tokens.push(parse_token(entry)?);
        }
        Ok(Self { tokens })
    }

    /// First explicit schema that exists and is accessible.
    #[must_use]
    pub fn current_schema<'a>(&self, catalog: &Catalog<'a>) -> Option<&'a str> {
        self.current_entry(catalog).map(|schema| schema.name.as_str())
    }

    /// Stored catalog key of [`Self::current_schema`].
    ///
    /// A path entry `public` can resolve to the stored key `default` when that
    /// is the legacy schema. A schema stored as `public` is a different key.
    #[must_use]
    pub fn current_stored_key<'a>(&self, catalog: &Catalog<'a>) -> Option<&'a str> {
        self.current_entry(catalog).map(|schema| schema.stored_key.as_str())
    }

    fn current_entry<'a>(&self, catalog: &Catalog<'a>) -> Option<&'a SchemaEntry> {
        for name in self.explicit_names(catalog) {
            if let Some(schema) = find(catalog.schemas, &name) {
                if schema.accessible {
                    return Some(schema);
                }
            }
        }
        None
    }

    /// Resolve `name` for `class`.
    ///
    /// Missing and inaccessible schemas are skipped. Implicit `pg_temp` is used
    /// for relations and types only. Implicit `pg_catalog` is searched before
    /// the explicit path unless the path already names it.
    #[must_use]
    pub fn resolve<'a>(
        &self,
        class: ObjectClass,
        name: &str,
        catalog: &Catalog<'a>,
    ) -> Option<Resolved<'a>> {
        for schema_name in self.lookup_names(class, catalog) {
            let Some(schema) = find(catalog.schemas, &schema_name) else {
                continue;
            };
            if !schema.accessible {
                continue;
            }
            if schema
                .objects
                .iter()
                .any(|(object_class, object_name)| *object_class == class && object_name == name)
            {
                return Some(Resolved {
                    schema: schema.name.as_str(),
                });
            }
        }
        None
    }

    fn explicit_names(&self, catalog: &Catalog<'_>) -> Vec<String> {
        let mut names = Vec::with_capacity(self.tokens.len());
        for token in &self.tokens {
            match token {
                PathToken::User => names.push(catalog.user.to_string()),
                PathToken::Temp => {
                    if let Some(temp) = catalog.temp_schema {
                        names.push(temp.to_string());
                    }
                },
                PathToken::Catalog => names.push("pg_catalog".to_string()),
                PathToken::Schema(name) => names.push(name.clone()),
            }
        }
        names
    }

    fn lookup_names(&self, class: ObjectClass, catalog: &Catalog<'_>) -> Vec<String> {
        let explicit = self.explicit_names(catalog);
        let sees_temp = matches!(class, ObjectClass::Relation | ObjectClass::Type);
        let lists_temp = self.tokens.iter().any(|token| matches!(token, PathToken::Temp));
        let lists_catalog = self.tokens.iter().any(|token| matches!(token, PathToken::Catalog));
        let mut names = Vec::with_capacity(explicit.len() + 2);
        if sees_temp && !lists_temp {
            if let Some(temp) = catalog.temp_schema {
                names.push(temp.to_string());
            }
        }
        if !lists_catalog {
            names.push("pg_catalog".to_string());
        }
        for name in explicit {
            if !sees_temp && catalog.temp_schema == Some(name.as_str()) {
                continue;
            }
            names.push(name);
        }
        names
    }
}

/// Session path plus at most one `SET LOCAL` override.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSearchPath {
    session:  SearchPath,
    local:    Option<SearchPath>,
    revision: u64,
}

impl SessionSearchPath {
    /// Session path with revision 1 and no local override.
    #[must_use]
    pub fn new(session: SearchPath) -> Self {
        Self {
            session,
            local: None,
            revision: 1,
        }
    }

    /// Default PostgreSQL path.
    #[must_use]
    pub fn postgres_default() -> Self {
        Self::new(SearchPath::postgres_default())
    }

    /// Replace the session path. A local override still wins until it is popped.
    pub fn set_session(&mut self, path: SearchPath) {
        self.session = path;
        self.bump();
    }

    /// Install a transaction-local path.
    pub fn set_local(&mut self, path: SearchPath) {
        self.local = Some(path);
        self.bump();
    }

    /// Drop the local path. This is the rollback of `SET LOCAL`.
    pub fn rollback_local(&mut self) {
        if self.local.take().is_some() {
            self.bump();
        }
    }

    /// Path used for lookup.
    #[must_use]
    pub fn effective(&self) -> &SearchPath {
        self.local.as_ref().unwrap_or(&self.session)
    }

    /// Changes whenever the session or local path changes.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// [`SearchPath::current_schema`] on the effective path.
    #[must_use]
    pub fn current_schema<'a>(&self, catalog: &Catalog<'a>) -> Option<&'a str> {
        self.effective().current_schema(catalog)
    }

    /// [`SearchPath::current_stored_key`] on the effective path.
    #[must_use]
    pub fn current_stored_key<'a>(&self, catalog: &Catalog<'a>) -> Option<&'a str> {
        self.effective().current_stored_key(catalog)
    }

    /// [`SearchPath::resolve`] on the effective path.
    #[must_use]
    pub fn resolve<'a>(
        &self,
        class: ObjectClass,
        name: &str,
        catalog: &Catalog<'a>,
    ) -> Option<Resolved<'a>> {
        self.effective().resolve(class, name, catalog)
    }

    fn bump(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
}

impl Default for SessionSearchPath {
    fn default() -> Self {
        Self::postgres_default()
    }
}

/// Revision captured when a prepared statement was bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreparedPlan {
    revision: u64,
}

impl PreparedPlan {
    /// Capture the path revision at bind time.
    #[must_use]
    pub fn capture(paths: &SessionSearchPath) -> Self {
        Self {
            revision: paths.revision(),
        }
    }

    /// A path change since [`Self::capture`] requires the statement to rebind.
    #[must_use]
    pub fn needs_rebind(&self, paths: &SessionSearchPath) -> bool {
        self.revision != paths.revision()
    }
}

fn find<'a>(schemas: &'a [SchemaEntry], name: &str) -> Option<&'a SchemaEntry> {
    schemas.iter().find(|schema| schema.name == name)
}

/// Catalog rows for stored schema keys.
///
/// The stored key `default` is also searchable as `public` while no schema is
/// stored under `public`. Creating a schema whose key is `public` does not
/// retarget `default`.
#[must_use]
pub fn catalog_entries_for_stored_keys(stored_keys: &[(&str, bool)]) -> Vec<SchemaEntry> {
    let has_public_key = stored_keys.iter().any(|(key, _)| *key == "public");
    let mut entries = Vec::with_capacity(stored_keys.len() + 1);
    for (key, accessible) in stored_keys {
        if *key == "default" && !has_public_key {
            entries.push(SchemaEntry {
                name:       "public".to_string(),
                stored_key: "default".to_string(),
                accessible: *accessible,
                objects:    Vec::new(),
            });
        }
        entries.push(SchemaEntry {
            name:       (*key).to_string(),
            stored_key: (*key).to_string(),
            accessible: *accessible,
            objects:    Vec::new(),
        });
    }
    entries
}

fn split_entries(text: &str) -> Result<Vec<&str>, PathError> {
    let mut entries = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    for (index, ch) in text.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                let entry = text[start..index].trim();
                if entry.is_empty() {
                    return Err(PathError::new("empty search_path entry"));
                }
                entries.push(entry);
                start = index + 1;
            },
            _ => {},
        }
    }
    if in_quotes {
        return Err(PathError::new("unclosed identifier quote"));
    }
    let tail = text[start..].trim();
    if tail.is_empty() {
        return Err(PathError::new("empty search_path entry"));
    }
    entries.push(tail);
    Ok(entries)
}

fn parse_token(token: &str) -> Result<PathToken, PathError> {
    if let Some(inner) = unquote(token)? {
        if inner == "$user" {
            return Ok(PathToken::User);
        }
        if inner == "pg_catalog" {
            return Ok(PathToken::Catalog);
        }
        if is_temp_name(&inner) {
            return Ok(PathToken::Temp);
        }
        return Ok(PathToken::Schema(inner));
    }
    if token.eq_ignore_ascii_case("$user") {
        return Ok(PathToken::User);
    }
    let folded = fold_ascii(token);
    if folded == "pg_catalog" {
        return Ok(PathToken::Catalog);
    }
    if is_temp_name(&folded) {
        return Ok(PathToken::Temp);
    }
    if folded.is_empty() || folded.contains('\0') {
        return Err(PathError::new("empty search_path entry"));
    }
    Ok(PathToken::Schema(folded))
}

fn unquote(token: &str) -> Result<Option<String>, PathError> {
    if !token.starts_with('"') {
        return Ok(None);
    }
    let mut chars = token.chars();
    if chars.next() != Some('"') {
        return Err(PathError::new("unclosed identifier quote"));
    }
    let mut out = String::new();
    let mut closed = false;
    while let Some(ch) = chars.next() {
        if ch == '"' {
            if chars.as_str().starts_with('"') {
                chars.next();
                out.push('"');
            } else if chars.as_str().is_empty() {
                closed = true;
            } else {
                return Err(PathError::new("unclosed identifier quote"));
            }
        } else if closed {
            return Err(PathError::new("unclosed identifier quote"));
        } else {
            out.push(ch);
        }
    }
    if !closed || out.is_empty() {
        return Err(PathError::new("unclosed identifier quote"));
    }
    Ok(Some(out))
}

fn is_temp_name(name: &str) -> bool {
    if name == "pg_temp" {
        return true;
    }
    let Some(rest) = name.strip_prefix("pg_temp_") else {
        return false;
    };
    !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_digit())
}

fn fold_ascii(value: &str) -> String {
    let mut folded = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_uppercase() {
            folded.push(ch.to_ascii_lowercase());
        } else {
            folded.push(ch);
        }
    }
    folded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, accessible: bool, objects: &[(ObjectClass, &str)]) -> SchemaEntry {
        SchemaEntry {
            name: name.to_string(),
            stored_key: name.to_string(),
            accessible,
            objects: objects
                .iter()
                .map(|(class, object)| (*class, (*object).to_string()))
                .collect(),
        }
    }

    fn catalog<'a>(
        user: &'a str,
        temp_schema: Option<&'a str>,
        schemas: &'a [SchemaEntry],
    ) -> Catalog<'a> {
        Catalog {
            user,
            temp_schema,
            schemas,
        }
    }

    #[test]
    fn quoted_user_token_matches_the_unquoted_form() {
        assert_eq!(
            SearchPath::parse("\"$user\", public").unwrap(),
            SearchPath::parse("$user, public").unwrap()
        );
        assert_eq!(SearchPath::postgres_default(), SearchPath::parse("\"$user\", public").unwrap());
    }

    #[test]
    fn missing_user_schema_skips_to_public() {
        let path = SearchPath::parse("\"$user\", public").unwrap();
        let schemas = vec![entry("public", true, &[(ObjectClass::Relation, "items")])];
        let view = catalog("alice", None, &schemas);
        assert_eq!(path.current_schema(&view), Some("public"));
        assert_eq!(
            path.resolve(ObjectClass::Relation, "items", &view).map(|hit| hit.schema),
            Some("public")
        );
    }

    #[test]
    fn legacy_default_key_is_visible_as_public_without_replacing_a_public_schema() {
        let path = SearchPath::parse("\"$user\", public").unwrap();
        let legacy = catalog_entries_for_stored_keys(&[("default", true), ("app", true)]);
        let view = catalog("alice", None, &legacy);
        assert_eq!(path.current_schema(&view), Some("public"));
        assert_eq!(path.current_stored_key(&view), Some("default"));

        let both = catalog_entries_for_stored_keys(&[("default", true), ("public", true)]);
        let view = catalog("alice", None, &both);
        assert_eq!(path.current_stored_key(&view), Some("public"));
        let stored_default = SearchPath::parse("default").unwrap();
        assert_eq!(stored_default.current_stored_key(&view), Some("default"));
    }

    #[test]
    fn existing_user_schema_is_current() {
        let path = SearchPath::parse("\"$user\", public").unwrap();
        let schemas = vec![
            entry("alice", true, &[(ObjectClass::Relation, "items")]),
            entry("public", true, &[(ObjectClass::Relation, "other")]),
        ];
        let view = catalog("alice", None, &schemas);
        assert_eq!(path.current_schema(&view), Some("alice"));
        assert_eq!(
            path.resolve(ObjectClass::Relation, "items", &view).map(|hit| hit.schema),
            Some("alice")
        );
    }

    #[test]
    fn current_schema_is_not_the_schema_that_held_the_object() {
        let path = SearchPath::parse("public").unwrap();
        let schemas = vec![
            entry("pg_catalog", true, &[(ObjectClass::Relation, "pg_type")]),
            entry("public", true, &[(ObjectClass::Relation, "items")]),
        ];
        let view = catalog("alice", None, &schemas);
        assert_eq!(
            path.resolve(ObjectClass::Relation, "pg_type", &view).map(|hit| hit.schema),
            Some("pg_catalog")
        );
        assert_eq!(path.current_schema(&view), Some("public"));
    }

    #[test]
    fn implicit_pg_catalog_precedes_the_explicit_path() {
        let path = SearchPath::parse("public").unwrap();
        let schemas = vec![
            entry("pg_catalog", true, &[(ObjectClass::Relation, "shared")]),
            entry("public", true, &[(ObjectClass::Relation, "shared")]),
        ];
        let view = catalog("alice", None, &schemas);
        assert_eq!(
            path.resolve(ObjectClass::Relation, "shared", &view).map(|hit| hit.schema),
            Some("pg_catalog")
        );
    }

    #[test]
    fn explicit_pg_catalog_keeps_its_position() {
        let path = SearchPath::parse("public, pg_catalog").unwrap();
        let schemas = vec![
            entry("pg_catalog", true, &[(ObjectClass::Relation, "shared")]),
            entry("public", true, &[(ObjectClass::Relation, "shared")]),
        ];
        let view = catalog("alice", None, &schemas);
        assert_eq!(
            path.resolve(ObjectClass::Relation, "shared", &view).map(|hit| hit.schema),
            Some("public")
        );
    }

    #[test]
    fn implicit_temp_is_visible_to_relations_and_types_only() {
        let path = SearchPath::parse("public").unwrap();
        let objects = [
            (ObjectClass::Relation, "scratch"),
            (ObjectClass::Type, "scratch"),
            (ObjectClass::Routine, "scratch"),
            (ObjectClass::Operator, "scratch"),
        ];
        let schemas = vec![
            entry("pg_temp_1", true, &objects),
            entry("public", true, &objects),
        ];
        let view = catalog("alice", Some("pg_temp_1"), &schemas);
        assert_eq!(
            path.resolve(ObjectClass::Relation, "scratch", &view).map(|hit| hit.schema),
            Some("pg_temp_1")
        );
        assert_eq!(
            path.resolve(ObjectClass::Type, "scratch", &view).map(|hit| hit.schema),
            Some("pg_temp_1")
        );
        assert_eq!(
            path.resolve(ObjectClass::Routine, "scratch", &view).map(|hit| hit.schema),
            Some("public")
        );
        assert_eq!(
            path.resolve(ObjectClass::Operator, "scratch", &view).map(|hit| hit.schema),
            Some("public")
        );
    }

    #[test]
    fn explicit_temp_is_still_hidden_from_routines() {
        let path = SearchPath::parse("pg_temp, public").unwrap();
        let schemas = vec![
            entry(
                "pg_temp_1",
                true,
                &[
                    (ObjectClass::Routine, "scratch"),
                    (ObjectClass::Relation, "scratch"),
                ],
            ),
            entry("public", true, &[]),
        ];
        let view = catalog("alice", Some("pg_temp_1"), &schemas);
        assert_eq!(path.resolve(ObjectClass::Routine, "scratch", &view), None);
        assert_eq!(
            path.resolve(ObjectClass::Relation, "scratch", &view).map(|hit| hit.schema),
            Some("pg_temp_1")
        );
    }

    #[test]
    fn pg_temp_alias_uses_the_session_temp_schema() {
        let path = SearchPath::parse("pg_temp_9").unwrap();
        let schemas = vec![entry(
            "pg_temp_1",
            true,
            &[(ObjectClass::Relation, "scratch")],
        )];
        let view = catalog("alice", Some("pg_temp_1"), &schemas);
        assert_eq!(path.current_schema(&view), Some("pg_temp_1"));
        assert_eq!(
            path.resolve(ObjectClass::Relation, "scratch", &view).map(|hit| hit.schema),
            Some("pg_temp_1")
        );
        assert_eq!(SearchPath::parse("pg_temp").unwrap(), SearchPath::parse("pg_temp_9").unwrap());
    }

    #[test]
    fn empty_path_has_no_current_schema_and_still_searches_implicit_schemas() {
        let path = SearchPath::parse("").unwrap();
        let schemas = vec![
            entry("pg_temp_1", true, &[(ObjectClass::Relation, "scratch")]),
            entry("pg_catalog", true, &[(ObjectClass::Routine, "now")]),
        ];
        let view = catalog("alice", Some("pg_temp_1"), &schemas);
        assert_eq!(path.current_schema(&view), None);
        assert_eq!(
            path.resolve(ObjectClass::Relation, "scratch", &view).map(|hit| hit.schema),
            Some("pg_temp_1")
        );
        assert_eq!(
            path.resolve(ObjectClass::Routine, "now", &view).map(|hit| hit.schema),
            Some("pg_catalog")
        );
        assert_eq!(path.resolve(ObjectClass::Routine, "scratch", &view), None);
    }

    #[test]
    fn missing_and_inaccessible_entries_are_skipped() {
        let path = SearchPath::parse("missing, secret, public").unwrap();
        let schemas = vec![
            entry("secret", false, &[(ObjectClass::Relation, "items")]),
            entry("public", true, &[(ObjectClass::Relation, "items")]),
        ];
        let view = catalog("alice", None, &schemas);
        assert_eq!(path.current_schema(&view), Some("public"));
        assert_eq!(
            path.resolve(ObjectClass::Relation, "items", &view).map(|hit| hit.schema),
            Some("public")
        );
    }

    #[test]
    fn set_local_rollback_restores_the_session_path() {
        let schemas = vec![
            entry("public", true, &[(ObjectClass::Relation, "items")]),
            entry("app", true, &[(ObjectClass::Relation, "items")]),
        ];
        let view = catalog("alice", None, &schemas);
        let mut paths = SessionSearchPath::postgres_default();
        assert_eq!(paths.current_schema(&view), Some("public"));
        paths.set_local(SearchPath::parse("app").unwrap());
        assert_eq!(
            paths.resolve(ObjectClass::Relation, "items", &view).map(|hit| hit.schema),
            Some("app")
        );
        paths.rollback_local();
        assert_eq!(
            paths.resolve(ObjectClass::Relation, "items", &view).map(|hit| hit.schema),
            Some("public")
        );
    }

    #[test]
    fn security_definer_uses_the_definer_path() {
        let schemas = vec![
            entry("public", true, &[(ObjectClass::Relation, "items")]),
            entry("app", true, &[(ObjectClass::Relation, "items")]),
        ];
        let view = catalog("alice", None, &schemas);
        let caller = SearchPath::parse("public").unwrap();
        let definer = SearchPath::parse("app").unwrap();
        assert_eq!(
            caller.resolve(ObjectClass::Relation, "items", &view).map(|hit| hit.schema),
            Some("public")
        );
        assert_eq!(
            definer.resolve(ObjectClass::Relation, "items", &view).map(|hit| hit.schema),
            Some("app")
        );
    }

    #[test]
    fn prepared_plan_rebinds_when_the_revision_changes() {
        let mut paths = SessionSearchPath::postgres_default();
        let prepared = PreparedPlan::capture(&paths);
        assert!(!prepared.needs_rebind(&paths));
        paths.set_local(SearchPath::parse("app").unwrap());
        assert!(prepared.needs_rebind(&paths));
        paths.rollback_local();
        assert!(prepared.needs_rebind(&paths));
        let rebound = PreparedPlan::capture(&paths);
        assert!(!rebound.needs_rebind(&paths));
    }
}
