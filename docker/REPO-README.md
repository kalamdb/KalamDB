# KalamDB

### One SQL schema. Your whole realtime backend.

KalamDB is an open-source, **SQL-first backend** that combines database tables, realtime subscriptions, durable pub/sub, and server functions in one system. It speaks the **PostgreSQL wire protocol**, so existing PostgreSQL tools and drivers can connect directly.

Define the backend once in SQL. KalamDB uses that schema for storage, permissions, realtime events, procedure contracts, backend-managed schema migrations, and generated TypeScript and Dart/Flutter types.

This image publishes the server. It includes:

- `kalamdb-server`
- `kalam`
- `kalam-cli`

| Your app needs | This server gives you |
| --- | --- |
| One source of truth | **Schema-first SQL:** tables, types, enums, procedures, topics, and policies |
| Existing PostgreSQL tools | **PGWire:** `psql`, drivers, prepared queries, transactions, and `CALL` (enable it in `server.toml`) |
| Live chat, feeds, and dashboards | **Realtime queries** over WebSocket |
| Background jobs and AI workers | **Durable pub/sub** with acknowledgements and retries |
| Backend business logic | **Server functions:** sandboxed TypeScript in a V8 runtime |
| Personal vs shared data | **USER**, **SHARED**, and **STREAM** tables |
| Growing data | **Hot RocksDB** plus **cold Parquet** on filesystem or object storage |
| More connections | **Multi-Raft** clusters |

Apache-2.0. Documentation: https://kalamdb.org/docs · Source: https://github.com/kalamdb/KalamDB

## Quick start

```bash
docker pull kalamdb/kalamdb:latest

docker run -d \
  --name kalamdb \
  -p 2900:2900 \
  -e KALAMDB_SERVER_HOST=0.0.0.0 \
  -e KALAMDB_ROOT_PASSWORD=kalamdb123 \
  -e KALAMDB_JWT_SECRET=replace-with-a-32-char-secret \
  -v kalamdb_data:/data \
  kalamdb/kalamdb:latest
```

The admin UI, when the image is built with it, is at `http://localhost:2900`. Sign in as `root` with `KALAMDB_ROOT_PASSWORD`.

```bash
curl http://localhost:2900/health
curl http://localhost:2900/v1/api/healthcheck
```

```bash
curl -X POST http://localhost:2900/v1/api/auth/login \
  -H "Content-Type: application/json" \
  -d '{"username":"root","password":"kalamdb123"}'
```

```bash
curl -X POST http://localhost:2900/v1/api/sql \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{"sql":"SELECT 1 AS ok"}'
```

For an application project, install the CLI on the host and point it at this server:

```bash
npm install -g @kalamdb/cli
kalam login --url http://localhost:2900
```

