# ADR-012: sqlparser-rs Integration for SQL Parsing

**Date**: 2025-10-23  
**Status**: Accepted  
**Context**: Phase 2a - User Story 14 (004-system-improvements-and)

## Context

KalamDB initially implemented custom SQL parsers for all DDL operations (CREATE TABLE, CREATE NAMESPACE, etc.). This approach led to:

1. **Code Duplication**: Similar parsing logic repeated across multiple files (280+ lines of duplicate code)
2. **Maintenance Burden**: Changes to SQL syntax required updating multiple parsers
3. **Limited Compatibility**: Custom parsers didn't support PostgreSQL/MySQL syntax variations
4. **Testing Overhead**: Each custom parser needed independent test coverage
5. **Parser Bugs**: Edge cases in custom parsers caused subtle parsing errors

### Example of Duplication

Before consolidation, parsing CREATE TABLE syntax existed in 3 different files:
- `kalamdb-core/src/ddl/create_user_table.rs`
- `kalamdb-core/src/ddl/create_shared_table.rs`
- `kalamdb-core/src/ddl/create_stream_table.rs`

Each file had similar logic for:
- Tokenizing column definitions
- Parsing data types
- Extracting constraints (NOT NULL, PRIMARY KEY)
- Handling syntax errors

### Compatibility Requirements

Users familiar with PostgreSQL or MySQL expect:
- PostgreSQL type aliases (`SERIAL`, `INT4`, `VARCHAR`)
- MySQL type aliases (`MEDIUMINT`, `UNSIGNED INT`, `AUTO_INCREMENT`)
- Standard SQL INSERT/SELECT/UPDATE syntax
- Familiar error messages

Custom parsers couldn't easily support these variations without exponential code growth.

## Decision

