# KalamDB Docker Guide

KalamDB is a SQL-first realtime backend for agent-native apps. Agents and frontends share one HTTP SQL API for reads and writes plus WebSocket live subscriptions, with USER tables and auth keeping access policy-scoped. This directory contains the Docker image definitions and compose setups used to run KalamDB as a single node or a 3-node cluster.

The Docker image ships with both binaries:

- `kalamdb-server` for the database server
- `kalam` and `kalam-cli` for the command-line client

## What Is Here

```text
docker/
├── build/
│   ├── Dockerfile             # Full source build image
│   ├── Dockerfile.prebuilt    # Runtime image from prebuilt binaries
│   ├── build-and-test-local.sh
│   └── test-docker-image.sh
└── run/
    ├── single/
    │   ├── docker-compose.yml
    │   └── server.toml
    └── cluster/
        ├── docker-compose.yml
        ├── server1.toml
        ├── server2.toml
        └── server3.toml
```

## Quick Start

### Pull and run the published image

```bash
docker pull jamals86/kalamdb:latest

docker run -d \
  --name kalamdb \
  -p 2900:2900 \
  -e KALAMDB_SERVER_HOST=0.0.0.0 \
  -e KALAMDB_ROOT_PASSWORD=kalamdb123 \
  -e KALAMDB_JWT_SECRET="replace-with-a-32-char-secret" \
  -v kalamdb_data:/data \
  jamals86/kalamdb:latest
```

Verify the server is healthy:

```bash
curl http://localhost:2900/health
curl http://localhost:2900/v1/api/healthcheck
```

### Run with Docker Compose

The single-node compose file maps container port `2900` to host port `2900` by default.

```bash
cd docker/run/single
export KALAMDB_JWT_SECRET="replace-with-a-32-char-secret"
docker compose up -d
```

Open the server at `http://localhost:8088`.

To change the host port:

```bash
KALAMDB_PORT=2900 docker compose up -d
```

### Start a 3-node cluster

```bash
cd docker/run/cluster
export KALAMDB_JWT_SECRET="replace-with-a-32-char-secret"
docker compose up -d
```

Default node endpoints:

- Node 1: `http://localhost:2901`
- Node 2: `http://localhost:2902`
- Node 3: `http://localhost:2903`

## Build the Image Locally

### Recommended: build from prebuilt binaries

This is the path used by the release workflow. Build the Rust binaries once, then create the smaller runtime image.

```bash
./docker/build/build-and-test-local.sh kalamdb:local
```

That script will:

1. Build Linux-targeted `kalamdb-server` and `kalam` binaries for your local Docker platform
2. Stage them as a Docker build context
3. Build `docker/build/Dockerfile.prebuilt`
4. Run smoke tests against the resulting image

### Manual prebuilt image build

```bash
cd /path/to/KalamDB

case "$(uname -m)" in
  x86_64)
    export LOCAL_TARGET=x86_64-unknown-linux-gnu
    export LOCAL_PLATFORM=linux/amd64
    export BIN_DIR=binaries-amd64
    SKIP_UI_BUILD=1 cargo build --release --target "$LOCAL_TARGET" --features mimalloc -p kalamdb-server -p kalam-cli --bin kalamdb-server --bin kalam
    ;;
  arm64)
    export LOCAL_TARGET=aarch64-unknown-linux-gnu
    export LOCAL_PLATFORM=linux/arm64
    export BIN_DIR=binaries-arm64
    SKIP_UI_BUILD=1 cross build --release --target "$LOCAL_TARGET" --features mimalloc -p kalamdb-server -p kalam-cli --bin kalamdb-server --bin kalam
    ;;
esac

mkdir -p "$BIN_DIR"
cp "target/$LOCAL_TARGET/release/kalamdb-server" "$BIN_DIR/"
cp "target/$LOCAL_TARGET/release/kalam" "$BIN_DIR/"

docker build \
  --platform "$LOCAL_PLATFORM" \
  --build-context binaries="$BIN_DIR" \
  -f docker/build/Dockerfile.prebuilt \
  -t kalamdb:local \
  .
```

### Full source build inside Docker

For maintainers, `docker/build/Dockerfile` performs a full multi-stage source build and then packages the server and CLI into a runtime image.

```bash
docker build -f docker/build/Dockerfile -t kalamdb:source-build .
```

## Authentication and First Login

If you set `KALAMDB_ROOT_PASSWORD`, you can authenticate immediately after startup.

