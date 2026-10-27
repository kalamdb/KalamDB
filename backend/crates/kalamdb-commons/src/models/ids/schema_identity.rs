//! Stable schema and table identity, separate from the display name.
//!
//! `SchemaId` and `TableId` keep their folded storage keys. These types are the
//! model those keys will point at. Nothing here is written to RocksDB, Raft,
//! protocol messages, config, or clients.

use std::fmt;

const IDENT_MAX_BYTES: usize = 63;
const CODEC_VERSION: u8 = 1;
const LEGACY_TAG: u8 = 1;
const ALLOCATED_TAG: u8 = 2;

/// Failure while parsing an identifier or identity bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentError {
    /// The identifier is empty.
    Empty,
    /// An unquoted identifier has an illegal character.
    InvalidUnquoted,
    /// A quoted identifier is missing its closing quote.
    UnclosedQuote,
    /// The byte string is not a valid identity record.
    InvalidBytes,
}

impl fmt::Display for IdentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Empty => "empty identifier",
            Self::InvalidUnquoted => "invalid unquoted identifier",
            Self::UnclosedQuote => "unclosed identifier quote",
            Self::InvalidBytes => "invalid identity bytes",
        })
    }
}

impl std::error::Error for IdentError {}

/// Canonical SQL identifier.
///
/// Unquoted text folds ASCII letters once. Quoted text keeps case. Both forms
/// truncate to 63 bytes on a UTF-8 boundary, so names that truncate to the same
/// bytes are the same identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SqlIdent {
    canonical: String,
}

impl SqlIdent {
    /// Parse one identifier, quoted or unquoted.
    ///
    /// # Errors
    ///
    /// Returns an error when the text is empty, the quotes are broken, or an
    /// unquoted identifier contains a character PostgreSQL would not accept.
    pub fn parse(input: &str) -> Result<Self, IdentError> {
        let input = input.trim();
        if input.starts_with('"') {
            Self::quoted(&unescape_quoted(input)?)
        } else {
            Self::unquoted(input)
        }
    }

    /// Fold and truncate an unquoted identifier.
    ///
    /// # Errors
    ///
    /// Returns an error when `name` is empty or contains a character that cannot
    /// appear in an unquoted identifier.
    pub fn unquoted(name: &str) -> Result<Self, IdentError> {
        if name.is_empty() || name.contains('\0') {
            return Err(IdentError::Empty);
        }
        let mut chars = name.chars();
        let first = chars.next().ok_or(IdentError::Empty)?;
        if !is_unquoted_start(first) || !chars.all(is_unquoted_continue) {
            return Err(IdentError::InvalidUnquoted);
        }
        Self::from_canonical(&truncate_ident(&fold_ascii(name)))
    }

    /// Keep the case of an already-unescaped quoted identifier and truncate it.
    ///
    /// # Errors
    ///
    /// Returns an error when `content` is empty or contains a null byte.
    pub fn quoted(content: &str) -> Result<Self, IdentError> {
        if content.is_empty() {
            return Err(IdentError::Empty);
        }
        if content.contains('\0') {
            return Err(IdentError::InvalidBytes);
        }
        Self::from_canonical(truncate_ident(content))
    }

    /// Canonical bytes after folding or quote removal.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.canonical
    }

    fn from_canonical(canonical: &str) -> Result<Self, IdentError> {
        if canonical.is_empty() {
            return Err(IdentError::Empty);
        }
        if canonical.len() > IDENT_MAX_BYTES || canonical.contains('\0') {
            return Err(IdentError::InvalidBytes);
        }
        Ok(Self {
            canonical: canonical.to_string(),
        })
    }
}

impl fmt::Display for SqlIdent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if needs_quotes(&self.canonical) {
            f.write_str("\"")?;
            for ch in self.canonical.chars() {
                if ch == '"' {
                    f.write_str("\"\"")?;
                } else {
                    write!(f, "{ch}")?;
                }
            }
            f.write_str("\"")
        } else {
            f.write_str(&self.canonical)
        }
    }
}

/// Counter for identities that are not derived from a name.
#[derive(Debug)]
pub struct IdentitySource {
    next: u128,
}

