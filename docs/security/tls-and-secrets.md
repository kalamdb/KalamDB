# TLS and Secrets Guide (Actix + KalamDB)

This guide explains how to secure transport and secrets for KalamDB.

## 1) TLS architecture for KalamDB

KalamDB has two traffic planes:

1. **Client/API plane (Actix HTTP server)**
   - Serves API and WebSocket endpoints.
   - In current runtime flow, Actix is typically bound as HTTP/h2c and is expected to run behind a TLS termination layer.

2. **Cluster inter-node plane (gRPC Raft + cluster RPC)**
   - Node-to-node communication.
   - Supports mTLS via `cluster.rpc_tls`.

## 2) Recommended: install SSL for Actix using reverse proxy

For end users and production operators, this is the safest and simplest path.

### Option A: Caddy (automatic certificates)

Example Caddyfile:

```caddy
kalamdb.example.com {
  encode gzip

  reverse_proxy 127.0.0.1:2900 {
    header_up X-Forwarded-For {remote_host}
    header_up X-Real-IP {remote_host}
    header_up X-Forwarded-Proto https
  }
}
```

### Option B: Nginx (Let’s Encrypt or managed certs)

```nginx
server {
  listen 443 ssl http2;
  server_name kalamdb.example.com;

  ssl_certificate     /etc/letsencrypt/live/kalamdb.example.com/fullchain.pem;
  ssl_certificate_key /etc/letsencrypt/live/kalamdb.example.com/privkey.pem;

  location / {
    proxy_pass http://127.0.0.1:2900;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-Proto https;
  }
}
```

### Why this is preferred

- Simpler certificate lifecycle management
- Easier OCSP/HSTS/hardening controls
- Keeps app runtime focused on database concerns

## 3) "Install SSL in Actix" directly (custom build path)

If you maintain a custom fork and explicitly want TLS in-process, you can switch the HTTP bind path to a TLS bind (for example Rustls-backed Actix). This is **not** the standard deployment path documented above.

Use this approach only if you need direct TLS in the app process and can own cert rotation/reload + operational complexity.

## 4) Enable mTLS for inter-node gRPC

For any multi-node deployment, enable `cluster.rpc_tls`.

In `backend/server.toml`:

```toml
[cluster]
cluster_id = "prod-cluster"
node_id = 1
rpc_addr = "0.0.0.0:2910"
api_addr = "http://node1.example.com:2900"

[cluster.rpc_tls]
enabled = true
ca_cert_path = "/etc/kalamdb/certs/cluster-ca.pem"
node_cert_path = "/etc/kalamdb/certs/node1.pem"
node_key_path = "/etc/kalamdb/certs/node1.key"

[[cluster.peers]]
node_id = 2
rpc_addr = "node2.example.com:2910"
api_addr = "http://node2.example.com:2900"
rpc_server_name = "node2.cluster.local"
```

### Validation behavior when mTLS is enabled

KalamDB config validation requires:

- `ca_cert_path`
- `node_cert_path`
- `node_key_path`
- `rpc_server_name` per peer

If these are missing, startup/config validation fails.

## 5) Secret generation and storage

### Generate a strong JWT secret

```bash
openssl rand -base64 32
```

### Set via environment variable

```bash
export KALAMDB_JWT_SECRET="<your-random-secret>"
```

### Useful security env overrides

- `KALAMDB_JWT_SECRET`
- `KALAMDB_JWT_TRUSTED_ISSUERS`
- `KALAMDB_RATE_LIMIT_AUTH_REQUESTS_PER_IP_PER_SEC`
- `KALAMDB_CLUSTER_RPC_TLS_ENABLED`
- `KALAMDB_CLUSTER_RPC_TLS_CA_CERT_PATH`
- `KALAMDB_CLUSTER_RPC_TLS_NODE_CERT_PATH`
- `KALAMDB_CLUSTER_RPC_TLS_NODE_KEY_PATH`

## 6) Secret management best practices

- Keep secrets out of git and container images.
- Use secret managers (Vault/KMS/SSM/Kubernetes Secrets).
- Rotate JWT signing secrets with a planned token cutover window.
- Restrict file permissions on cert/key material (`600` keys, least privilege service account).
- Never share node private keys between cluster members.

## 7) Certificate lifecycle best practices

- Use an internal CA for cluster mTLS.
- Use unique certificate/key per node.
- Rotate node certs periodically.
- Validate SAN/CN alignment with `rpc_server_name`.
- Monitor expiration and alert early (for example at 30/14/7 days).