`kalam init` and `kalam dev` are the usual way to start a schema, generate contracts, and run an app. See the [GitHub README](https://github.com/kalamdb/KalamDB#get-started).

## PostgreSQL wire protocol

PGWire is off in the default image config. To use `psql` or a PostgreSQL driver, set this in the mounted `server.toml` and publish the port:

```toml
[postgres_wire]
enabled = true
host = "0.0.0.0"
port = 5432
```

```bash
docker run -d \
  --name kalamdb \
  -p 2900:2900 \
  -p 5432:5432 \
  -e KALAMDB_SERVER_HOST=0.0.0.0 \
  -e KALAMDB_ROOT_PASSWORD=kalamdb123 \
  -e KALAMDB_JWT_SECRET=replace-with-a-32-char-secret \
  -v kalamdb_data:/data \
  -v "$PWD/server.toml":/config/server.toml:ro \
  kalamdb/kalamdb:latest
```

```bash
psql -h 127.0.0.1 -p 5432 -U root -d kalam -W
```

Queries still use KalamDB's SQL engine and permissions. Protocol support does not mean full PostgreSQL SQL or extension compatibility. See [client compatibility](https://github.com/kalamdb/KalamDB/blob/main/docs/architecture/pg-catalog-shims.md).

## Docker Compose

Single node. The compose file publishes host port **8088**:

- https://github.com/kalamdb/KalamDB/blob/main/docker/run/single/docker-compose.yml

```bash
git clone https://github.com/kalamdb/KalamDB.git
cd KalamDB/docker/run/single
export KALAMDB_JWT_SECRET=replace-with-a-32-char-secret
docker compose up -d
```

Open `http://localhost:8088`. Override the host port with `KALAMDB_PORT=2900 docker compose up -d`.

Three-node cluster. Host HTTP ports are **8081**, **8082**, and **8083**:

- https://github.com/kalamdb/KalamDB/blob/main/docker/run/cluster/docker-compose.yml

```bash
git clone https://github.com/kalamdb/KalamDB.git
cd KalamDB
docker compose -f docker/run/cluster/docker-compose.yml up -d
```

- Node 1: `http://localhost:8081`
- Node 2: `http://localhost:8082`
- Node 3: `http://localhost:8083`

## CLI in the container

```bash
docker exec -it kalamdb kalam --version
docker exec -it kalamdb kalam --help
docker exec -it kalamdb bash
```

The CLI keeps config and credentials in `/data/.kalam`, so they persist with the data volume.

```bash
docker logs -f kalamdb
docker stop kalamdb
docker start kalamdb
```

## Environment variables

| Variable | Purpose | Example |
| --- | --- | --- |
| `KALAMDB_SERVER_HOST` | Bind address inside the container | `0.0.0.0` |
| `KALAMDB_ROOT_PASSWORD` | Root password for the first login | `kalamdb123` |
| `KALAMDB_JWT_SECRET` | JWT signing secret, at least 32 characters | `replace-with-a-32-char-secret` |
| `KALAMDB_LOG_LEVEL` | Server log level | `info` |
| `KALAMDB_PORT` | Host port for the single-node compose file | `8088` |
| `KALAMDB_ALLOW_REMOTE_SETUP` | Allow initial setup from outside the container | `true` |
| `KALAMDB_CLUSTER_ID` | Cluster identifier | `docker-cluster` |
| `KALAMDB_NODE_ID` | Node id | `1` |
| `KALAMDB_CLUSTER_RPC_ADDR` | Raft address for this node | `kalamdb-node1:2910` |
| `KALAMDB_CLUSTER_API_ADDR` | API address for this node | `http://kalamdb-node1:2900` |
| `KALAMDB_CLUSTER_PEERS` | Other cluster members | `2@kalamdb-node2:2910@http://kalamdb-node2:2900` |

Change the root password and JWT secret before anything other than local testing, and keep `/data` on a volume.

## Persistence

Database files live under `/data`.

```bash
docker run --rm \
  -v kalamdb_data:/data \
  -v "$PWD":/backup \
  alpine \
  tar czf /backup/kalamdb-data.tar.gz /data
```

## Troubleshooting

If the container is up and the port does not answer, check `docker logs kalamdb`. The server must listen on `0.0.0.0`, and port `2900` (or the compose host port) must be published. A non-localhost bind needs `KALAMDB_JWT_SECRET`.

Login uses the `KALAMDB_ROOT_PASSWORD` from container startup.

`Permission denied (os error 13)` means the image user cannot write its paths. The process is not root. A custom `server.toml` must be readable, `data_path` and `logs_path` must be writable, and a bind-mounted `/data` must be writable by uid `65532`. A named volume is the simplest persistent setup.

Reset a local container:

```bash
docker rm -f kalamdb
docker volume rm kalamdb_data
```

## Links

- https://hub.docker.com/r/kalamdb/kalamdb
- https://github.com/kalamdb/KalamDB
- https://kalamdb.org/docs
- https://github.com/kalamdb/KalamDB/blob/main/docker/run/single/docker-compose.yml
- https://github.com/kalamdb/KalamDB/blob/main/docker/run/cluster/docker-compose.yml
