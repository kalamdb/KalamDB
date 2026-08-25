# Shared Table Streaming Notifications

## Overview

This document describes the shared-table live query notification path introduced in kalamdb-core to support large subscriber fan-out efficiently.

Goal:
- Support SHARED table subscriptions similarly to USER table subscriptions.
- Avoid per-subscriber full row conversion overhead when shared subscribers are in the thousands.

## Problem

For SHARED tables with many subscribers, repeatedly materializing row payloads and applying projection logic per subscriber creates avoidable CPU and latency overhead in the notification worker.

## Design

### Shared subscription indexing

ConnectionsManager indexes shared-table subscriptions through `IndexedSubscriberRelation`:

- Broadcast: every change on the table is a candidate
- Keyed: lookup by equality column name and typed `ScalarValue` (for example `conversation_id`)
- Deny: not stored in lookup buckets

The bound RLS evaluator still fail-closes after candidate selection. Fan-out is not “all subscribers on the table” unless the policy is broadcast (`USING (true)` or RLS bypass).

### Streaming fan-out path

NotificationService adds a dedicated shared dispatch path:

- Method: notify_shared_table_streaming
- Chunk size constant: SHARED_NOTIFY_CHUNK_SIZE = 2048

The method executes in three phases:

1. Pre-compute row JSON once
   - Convert row_data (and old_data when present) once into JSON maps.
   - Wrap maps in Arc for zero-copy sharing across chunk tasks.

2. Collect candidate handles
   - Look up broadcast plus keyed buckets for the changed row, then clone those handles.

3. Parallel chunk processing
   - Split subscribers into chunks of 256.
   - Spawn a Tokio task per chunk.
   - For each subscriber in a chunk:
     - Evaluate optional filter expression.
     - Apply projection using the pre-computed JSON map.
     - Build per-subscriber notification payload.
     - Apply flow-control gate.
     - try_send to the subscriber channel.

## Why this is faster

- Eliminates N repeated ScalarValue to JSON conversions.
- Uses Arc-shared pre-computed payload for chunk workers.
- Exploits task-level parallelism for high fan-out tables.
- Keeps existing per-subscriber semantics (filter, projection, flow control, unique subscription id).

## Integration points

- Worker loop shared-table branch now routes to streaming fan-out.
- Forwarded shared notifications also route to streaming fan-out.
- has_subscribers checks include shared-table subscriptions.

## Correctness and safety notes

- USER and SHARED subscription indexes remain isolated.
- Unregister and disconnect cleanup remove shared index entries.
- Shutdown clears shared subscription index.
- Existing notification delivery backpressure behavior remains unchanged via try_send and flow control.

## Tests added

ConnectionsManager tests cover both behavior and scale characteristics:

- shared subscription index and unindex
- cleanup on disconnect
- has_subscriptions_for_table includes shared subscriptions
- delivery to multiple shared subscribers
- mixed user and shared isolation
- reverse lookup by live query id
- empty shared table lookup behavior
- many-subscriber performance sanity (1000 subscribers, bounded runtime expectation)

## Validation status

- Targeted shared subscription test group passes.
- Full connections_manager test group passes with no regressions.