impl IdentitySource {
    /// Start at 1. Zero is unused so a zeroed buffer cannot look allocated.
    #[must_use]
    pub fn new() -> Self {
        Self { next: 1 }
    }

    fn next_id(&mut self) -> u128 {
        let id = self.next;
        self.next = self.next.saturating_add(1);
        id
    }
}

impl Default for IdentitySource {
    fn default() -> Self {
        Self::new()
    }
}

/// Schema identity. Legacy values are the stored namespace key, not the display name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SchemaIdentity {
    /// Existing RocksDB namespace primary key.
    Legacy(String),
    /// Identity allocated independently of the name.
    Allocated(u128),
}

impl SchemaIdentity {
    /// Stored namespace key when this identity came from the current catalog.
    #[must_use]
    pub fn legacy_stored_key(&self) -> Option<&str> {
        match self {
            Self::Legacy(key) => Some(key.as_str()),
            Self::Allocated(_) => None,
        }
    }

    /// Hex encoding safe to place in a filesystem path.
    #[must_use]
    pub fn encoded_identity(&self) -> String {
        match self {
            Self::Legacy(key) => format!("sch-{}", hex::encode(key.as_bytes())),
            Self::Allocated(id) => format!("sch-{}", hex::encode(id.to_be_bytes())),
        }
    }
}

/// One schema: a stable identity plus a quoted-aware name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaRecord {
    identity: SchemaIdentity,
    name: SqlIdent,
}

impl SchemaRecord {
    /// Allocate a schema. The name is not part of the identity.
    #[must_use]
    pub fn create(source: &mut IdentitySource, name: SqlIdent) -> Self {
        Self {
            identity: SchemaIdentity::Allocated(source.next_id()),
            name,
        }
    }

    /// Map an existing stored namespace key to a schema without minting a new one.
    ///
    /// # Errors
    ///
    /// Returns an error when `stored_key` is empty or contains a null byte.
    pub fn legacy(stored_key: &str, name: SqlIdent) -> Result<Self, IdentError> {
        if stored_key.is_empty() || stored_key.contains('\0') {
            return Err(IdentError::InvalidBytes);
        }
        Ok(Self {
            identity: SchemaIdentity::Legacy(stored_key.to_string()),
            name,
        })
    }

    /// The stored `default` namespace, displayed as `public`, without merging the two names.
    ///
    /// # Panics
    ///
    /// Panics if `public` is ever rejected as an unquoted identifier. That is a bug.
    #[must_use]
    pub fn legacy_default() -> Self {
        Self::legacy("default", SqlIdent::unquoted("public").expect("public is a valid identifier"))
            .expect("default is a valid stored key")
    }

    /// Stable identity.
    #[must_use]
    pub fn identity(&self) -> &SchemaIdentity {
        &self.identity
    }

    /// Display name.
    #[must_use]
    pub fn name(&self) -> &SqlIdent {
        &self.name
    }

    /// Stored namespace key, when this record still uses one.
    #[must_use]
    pub fn stored_key(&self) -> Option<&str> {
        self.identity.legacy_stored_key()
    }

    /// Change the display name. The identity and any legacy key stay put.
    pub fn rename(&mut self, name: SqlIdent) {
        self.name = name;
    }

    /// Filesystem-safe encoding of the identity. Filestore owns real path templates.
    #[must_use]
    pub fn encoded_identity(&self) -> String {
        self.identity.encoded_identity()
    }

    /// Proof encoding for a restart. This is not the RocksDB catalog payload.
    ///
    /// # Errors
    ///
    /// Returns an error when a component does not fit the length prefix.
    pub fn to_bytes(&self) -> Result<Vec<u8>, IdentError> {
        let mut buf = Vec::with_capacity(64);
        buf.push(CODEC_VERSION);
        write_schema_identity(&mut buf, &self.identity)?;
        write_str(&mut buf, self.name.as_str())?;
        Ok(buf)
    }

    /// Read [`Self::to_bytes`].
    ///
    /// # Errors
    ///
    /// Returns an error when `bytes` is truncated, tagged incorrectly, or has trailing data.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, IdentError> {
        let mut cursor = Cursor { rest: bytes };
        if cursor.u8()? != CODEC_VERSION {
            return Err(IdentError::InvalidBytes);
        }
        let identity = read_schema_identity(&mut cursor)?;
        let name = SqlIdent::from_canonical(cursor.str()?)?;
        cursor.finish()?;
        Ok(Self { identity, name })
    }
}