```bash
curl -X POST http://localhost:2900/v1/api/auth/login \
  -H "Content-Type: application/json" \
  -d '{"username":"root","password":"kalamdb123"}'
```

Then run a SQL request with the returned bearer token:

```bash
curl -X POST http://localhost:2900/v1/api/sql \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{"sql":"SELECT 1 AS ok"}'
```

## Using the CLI in the Container

The image includes the Kalam CLI under both `kalam` and `kalam-cli`.

Inside the container, the CLI stores local config, credentials, and history in `/data/.kalam` by default so they persist with the `/data` volume.

```bash
docker exec -it kalamdb kalam --version
docker exec -it kalamdb kalam-cli --version
```

If you want an interactive shell in the container, use bash:

```bash
docker exec -it kalamdb bash
```

## Configuration

The container starts with:

- data under `/data`
- config under `/config/server.toml`
- JSON file logging enabled in Docker configs so `system.server_logs` can return rows
- server command `kalamdb-server /config/server.toml`

The single-node compose file already mounts a persistent volume for `/data`. To provide your own config, mount a custom `server.toml` over `/config/server.toml`.

Example:

```yaml
services:
  kalamdb:
    volumes:
      - kalamdb_data:/data
      - ./server.toml:/config/server.toml:ro
```

Important environment variables used by the compose files:

- `KALAMDB_JWT_SECRET`: required for safe non-localhost deployments
- `KALAMDB_ROOT_PASSWORD`: optional root password for immediate login
- `KALAMDB_PORT`: host port override for the single-node compose setup
- `KALAMDB_LOG_LEVEL`: server log level

## Persistence

The default compose setup stores data in Docker-managed volumes.

Inspect the single-node volume:

```bash
docker volume inspect kalamdb_data
```

Back up the data volume:

```bash
docker run --rm \
  -v kalamdb_data:/data \
  -v "$PWD":/backup \
  alpine \
  tar czf /backup/kalamdb-data.tar.gz /data
```

## Smoke Testing

To validate an image locally:

```bash
./docker/build/test-docker-image.sh kalamdb:local
```

The smoke test checks:

- server startup and health endpoints
- presence of `kalamdb-server`, `kalam-cli`, and `kalam`
- root login flow
- SQL execution through the HTTP API

## Troubleshooting

### Container starts but is unreachable

Make sure the server is bound to `0.0.0.0` when publishing ports from Docker.

```bash
docker logs kalamdb
```

### Compose stack is healthy but login fails

If you did not set `KALAMDB_ROOT_PASSWORD`, set one explicitly and recreate the container.

### Port collision on the host

Override the published port:

```bash
KALAMDB_PORT=8090 docker compose up -d
```

### Clean reset

```bash
docker compose down -v
docker volume prune
```

Use volume deletion carefully. It removes persisted KalamDB data.

## Related Links

- Main project README: `../README.md`
- Docker single-node compose: `run/single/docker-compose.yml`
- Docker cluster compose: `run/cluster/docker-compose.yml`
- Project docs: <https://kalamdb.org/docs>

# Create new user
docker exec kalamdb kalam-cli user create --name "newuser" --role "user"
```

---

## Advanced Configuration

### Custom Dockerfile

If you need to modify the build:

1. Copy `Dockerfile` to `Dockerfile.custom`
2. Make your changes
3. Build: `docker build -f Dockerfile.custom -t kalamdb:custom ../../`

### Debug Mode

Run with debug logging:

```bash
docker-compose up -d
docker-compose exec kalamdb kalam-cli exec "UPDATE system.config SET log_level='debug'"
docker-compose restart
```

### Health Check Customization

Edit `docker-compose.yml`:

```yaml
healthcheck:
  test: ["CMD", "/usr/local/bin/kalam-cli", "exec", "SELECT COUNT(*) FROM system.users"]
  interval: 10s
  timeout: 5s
  start_period: 30s
  retries: 5
```

---

## Related Documentation

- [Quick Start Guide](../../specs/006-docker-wasm-examples/quickstart.md)
- [Data Model](../../specs/006-docker-wasm-examples/data-model.md)
- [HTTP Auth Contract](../../specs/006-docker-wasm-examples/contracts/http-auth.md)
- [Backend Documentation](../../backend/README.md)

---

## Support

For issues and questions:
- GitHub Issues: https://github.com/yourusername/KalamDB/issues
- Documentation: https://kalamdb.dev/docs
