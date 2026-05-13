# Quickstart Guide: Using kalam-cli

**Feature**: System Improvements and Performance Optimization  
**Component**: kalam-cli (Interactive Command-Line Client)  
**Version**: 1.0.0

## Overview

`kalam-cli` is an interactive command-line client for KalamDB, similar to `mysql` or `psql`. It provides a familiar SQL shell experience with modern features like live query subscriptions, real-time data streaming, and SQL keyword auto-completion.

**Key Features**:
- Interactive SQL query execution
- WebSocket subscriptions for real-time updates
- Multiple output formats (table, JSON, CSV)
- Command history and auto-completion
- Configuration file support
- Batch SQL execution from files

---

## Installation

### Prerequisites
- Rust 1.75+ (stable toolchain)
- KalamDB server running (default: http://localhost:2900)

### Build from Source

```bash
# Clone repository
git clone https://github.com/your-org/KalamDB.git
cd KalamDB/cli

# Build CLI
cargo build --release

# Install (optional)
cargo install --path kalam-cli

# Run
./target/release/kalam-cli
```

### Binary Release

```bash
# Download latest release
curl -LO https://github.com/your-org/KalamDB/releases/latest/download/kalam-cli-macos
chmod +x kalam-cli-macos
./kalam-cli-macos
```

---

## Quick Start

### First Connection

```bash
# Connect with default settings (localhost:2900, user: system)
kalam-cli

# Connect with specific host and user
kalam-cli -u jamal -h http://localhost:2900

# Connect with JWT authentication
kalam-cli -u jamal -h http://localhost:2900 --token eyJhbGc...

# Connect with API key
kalam-cli -u jamal --apikey your-api-key
```

On first run, `kalam-cli` creates a configuration file at `~/.kalam/config.toml` with default settings.

---

## Basic Usage

### Interactive SQL Queries

```sql
-- Create namespace
kalam> CREATE NAMESPACE chat;
Namespace created.

-- Create table
kalam> CREATE USER TABLE chat.messages (
    id INT,
    user_id TEXT,
    content TEXT,
    created_at TIMESTAMP
);
Table created.

-- Insert data
kalam> INSERT INTO chat.messages (user_id,content,created_at) VALUES
('jamal', 'Hello, world!', now());
1 row inserted.

-- Query data
kalam> SELECT * FROM chat.messages LIMIT 10;
┌────┬─────────┬───────────────┬─────────────────────┐
│ id │ user_id │ content       │ created_at          │
├────┼─────────┼───────────────┼─────────────────────┤
│ 1  │ jamal   │ Hello, world! │ 2025-10-21 14:30:00 │
└────┴─────────┴───────────────┴─────────────────────┘
1 row returned (0.023s)

-- Update data
kalam> UPDATE chat.messages SET content = 'Updated message' WHERE id = 1;
1 row updated.

-- Delete data
kalam> DELETE FROM chat.messages WHERE id = 1;
1 row deleted.
```

### Output Formats

```bash
# JSON output
kalam-cli --json
kalam> SELECT * FROM messages LIMIT 2;
[
  {"id": 1, "user_id": "jamal", "content": "Hello", "created_at": "2025-10-21T14:30:00Z"},
  {"id": 2, "user_id": "jamal", "content": "World", "created_at": "2025-10-21T14:31:00Z"}
]

# CSV output
kalam-cli --csv
kalam> SELECT * FROM messages LIMIT 2;
id,user_id,content,created_at
1,jamal,Hello,2025-10-21T14:30:00Z
2,jamal,World,2025-10-21T14:31:00Z
```

---

## Live Query Subscriptions

### Basic Subscription

```sql
-- Subscribe to real-time updates
kalam> SUBSCRIBE TO chat.messages;
Subscription established (live_id: 660e8400-e29b-41d4-a716-446655440001)
Listening for changes... (Press Ctrl+C to stop)

[14:35:22] INSERT → id=2, user_id=jamal, content=New message
[14:35:30] UPDATE → id=1, content: Hello → Hello, updated!
[14:35:45] DELETE → id=2
```

### Filtered Subscription

```sql
-- Subscribe with WHERE clause
kalam> SUBSCRIBE TO chat.messages WHERE user_id = 'jamal';

[14:36:00] INSERT → id=3, user_id=jamal, content=Another message
```

### Subscription with Initial Data

```sql
-- Fetch last 100 rows immediately, then stream changes
kalam> SUBSCRIBE TO chat.messages LAST 100;

Initial data: 100 rows
[14:37:00] INSERT → id=101, user_id=alice, content=New from alice
```

### Controlling Subscriptions

```sql
-- Pause streaming (stop receiving updates)
kalam> \pause

-- Continue streaming
kalam> \continue

-- Stop subscription
Press Ctrl+C or type \quit
```

---

## Utility Commands

### Show Tables

```sql
kalam> SHOW TABLES;
┌───────────┬─────────────┬───────────┐
│ Namespace │ Table       │ Type      │
├───────────┼─────────────┼───────────┤
│ chat      │ messages    │ User      │
│ chat      │ channels    │ Shared    │
│ analytics │ events      │ Stream    │
└───────────┴─────────────┴───────────┘
3 tables
```

### Describe Table

```sql
kalam> DESCRIBE chat.messages;
┌──────────────┬───────────┬──────────┬─────────────────┐
│ Column       │ Type      │ Nullable │ Default         │
├──────────────┼───────────┼──────────┼─────────────────┤
│ id           │ INT       │ No       │ AUTO_INCREMENT  │
│ user_id      │ TEXT      │ No       │                 │
│ content      │ TEXT      │ No       │                 │
│ created_at   │ TIMESTAMP │ No       │ now()           │
│ _updated     │ TIMESTAMP │ Yes      │                 │
│ _deleted     │ BOOLEAN   │ Yes      │ false           │
└──────────────┴───────────┴──────────┴─────────────────┘
Schema version: 3
History: SELECT * FROM system.table_schemas WHERE table_name = 'messages'
```

### Table Statistics

```sql
kalam> SHOW TABLE STATS chat.messages;
┌───────────────────┬────────────┐
│ Metric            │ Value      │
├───────────────────┼────────────┤
│ Buffered rows     │ 1,523      │
│ Flushed rows      │ 10,000     │
│ Total rows        │ 11,523     │
│ Storage size      │ 45.2 MB    │
│ Last flush        │ 5 min ago  │
└───────────────────┴────────────┘
```

---

## Backslash Commands

### Connection Management

```sql
-- Connect to different host
\connect -h http://production.kalamdb.com -u jamal --token <jwt>

-- Show current configuration
\config
Host: http://localhost:2900
User: jamal
Auth: JWT (expires: 2025-10-22)
Output: Table
Color: Enabled
```

### Administration

```sql
-- Storage flush all tables in current namespace
\flush
Storage flush completed successfully

-- Check server health
\health
Status: OK
Version: 0.1.0-alpha
Uptime: 2d 3h 15m
Active connections: 42
Active subscriptions: 18
```

### Help

```sql
-- Show all commands
\help
SQL Commands:
  SELECT, INSERT, UPDATE, DELETE, CREATE, DROP, ALTER, FLUSH
  SHOW TABLES, DESCRIBE <table>, SHOW TABLE STATS <table>
  SUBSCRIBE TO <table> [WHERE ...] [LAST <n>]

Backslash Commands:
  \quit, \q        Exit the CLI
  \help            Show this help
  \connect         Switch connection
  \config          Show configuration
  \flush           Storage flush all tables in current namespace
  \health          Check server health
  \pause           Pause subscription
  \continue        Resume subscription
```

---

## Batch Execution

### From File

```bash
# Create SQL file
cat > setup.sql << EOF
CREATE NAMESPACE demo;
CREATE USER TABLE demo.messages (id INT, content TEXT);
INSERT INTO demo.messages VALUES (1, 'First');
INSERT INTO demo.messages VALUES (2, 'Second');
EOF

# Execute batch
kalam-cli --file setup.sql
Statement 1: Namespace created
Statement 2: Table created
Statement 3: 1 row inserted
Statement 4: 1 row inserted

Completed 4 statements in 0.145s
```

### Multi-line Queries

```sql
-- Use semicolon to execute
kalam> INSERT INTO messages (id, content)
...>   VALUES (1, 'Hello'),
...>          (2, 'World');
2 rows inserted.
```

---

## Configuration File

### Location
- `~/.kalam/config.toml`
- Created automatically on first run

### Example Configuration

```toml
[connection]
host = "http://localhost:2900"
user = "jamal"
token = "eyJhbGc..."  # Optional: JWT token

[output]
format = "table"  # Options: "table", "json", "csv"
color = true
max_rows = 1000  # Max rows per page
```

### Override with Command-Line Flags

```bash
# Config file says format="table", but override with JSON
kalam-cli --json
```

---

## Auto-Completion

### SQL Keywords

```sql
-- Press TAB after typing partial keyword
kalam> SEL<TAB>
SELECT

kalam> CRE<TAB>
CREATE
```

### Multiple Matches

```sql
-- Shows suggestions when multiple matches
kalam> SH<TAB>
SHOW    SHARED
```

### Supported Keywords
- SELECT, INSERT, UPDATE, DELETE
- CREATE, DROP, ALTER
- FROM, WHERE, ORDER BY, GROUP BY, LIMIT, OFFSET
- JOIN, INNER, LEFT, RIGHT, OUTER
- SUBSCRIBE, SHOW, DESCRIBE

---

## Command History

### Navigation

```sql
-- Press UP arrow to see previous commands
-- Press DOWN arrow to move forward in history

-- History is saved across sessions
```

### History File
- Location: `~/.kalam/history`
- Stores last 1000 commands

---

## Examples

### Real-Time Chat Application

```sql
-- User 1: Subscribe to messages
kalam> SUBSCRIBE TO chat.messages WHERE user_id = 'jamal';

-- User 2: Send message (separate CLI instance)
kalam> INSERT INTO chat.messages VALUES (100, 'jamal', 'Hello from User 2!', now());

-- User 1 sees immediately:
[14:40:00] INSERT → id=100, user_id=jamal, content=Hello from User 2!
```

### AI Agent Monitoring

```sql
-- Monitor AI agent outputs in real-time
kalam> SUBSCRIBE TO ai.conversations WHERE agent_id = 'gpt-4';

[14:41:00] INSERT → message_id=1523, role=assistant, content=Here's the analysis...
[14:41:05] INSERT → message_id=1524, role=assistant, content=Based on the data...
```

### Data Migration

```bash
# Export data to JSON
kalam-cli --json <<< "SELECT * FROM messages" > backup.json

# Import data (using separate script or API)
curl -X POST http://localhost:2900/api/sql \
  -H "X-USER-ID: jamal" \
  -d '{"sql": "INSERT INTO messages ..."}'
```

---

## Troubleshooting

### Connection Issues

```bash
# Test server connectivity
curl http://localhost:2900/v1/health

# Check CLI version
kalam-cli --version

# Enable debug logging
RUST_LOG=debug kalam-cli
```

### Authentication Errors

```bash
# Verify JWT token is valid
kalam-cli --token <your-token>

# Try localhost bypass (if enabled on server)
kalam-cli -h http://localhost:2900
```

### Subscription Issues

```sql
-- Check active subscriptions
SELECT * FROM system.live_queries WHERE user_id = 'jamal';

-- Kill stuck subscription
KILL LIVE QUERY '660e8400-e29b-41d4-a716-446655440001';
```

---

## Next Steps

1. **Explore system tables**: `SELECT * FROM system.users`, `system.jobs`, `system.live_queries`
2. **Try live queries**: Real-time monitoring with `SUBSCRIBE TO`
3. **Automate workflows**: Use `--file` for batch scripts
4. **Configure defaults**: Edit `~/.kalam/config.toml` for your environment

---

## Additional Resources

- [kalam-link API Documentation](../contracts/kalam-link-api.md): For programmatic usage
- [SQL Syntax Reference](../../../docs/backend/SQL_SYNTAX.md): Complete SQL command guide
- [WebSocket Protocol](../../../docs/backend/WEBSOCKET_PROTOCOL.md): Live query subscription details
- [Server Configuration](../../../docs/backend/README.md): Setting up KalamDB server

---

## Feedback

Found an issue or have a feature request? Open an issue on [GitHub](https://github.com/your-org/KalamDB/issues).