/// How an ordinary table is stored. Shared is the PostgreSQL default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TableKind {
    /// One table visible according to schema privileges.
    Shared,
    /// Per-user storage, only when `WITH (TYPE = 'USER')` says so.
    User,
    /// Stream storage, only when `WITH (TYPE = 'STREAM')` says so.
    Stream,
}

/// Table identity. A legacy value is the current `TableId` key, not the display name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TableIdentity {
    /// Existing `(namespace, table)` storage key.
    Legacy {
        namespace_key: String,
        table_key: String,
    },
    /// Identity allocated independently of the name and schema.
    Allocated(u128),
}

impl TableIdentity {
    /// Hex encoding safe to place in a filesystem path.
    #[must_use]
    pub fn encoded_identity(&self) -> String {
        match self {
            Self::Legacy {
                namespace_key,
                table_key,
            } => format!(
                "tbl-{}-{}",
                hex::encode(namespace_key.as_bytes()),
                hex::encode(table_key.as_bytes())
            ),
            Self::Allocated(id) => format!("tbl-{}", hex::encode(id.to_be_bytes())),
        }
    }
}

/// One table object. `SET SCHEMA` changes [`Self::schema`], not [`Self::identity`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableRecord {
    identity: TableIdentity,
    schema: SchemaIdentity,
    name: SqlIdent,
    kind: TableKind,
}

impl TableRecord {
    /// Allocate a table in `schema`.
    #[must_use]
    pub fn new(
        source: &mut IdentitySource,
        schema: SchemaIdentity,
        name: SqlIdent,
        kind: TableKind,
    ) -> Self {
        Self {
            identity: TableIdentity::Allocated(source.next_id()),
            schema,
            name,
            kind,
        }
    }

    /// Ordinary `CREATE TABLE`: shared storage, not a per-user table.
    #[must_use]
    pub fn ordinary(source: &mut IdentitySource, schema: SchemaIdentity, name: SqlIdent) -> Self {
        Self::new(source, schema, name, TableKind::Shared)
    }

    /// Map an existing table storage key without rewriting its prefix.
    ///
    /// # Errors
    ///
    /// Returns an error when either key is empty or contains a null byte.
    pub fn legacy(
        namespace_key: &str,
        table_key: &str,
        kind: TableKind,
    ) -> Result<Self, IdentError> {
        if namespace_key.is_empty()
            || table_key.is_empty()
            || namespace_key.contains('\0')
            || table_key.contains('\0')
        {
            return Err(IdentError::InvalidBytes);
        }
        let name = SqlIdent::from_canonical(truncate_ident(&fold_ascii(table_key)))?;
        Ok(Self {
            identity: TableIdentity::Legacy {
                namespace_key: namespace_key.to_string(),
                table_key: table_key.to_string(),
            },
            schema: SchemaIdentity::Legacy(namespace_key.to_string()),
            name,
            kind,
        })
    }

    /// Stable table identity.
    #[must_use]
    pub fn identity(&self) -> &TableIdentity {
        &self.identity
    }

    /// Schema that currently owns the table in the catalog.
    #[must_use]
    pub fn schema(&self) -> &SchemaIdentity {
        &self.schema
    }

    /// Display name.
    #[must_use]
    pub fn name(&self) -> &SqlIdent {
        &self.name
    }

    /// Storage kind.
    #[must_use]
    pub fn kind(&self) -> TableKind {
        self.kind
    }

    /// Change the display name. The identity and encoded prefix stay put.
    pub fn rename(&mut self, name: SqlIdent) {
        self.name = name;
    }

    /// Point the catalog object at another schema. Row prefixes stay on the identity.
    pub fn set_schema(&mut self, schema: SchemaIdentity) {
        self.schema = schema;
    }

    /// Filesystem-safe encoding of the table identity.
    #[must_use]
    pub fn encoded_identity(&self) -> String {
        self.identity.encoded_identity()
    }

