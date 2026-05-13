# Testing KalamDB CLI Against Different Configurations

This guide explains how to run KalamDB CLI tests against different server configurations (ports, hosts, authentication settings).

## Quick Start

### Using Helper Scripts (Recommended)

The easiest way to test with custom configurations is using the provided helper scripts:

**Unix/Linux/macOS:**
```bash
cd cli

# Test with default settings
./run-tests.sh --test smoke --nocapture

# Test against server on port 3000
./run-tests.sh --url http://localhost:3000 --test smoke --nocapture

# Test with authentication
./run-tests.sh --url http://localhost:2900 --password "your-password" --test smoke --nocapture
```

**Windows PowerShell:**
```powershell
cd cli

# Test with default settings
.\run-tests.ps1 -Test "smoke" -NoCapture

# Test against server on port 3000
.\run-tests.ps1 -Url "http://localhost:3000" -Test "smoke" -NoCapture

# Test with authentication
.\run-tests.ps1 -Url "http://localhost:2900" -Password "your-password" -Test "smoke" -NoCapture
```

### Using Environment Variables

You can also set environment variables directly:

**Unix/Linux/macOS:**
```bash
export KALAMDB_SERVER_URL="http://127.0.0.1:3000"
export KALAMDB_ROOT_PASSWORD="your-password"
cd cli
cargo test --test smoke -- --nocapture
```

**Windows PowerShell:**
```powershell
$env:KALAMDB_SERVER_URL = "http://127.0.0.1:3000"
$env:KALAMDB_ROOT_PASSWORD = "your-password"
cd cli
cargo test --test smoke -- --nocapture
```

## Configuration Options

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `KALAMDB_SERVER_URL` | Full server URL including protocol and port | `http://127.0.0.1:2900` |
| `KALAMDB_ROOT_PASSWORD` | Root user password for authentication | `""` (empty) |

### Script Options

**run-tests.sh (Bash):**
- `-u, --url <URL>` - Server URL
- `-p, --password <PASS>` - Root password
- `-t, --test <FILTER>` - Test filter
- `--nocapture` - Show test output
- `-h, --help` - Show help

**run-tests.ps1 (PowerShell):**
- `-Url <URL>` - Server URL
- `-Password <PASS>` - Root password
- `-Test <FILTER>` - Test filter
- `-NoCapture` - Show test output
- `-Help` - Show help

## Common Testing Scenarios

### 1. Testing Against Different Ports

When running multiple KalamDB instances:

```bash
# Start server on port 3000
cd backend
cargo run -- --port 3000

# Test against it
cd ../cli
./run-tests.sh --url http://127.0.0.1:3000 --test smoke --nocapture
```

### 2. Testing Against Cluster Nodes

When running a 3-node cluster:

```bash
# Test node 1 (port 8081)
./run-tests.sh --url http://127.0.0.1:2901 --test smoke

# Test node 2 (port 8082)
./run-tests.sh --url http://127.0.0.1:2902 --test smoke

# Test node 3 (port 8083)
./run-tests.sh --url http://127.0.0.1:2903 --test smoke
```

### 3. Testing With Authentication

When the server has authentication enabled:

```bash
# Start server with authentication
cd backend
cargo run

# Test with credentials
cd ../cli
./run-tests.sh --password "admin-password" --test smoke --nocapture
```

### 4. Testing Specific Features

```bash
# Test core operations only
./run-tests.sh --test smoke_test_core_operations --nocapture

# Test flush operations
./run-tests.sh --test smoke_test_flush --nocapture

# Test custom functions
./run-tests.sh --test smoke_test_custom_functions --nocapture
```

### 5. Testing Remote Servers

```bash
# Test against remote development server
./run-tests.sh --url https://dev.kalamdb.example.com --password "dev-pass" --test smoke --nocapture
```

## Implementation Details

The configuration system is implemented in [cli/tests/common/mod.rs](../cli/tests/common/mod.rs):

```rust
pub fn server_url() -> &'static str {
    SERVER_URL
        .get_or_init(|| {
            std::env::var("KALAMDB_SERVER_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:2900".to_string())
        })
        .as_str()
}

pub fn root_password() -> &'static str {
    ROOT_PASSWORD
        .get_or_init(|| {
            std::env::var("KALAMDB_ROOT_PASSWORD")
                .unwrap_or_else(|_| "".to_string())
        })
        .as_str()
}
```

All CLI tests use these functions, ensuring consistent configuration across the entire test suite.

## Troubleshooting

### Server Not Running

If you see this error:
```
╔══════════════════════════════════════════════════════════════════╗
║                    SERVER NOT RUNNING                            ║
╚══════════════════════════════════════════════════════════════════╝
```

**Solution:** Start the KalamDB server first:
```bash
cd backend
cargo run
```

### Connection Refused

If tests fail with connection errors:

1. **Check server is running:**
   ```bash
   curl http://localhost:2900/health
   ```

2. **Verify port matches:**
   - If server is on port 3000, use `--url http://localhost:3000`

3. **Check firewall/network:**
   - Ensure port is not blocked
   - For remote servers, check network connectivity

### Authentication Errors

If you see "Invalid credentials" or "Unauthorized":

1. **Verify root password:**
   ```bash
   # Check what password the server expects
   # Set matching password in tests
   ./run-tests.sh --password "correct-password"
   ```

2. **Reset root password:**
   - Stop server, clear data directory, restart with fresh installation

## Best Practices

1. **Use helper scripts** - They provide clear configuration output and error handling
2. **Test locally first** - Verify tests pass against local server before testing remote
3. **One test at a time** - When debugging, run specific tests with `--test <name>`
4. **Use --nocapture** - See detailed output when tests fail
5. **Document custom configs** - If using non-standard ports, document in project README

## See Also

- [CLI README](../cli/README.md) - Full CLI documentation
- [Smoke Tests README](../cli/tests/smoke/README.md) - Smoke test details
- [Test Config Quick Reference](../cli/TEST_CONFIG.md) - Quick reference guide
