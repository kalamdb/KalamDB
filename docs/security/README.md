# KalamDB Security Documentation

This directory contains end-user security guidance for running KalamDB safely in development, staging, and production.

## Documents

1. **[Backend Hardening Guide](./backend-hardening.md)**
   - Production-safe baseline settings
   - Authentication and authorization hardening
   - CORS, request limits, and abuse controls
   - Endpoint exposure strategy

2. **[TLS and Secrets Guide](./tls-and-secrets.md)**
   - How to add SSL/TLS in front of Actix (recommended path)
   - How to configure mTLS for inter-node gRPC (Raft + cluster RPC)
   - Secret generation, storage, rotation, and env overrides

3. **[gRPC Security and Unauthorized Access Protection](./grpc-security.md)**
   - Inter-node RPC trust model
   - Anti-bypass controls for cluster identity and peer validation
   - How forwarded SQL is authenticated and authorization-checked

4. **[Security Checklist](./security-checklist.md)**
   - Deployment checklist before go-live
   - Ongoing operations checklist
   - Incident response starter checklist

5. **[Firebase Authentication Through OIDC](./firebase-auth.md)**
   - Firebase setup through the standard `[auth.oidc]` provider model
   - `server.toml` configuration reference
   - Browser login discovery and user provisioning notes
   - Security guidance for issuer, audience, and default roles

## Recommended reading order

1. Backend Hardening Guide
2. TLS and Secrets Guide
3. gRPC Security and Unauthorized Access Protection
4. Security Checklist

---

If you only do one thing first: enable TLS at the edge, set a strong JWT secret (`KALAMDB_JWT_SECRET`), enable cluster `rpc_tls` for multi-node deployments, and keep setup/auth endpoints tightly controlled.