    /// Proof encoding for a restart. This is not the RocksDB table key.
    ///
    /// # Errors
    ///
    /// Returns an error when a component does not fit the length prefix.
    pub fn to_bytes(&self) -> Result<Vec<u8>, IdentError> {
        let mut buf = Vec::with_capacity(96);
        buf.push(CODEC_VERSION);
        write_table_identity(&mut buf, &self.identity)?;
        write_schema_identity(&mut buf, &self.schema)?;
        buf.push(match self.kind {
            TableKind::Shared => 1,
            TableKind::User => 2,
            TableKind::Stream => 3,
        });
        write_str(&mut buf, self.name.as_str())?;
        Ok(buf)
    }

    /// Read [`Self::to_bytes`].
    ///
    /// # Errors
    ///
    /// Returns an error when `bytes` is truncated, tagged incorrectly, or has trailing data.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, IdentError> {
        let mut cursor = Cursor { rest: bytes };
        if cursor.u8()? != CODEC_VERSION {
            return Err(IdentError::InvalidBytes);
        }
        let identity = read_table_identity(&mut cursor)?;
        let schema = read_schema_identity(&mut cursor)?;
        let kind = match cursor.u8()? {
            1 => TableKind::Shared,
            2 => TableKind::User,
            3 => TableKind::Stream,
            _ => return Err(IdentError::InvalidBytes),
        };
        let name = SqlIdent::from_canonical(cursor.str()?)?;
        cursor.finish()?;
        Ok(Self {
            identity,
            schema,
            name,
            kind,
        })
    }
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

fn truncate_ident(value: &str) -> &str {
    if value.len() <= IDENT_MAX_BYTES {
        return value;
    }
    let mut end = IDENT_MAX_BYTES;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn is_unquoted_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_' || is_unicode_ident(ch)
}

fn is_unquoted_continue(ch: char) -> bool {
    is_unquoted_start(ch) || ch.is_ascii_digit() || ch == '$'
}

fn is_unicode_ident(ch: char) -> bool {
    !ch.is_ascii() && !ch.is_control() && !ch.is_whitespace()
}

fn needs_quotes(canonical: &str) -> bool {
    let mut chars = canonical.chars();
    let Some(first) = chars.next() else {
        return true;
    };
    if !is_unquoted_start(first) || !chars.all(is_unquoted_continue) {
        return true;
    }
    canonical.bytes().any(|byte| byte.is_ascii_uppercase())
}

fn unescape_quoted(input: &str) -> Result<String, IdentError> {
    let mut chars = input.chars();
    if chars.next() != Some('"') {
        return Err(IdentError::UnclosedQuote);
    }
    let mut out = String::new();
    let mut closed = false;
    while let Some(ch) = chars.next() {
        if ch == '\0' {
            return Err(IdentError::InvalidBytes);
        }
        if ch == '"' {
            if chars.as_str().starts_with('"') {
                chars.next();
                out.push('"');
            } else if chars.as_str().is_empty() {
                closed = true;
            } else {
                return Err(IdentError::UnclosedQuote);
            }
        } else if closed {
            return Err(IdentError::UnclosedQuote);
        } else {
            out.push(ch);
        }
    }
    if !closed {
        return Err(IdentError::UnclosedQuote);
    }
    if out.is_empty() {
        return Err(IdentError::Empty);
    }
    Ok(out)
}

fn write_str(buf: &mut Vec<u8>, value: &str) -> Result<(), IdentError> {
    let len = u16::try_from(value.len()).map_err(|_| IdentError::InvalidBytes)?;
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(value.as_bytes());
    Ok(())
}

fn write_schema_identity(buf: &mut Vec<u8>, identity: &SchemaIdentity) -> Result<(), IdentError> {
    match identity {
        SchemaIdentity::Legacy(key) => {
            buf.push(LEGACY_TAG);
            write_str(buf, key)?;
        },
        SchemaIdentity::Allocated(id) => {
            buf.push(ALLOCATED_TAG);
            buf.extend_from_slice(&id.to_be_bytes());
        },
    }
    Ok(())
}

fn write_table_identity(buf: &mut Vec<u8>, identity: &TableIdentity) -> Result<(), IdentError> {
    match identity {
        TableIdentity::Legacy {
            namespace_key,
            table_key,
        } => {
            buf.push(LEGACY_TAG);
            write_str(buf, namespace_key)?;
            write_str(buf, table_key)?;
        },
        TableIdentity::Allocated(id) => {
            buf.push(ALLOCATED_TAG);
            buf.extend_from_slice(&id.to_be_bytes());
        },
    }
    Ok(())
}

