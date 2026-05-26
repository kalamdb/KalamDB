# KalamDB Security Checklist

Use this checklist before production rollout and during operations.

## A) Pre-production checklist

- [ ] API is exposed over HTTPS only (TLS termination at edge)
- [ ] `KALAMDB_JWT_SECRET` is strong and rotated from defaults
- [ ] Trusted issuers configured (`auth.jwt_trusted_issuers`) when using external IdP
- [ ] `auth.allow_remote_setup = false`
- [ ] Local password login policy set explicitly with `auth.local.enabled`
- [ ] OIDC issuer, client ID, and auto-provisioning policy set under `[auth.oidc]`
- [ ] Password policy defaults preserved or strengthened
- [ ] CORS origins restricted to known domains
- [ ] WebSocket origin rules set (`strict_ws_origin_check = true` where appropriate)
- [ ] Rate limiting enabled (`enable_connection_protection = true`)
- [ ] Body/message limits set to safe values
- [ ] Health/setup/admin-sensitive endpoints reachable only from trusted paths

## B) Multi-node/cluster checklist

- [ ] `cluster.rpc_tls.enabled = true`
- [ ] Valid CA, node cert, and node key configured per node
- [ ] `rpc_server_name` configured for each peer
- [ ] Raft RPC ports are private (VPC/subnet/firewall restricted)
- [ ] No shared private keys across nodes
- [ ] Certificate expiry alerts configured

## C) IAM and access checklist

- [ ] Least privilege role assignments (`user`/`service`/`dba`/`system`)
- [ ] Separate human admin accounts from service accounts
- [ ] No long-lived broad-scope tokens without rotation policy
- [ ] Audit review process for auth failures and privileged operations

## D) Runtime monitoring checklist

- [ ] Alert on spikes in 401/403 responses
- [ ] Alert on repeated rate-limit violations and IP bans
- [ ] Alert on auth endpoint brute-force patterns
- [ ] Alert on unusual cluster peer connectivity/auth failures
- [ ] Retain logs/metrics with tamper-aware storage policy

## E) Incident response starter checklist

- [ ] Rotate JWT secret if token compromise is suspected
- [ ] Revoke/disable compromised users or service identities
- [ ] Rotate cluster node certificates if key leakage is suspected
- [ ] Temporarily tighten ingress/rate limits during attack windows
- [ ] Preserve logs and request IDs for forensic review

## F) Quarterly review checklist

- [ ] Validate all certificates are within rotation window
- [ ] Re-verify CORS/origin and public endpoint exposure
- [ ] Re-test unauthorized access scenarios (auth bypass, forged token)
- [ ] Review role grants and remove stale privileged accounts
- [ ] Run current security regression tests in CI