We will use **sqlparser-rs** (https://github.com/sqlparser-rs/sqlparser-rs) as the foundation for SQL parsing with **custom extensions** for KalamDB-specific commands.

### Architecture

```
┌─────────────────────────────────────────────────┐
│              SQL Query String                    │
└─────────────────────────────────────────────────┘
                     │
                     ▼
         ┌───────────────────────┐
         │  Statement Detection   │
         │  (keyword matching)    │
         └───────────────────────┘
                     │
         ┌───────────┴───────────┐
         ▼                       ▼
┌─────────────────┐    ┌─────────────────────┐
│  Standard SQL   │    │  KalamDB-Specific   │
│  (sqlparser-rs) │    │  (Custom Parsers)   │
└─────────────────┘    └─────────────────────┘
         │                       │
         │  SELECT, INSERT,      │  CREATE NAMESPACE,
         │  UPDATE, DELETE,      │  FLUSH POLICY,
         │  CREATE TABLE         │  CREATE STORAGE,
         │  (structure only)     │  BACKUP/RESTORE
         │                       │
         └───────────┬───────────┘
                     ▼
         ┌───────────────────────┐
         │   Execution Logic     │
         │   (kalamdb-core)      │
         └───────────────────────┘
```

### Parsing Strategy

1. **Standard SQL Commands** (SELECT, INSERT, UPDATE, DELETE):
   - Parsed once by `KalamParser` using `KalamDbDialect` (`derive_dialect!(KalamDbDialect, PostgreSqlDialect, preserve_type_id = true)`)
   - Planned directly from the parsed `sqlparser::ast::Statement` by DataFusion via `SessionState::statement_to_plan(Statement::Statement(Box::new(ast)))` (never reparsed from text via `SessionContext.sql()`)
   - Parameter and result metadata in `kalamdb-postgres-wire` share one `statement_to_plan` invocation (`SqlExecutor::logical_plan_for_metadata`)

2. **Standard DDL with Extensions** (CREATE TABLE, CREATE TYPE, CREATE DOMAIN, CREATE FUNCTION/PROCEDURE):
   - Parsed once through the shared `KalamParser` token/AST boundary
   - KalamDB-specific extensions (`WITH (TYPE='USER'|'SHARED'|'STREAM')`, `FLUSH POLICY`, etc.) are extracted from the shared AST/token stream without reparsing user SQL text

3. **KalamDB-Specific Commands**:
   - Parsed via `KalamParser` into `StatementPayload::Kalam` or `StatementPayload::PostgresCompat` on the same token cursor
   - Examples: `CREATE STORAGE`, `STORAGE FLUSH`, `BACKUP/RESTORE`, `SUBSCRIBE TO`

### Custom Dialect Extension

KalamDB derives its dialect from `sqlparser::dialect::PostgreSqlDialect` with `preserve_type_id = true`:

```rust
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser_derive::derive_dialect;

derive_dialect!(KalamDbDialect, PostgreSqlDialect, preserve_type_id = true);
```

### Data Type Mapping

PostgreSQL and MySQL type aliases are mapped to Arrow types via `kalamdb-dialect/src/compatibility.rs`:

```rust
pub fn map_sql_type_to_arrow(sql_type: &SQLDataType) -> Result<DataType> {
    match sql_type {
        // PostgreSQL types
        SQLDataType::Custom(name) if name == "SERIAL" => Ok(DataType::Int32),
        SQLDataType::Custom(name) if name == "BIGSERIAL" => Ok(DataType::Int64),
        SQLDataType::Int4(_) => Ok(DataType::Int32),
        SQLDataType::Int8(_) => Ok(DataType::Int64),
        
        // MySQL types
        SQLDataType::MediumInt(_) => Ok(DataType::Int32),
        SQLDataType::UnsignedInt(_) => Ok(DataType::UInt32),
        
        // Standard types
        SQLDataType::Int(_) => Ok(DataType::Int32),
        SQLDataType::BigInt(_) => Ok(DataType::Int64),
        _ => Err(format!("Unsupported type: {:?}", sql_type))
    }
}
```

## Consequences

### Positive

1. **Reduced Code Duplication**: 280+ lines of duplicate parsing code eliminated
2. **Better Compatibility**: PostgreSQL/MySQL syntax supported out-of-box
3. **Improved Testing**: sqlparser-rs has comprehensive test coverage
4. **Future-Proof**: Easy to add new standard SQL features
5. **Familiar Error Messages**: sqlparser-rs produces standard SQL error messages
6. **Maintainability**: One parser to maintain for standard SQL

### Negative

1. **Dependency**: External dependency on sqlparser-rs (acceptable trade-off)
2. **Learning Curve**: Team must understand sqlparser-rs AST structure
3. **Hybrid Complexity**: Two parsing systems (standard + custom) to maintain
4. **Limited Control**: Cannot customize sqlparser-rs error messages easily
5. **Version Lock-in**: Upgrading sqlparser-rs may require code changes

### Mitigation Strategies

1. **Clear Boundaries**: Document which commands use which parser
2. **Wrapper Functions**: Isolate sqlparser-rs usage in adapter modules
3. **Custom Error Mapping**: Convert sqlparser errors to KalamDB error types
4. **Version Pinning**: Pin sqlparser-rs version, test upgrades carefully
5. **Hybrid Tests**: Test both sqlparser and custom parser paths

## Implementation

### Completed (Phase 2a)

- [X] Add sqlparser-rs dependency to kalamdb-dialect
- [X] Create compatibility module for type mapping
- [X] Migrate CREATE TABLE parsing to use sqlparser-rs for structure
- [X] Keep custom parsers for KalamDB extensions (FLUSH POLICY, etc.)
- [X] Add PostgreSQL data type alias support (SERIAL, INT4, etc.)
- [X] Add MySQL data type alias support (MEDIUMINT, UNSIGNED INT, etc.)
- [X] Consolidate duplicate parsing code (280 lines eliminated)
- [X] Create error formatting functions (PostgreSQL/MySQL styles)

### Parsing Modules (kalamdb-dialect Crate)

```
kalamdb-dialect/
├── src/
│   ├── lib.rs                    # Public exports
│   ├── compatibility.rs          # Type mapping + error formatting
│   ├── classifier/
│   │   ├── mod.rs                # Classifier exports
│   │   ├── types.rs              # SqlStatement / SqlStatementKind
│   │   └── engine/core.rs        # Statement classification logic
│   ├── parser/
│   │   ├── mod.rs                # Parser module
│   │   ├── extensions.rs         # KalamDB-specific parser hooks
│   │   ├── query_parser.rs       # Query parser helpers
│   │   └── utils.rs              # Shared parser utilities
│   ├── ddl.rs                    # DDL exports
│   ├── ddl/
│   │   ├── create_namespace.rs   # CREATE NAMESPACE
│   │   ├── create_table/
│   │   │   ├── parser.rs         # CREATE TABLE parser
│   │   │   └── types.rs          # CREATE TABLE helpers
│   │   ├── storage_commands.rs   # CREATE/ALTER/DROP/SHOW STORAGE
│   │   └── ...
│   ├── batch_execution.rs        # Multi-statement parsing helpers
│   └── validation.rs             # Identifier validation helpers
```

### Custom vs Standard SQL Decision Tree

```
                    SQL Statement
                         │
                         ▼
              ┌──────────┴──────────┐
              │  Keyword Detection   │
              └──────────┬──────────┘
                         │
         ┌───────────────┼───────────────┐
         │               │               │
         ▼               ▼               ▼
   ┌─────────┐    ┌──────────┐    ┌──────────┐
   │ SELECT  │    │  CREATE  │    │  CREATE  │
   │ INSERT  │    │  TABLE   │    │ NAMESPACE│
   │ UPDATE  │    │  (hybrid)│    │ (custom) │
   │ DELETE  │    └──────────┘    └──────────┘
   └─────────┘         │                │
        │              ▼                ▼
        │    ┌──────────────┐   ┌──────────────┐
        ▼    │ sqlparser-rs │   │ Custom Parser│
   ┌─────────────────┐      │   │  (regex/nom) │
   │  sqlparser-rs   │      │   └──────────────┘
   │  (full parse)   │      │
   └─────────────────┘      │
                            ▼
                   ┌────────────────┐
                   │ Extract FLUSH  │
                   │ POLICY, etc.   │
                   │ (custom code)  │
                   └────────────────┘
```

### Error Formatting

Both PostgreSQL and MySQL error formats are available:

```rust
use kalamdb_dialect::compatibility::{
    format_postgres_table_not_found,
    format_mysql_table_not_found,
    ErrorStyle,
};

// Choose error style at runtime
let style = ErrorStyle::PostgreSQL;  // or ErrorStyle::MySQL

match style {
    ErrorStyle::PostgreSQL => {
        format_postgres_table_not_found("users")
        // "ERROR: relation \"users\" does not exist"
    }
    ErrorStyle::MySQL => {
        format_mysql_table_not_found("mydb", "users")
        // "ERROR 1146 (42S02): Table 'mydb.users' doesn't exist"
    }
}
```

## Future Enhancements

### Phase 3: Full sqlparser-rs Integration

- [ ] Migrate UPDATE/DELETE parsing to sqlparser-rs
- [ ] Use sqlparser-rs for INSERT multi-row syntax
- [ ] Leverage sqlparser-rs for complex WHERE clause parsing
- [ ] Add CASE expression support via sqlparser-rs

### Phase 4: Advanced Features

- [ ] Support PostgreSQL-style CTEs (WITH queries)
- [ ] Support MySQL-style REPLACE INTO
- [ ] Add sqlparser-rs-based view support (CREATE VIEW)
- [ ] Implement PostgreSQL dollar-quoted strings

### Phase 5: Custom Dialect Registration

- [ ] Register KalamDB as official sqlparser-rs dialect
- [ ] Contribute FLUSH POLICY syntax upstream (as custom extension example)
- [ ] Document KalamDB dialect in sqlparser-rs examples

## Alternatives Considered

### 1. Fully Custom Parser (nom/pest)

**Description**: Build entire SQL parser from scratch using parser combinator libraries.

**Rejected because**:
- Huge development effort (months of work)
- Reinventing the wheel for standard SQL
- PostgreSQL/MySQL compatibility would require replicating their parsers
- No ecosystem benefits (testing, documentation, community fixes)

**Estimated Effort**: 3-6 months of development + ongoing maintenance

### 2. PostgreSQL libpq Parser Bindings

**Description**: Use FFI bindings to PostgreSQL's native C parser.

**Rejected because**:
- Tight coupling to PostgreSQL version
- Complex C FFI integration
- Difficult to extend for KalamDB-specific syntax
- Platform compatibility issues (Windows support)
- Large binary size increase

### 3. Multiple Parser Libraries

**Description**: Use different parsers for different SQL dialects (pg_query for PostgreSQL, mysql_parser for MySQL).

**Rejected because**:
- Maintenance nightmare (multiple parser APIs)
- Inconsistent AST structures
- Difficult to support cross-dialect features
- Increased binary size
- Complex conditional compilation

### 4. DataFusion's SQL Parser Only

**Description**: Rely solely on DataFusion's built-in SQL parsing.

**Rejected because**:
- DataFusion parser is tightly coupled to its execution engine
- Limited support for DDL operations
- No support for KalamDB-specific extensions
- Difficult to extract parsed AST without executing

## Testing Strategy

### sqlparser-rs Integration Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use sqlparser::parser::Parser;
    
    #[test]
    fn test_postgres_serial_type() {
        let sql = "CREATE TABLE users (id SERIAL PRIMARY KEY)";
        let dialect = KalamDbDialect::new();
        let ast = Parser::parse_sql(&dialect, sql).unwrap();
        
        // Verify sqlparser-rs parsed SERIAL correctly
        // Then map to KalamDB types
    }
    
    #[test]
    fn test_mysql_unsigned_int() {
        let sql = "CREATE TABLE users (id INT UNSIGNED)";
        let dialect = KalamDbDialect::new();
        let ast = Parser::parse_sql(&dialect, sql).unwrap();
        
        // Verify UNSIGNED INT mapping
    }
}
```

### Compatibility Tests

```rust
#[test]
fn test_postgres_compatibility() {
    // Test all PostgreSQL type aliases
    assert_eq!(map_sql_type("SERIAL"), DataType::Int32);
    assert_eq!(map_sql_type("BIGSERIAL"), DataType::Int64);
    assert_eq!(map_sql_type("INT4"), DataType::Int32);
}

#[test]
fn test_mysql_compatibility() {
    // Test all MySQL type aliases
    assert_eq!(map_sql_type("MEDIUMINT"), DataType::Int32);
    assert_eq!(map_sql_type("UNSIGNED INT"), DataType::UInt32);
}
```

## References

- [sqlparser-rs GitHub](https://github.com/sqlparser-rs/sqlparser-rs)
- [sqlparser-rs Documentation](https://docs.rs/sqlparser/)
- [PostgreSQL SQL Syntax](https://www.postgresql.org/docs/current/sql.html)
- [MySQL SQL Syntax](https://dev.mysql.com/doc/refman/8.0/en/sql-syntax.html)
- [Apache Arrow Data Types](https://arrow.apache.org/docs/format/Columnar.html)

## Related ADRs

- ADR-010: API Versioning Strategy (coordinates SQL syntax changes with API versions)
- ADR-004: Commons Crate (provides shared models used by parsers)
- Future: ADR-013: View and CTE Support (will extend sqlparser-rs integration)