fn read_schema_identity(cursor: &mut Cursor<'_>) -> Result<SchemaIdentity, IdentError> {
    match cursor.u8()? {
        LEGACY_TAG => Ok(SchemaIdentity::Legacy(cursor.str()?.to_string())),
        ALLOCATED_TAG => {
            let bytes = cursor.bytes(16)?;
            let mut id = [0_u8; 16];
            id.copy_from_slice(bytes);
            Ok(SchemaIdentity::Allocated(u128::from_be_bytes(id)))
        },
        _ => Err(IdentError::InvalidBytes),
    }
}

fn read_table_identity(cursor: &mut Cursor<'_>) -> Result<TableIdentity, IdentError> {
    match cursor.u8()? {
        LEGACY_TAG => Ok(TableIdentity::Legacy {
            namespace_key: cursor.str()?.to_string(),
            table_key: cursor.str()?.to_string(),
        }),
        ALLOCATED_TAG => {
            let bytes = cursor.bytes(16)?;
            let mut id = [0_u8; 16];
            id.copy_from_slice(bytes);
            Ok(TableIdentity::Allocated(u128::from_be_bytes(id)))
        },
        _ => Err(IdentError::InvalidBytes),
    }
}

struct Cursor<'a> {
    rest: &'a [u8],
}

impl<'a> Cursor<'a> {
    fn u8(&mut self) -> Result<u8, IdentError> {
        let (first, rest) = self.rest.split_first().ok_or(IdentError::InvalidBytes)?;
        self.rest = rest;
        Ok(*first)
    }

