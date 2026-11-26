# Cluster & Replication Test Scenarios [CATEGORY:CLUSTER]

This document covers distributed consensus (Raft) and multi-node coordination.

## 1. Cluster Status & Views
**Goal**: Verify visibility of cluster health.
- **Steps**:
  1. Query `system.cluster`.
  2. Verify all nodes are listed with correct status (Leader/Follower).
  3. Verify Raft term and index consistency.
- **Agent Friendly**: Check `backend/tests/testserver/cluster/test_cluster_views_http.rs`.

## 2. Replication Correctness (⚠️ DEGRADED)
**Goal**: Data inserted on leader is visible on followers.
- **Scenario**:
  1. Insert data on Node A (Leader).
  2. Query Node B (Follower).
  3. Verify data eventually arrives.
- **Note**: This is currently partially covered by `.disabled` files due to environment stability issues.

## 3. Node Recovery & Sync (⚠️ DEGRADED)
**Goal**: Catching up a node after downtime.
- **Scenario**:
  1. Stop Node C.
  2. Insert 100 rows on Leader.
  3. Restart Node C.
  4. Verify Node C receives missing logs and snapshots.
- **Note**: Covered by `test_cluster_node_recovery_and_sync.rs.disabled`.

## Coverage Analysis
- **Duplicates**: None (insufficient coverage).
- **Gaps**: Major gaps in automated testing for network partitions, leader re-election, and multi-node snapshots. This is the highest priority for additional test implementation.