    fn bytes(&mut self, len: usize) -> Result<&'a [u8], IdentError> {
        if self.rest.len() < len {
            return Err(IdentError::InvalidBytes);
        }
        let (head, tail) = self.rest.split_at(len);
        self.rest = tail;
        Ok(head)
    }

    fn str(&mut self) -> Result<&'a str, IdentError> {
        let len_bytes = self.bytes(2)?;
        let len = u16::from_be_bytes([len_bytes[0], len_bytes[1]]) as usize;
        let bytes = self.bytes(len)?;
        std::str::from_utf8(bytes).map_err(|_| IdentError::InvalidBytes)
    }

    fn finish(self) -> Result<(), IdentError> {
        if self.rest.is_empty() {
            Ok(())
        } else {
            Err(IdentError::InvalidBytes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unquoted_names_fold_and_quoted_names_keep_case() {
        assert_eq!(SqlIdent::unquoted("a").unwrap(), SqlIdent::unquoted("A").unwrap());
        assert_eq!(SqlIdent::unquoted("A").unwrap(), SqlIdent::quoted("a").unwrap());
        assert_ne!(SqlIdent::quoted("a").unwrap(), SqlIdent::quoted("A").unwrap());
        assert_ne!(SqlIdent::unquoted("A").unwrap(), SqlIdent::quoted("A").unwrap());
    }

    #[test]
    fn escaped_quotes_and_unicode_round_trip() {
        let escaped = SqlIdent::parse("\"a\"\"b\"").unwrap();
        assert_eq!(escaped.as_str(), "a\"b");
        assert_eq!(escaped, SqlIdent::quoted("a\"b").unwrap());
        assert_eq!(SqlIdent::quoted("café").unwrap().as_str(), "café");
        assert_eq!(SqlIdent::unquoted("CAFÉ").unwrap().as_str(), "cafÉ");
        assert_eq!(SqlIdent::unquoted("CAFÉ").unwrap(), SqlIdent::quoted("cafÉ").unwrap());
        assert_ne!(SqlIdent::unquoted("CAFÉ").unwrap(), SqlIdent::quoted("CAFÉ").unwrap());
    }

    #[test]
    fn truncation_collides_on_a_utf8_boundary() {
        let long = "a".repeat(64);
        let short = "a".repeat(63);
        assert_eq!(SqlIdent::unquoted(&long).unwrap(), SqlIdent::unquoted(&short).unwrap());
        assert_eq!(SqlIdent::unquoted(&short).unwrap().as_str().len(), 63);

        let accent = "é";
        assert_eq!(accent.len(), 2);
        assert_eq!(
            SqlIdent::unquoted(&accent.repeat(32)).unwrap(),
            SqlIdent::unquoted(&accent.repeat(31)).unwrap()
        );
        assert_eq!(SqlIdent::unquoted(&accent.repeat(32)).unwrap().as_str().len(), 62);

        let emoji = "😀";
        assert_eq!(emoji.len(), 4);
        assert_eq!(
            SqlIdent::quoted(&emoji.repeat(16)).unwrap(),
            SqlIdent::quoted(&emoji.repeat(15)).unwrap()
        );
        assert_eq!(SqlIdent::quoted(&emoji.repeat(16)).unwrap().as_str().len(), 60);
    }

    #[test]
    fn quoted_names_are_not_filename_restricted() {
        assert!(SqlIdent::quoted("../etc/café").is_ok());
        assert!(SqlIdent::unquoted("../etc").is_err());
        let mut source = IdentitySource::new();
        let schema = SchemaRecord::create(&mut source, SqlIdent::quoted("../etc/café").unwrap());
        let encoded = schema.encoded_identity();
        assert!(encoded.starts_with("sch-"));
        assert!(!encoded.contains('/'));
        assert!(!encoded.contains('\\'));
        assert!(!encoded.contains(".."));
        assert!(!encoded.contains('é'));
        assert!(encoded.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'));
    }

    #[test]
    fn legacy_default_displays_as_public_and_does_not_merge() {
        let legacy = SchemaRecord::legacy_default();
        assert_eq!(legacy.stored_key(), Some("default"));
        assert_eq!(legacy.name().as_str(), "public");
        let mut source = IdentitySource::new();
        let created = SchemaRecord::create(&mut source, SqlIdent::unquoted("public").unwrap());
        assert_ne!(legacy.identity(), created.identity());
        assert!(created.stored_key().is_none());
    }

    #[test]
    fn rename_keeps_schema_identity_and_encoding() {
        let mut legacy = SchemaRecord::legacy_default();
        let identity = legacy.identity().clone();
        let encoded = legacy.encoded_identity();
        legacy.rename(SqlIdent::quoted("Application").unwrap());
        assert_eq!(legacy.identity(), &identity);
        assert_eq!(legacy.stored_key(), Some("default"));
        assert_eq!(legacy.encoded_identity(), encoded);
        assert_eq!(legacy.name().as_str(), "Application");
        let restored = SchemaRecord::from_bytes(&legacy.to_bytes().unwrap()).unwrap();
        assert_eq!(restored, legacy);
    }

    #[test]
    fn table_move_and_rename_keep_the_row_prefix() {
        let mut source = IdentitySource::new();
        let schema_a = SchemaRecord::create(&mut source, SqlIdent::unquoted("app").unwrap());
        let schema_b = SchemaRecord::create(&mut source, SqlIdent::unquoted("other").unwrap());
        let mut table = TableRecord::ordinary(
            &mut source,
            schema_a.identity().clone(),
            SqlIdent::unquoted("items").unwrap(),
        );
        assert_eq!(table.kind(), TableKind::Shared);
        let identity = table.identity().clone();
        let encoded = table.encoded_identity();
        table.rename(SqlIdent::quoted("Items").unwrap());
        table.set_schema(schema_b.identity().clone());
        assert_eq!(table.identity(), &identity);
        assert_eq!(table.encoded_identity(), encoded);
        assert_eq!(table.schema(), schema_b.identity());
        assert_eq!(table.name().as_str(), "Items");
        let restored = TableRecord::from_bytes(&table.to_bytes().unwrap()).unwrap();
        assert_eq!(restored, table);

        let mut legacy = TableRecord::legacy("default", "items", TableKind::Shared).unwrap();
        let legacy_encoded = legacy.encoded_identity();
        legacy.set_schema(schema_b.identity().clone());
        legacy.rename(SqlIdent::quoted("Items").unwrap());
        assert_eq!(legacy.encoded_identity(), legacy_encoded);
        assert!(!legacy.encoded_identity().contains('/'));
        assert_eq!(
            TableRecord::from_bytes(&legacy.to_bytes().unwrap()).unwrap().identity(),
            legacy.identity()
        );
    }
}
