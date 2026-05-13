# Spec 019: Unified Command Applier (Event-Driven Architecture)

## Status: Ready for Implementation
## Created: 2026-01-09
## Updated: 2026-01-09

---

## 0. Critical Invariants

> **These invariants MUST be enforced throughout implementation.**

### Invariant 1: Single Mutation Point
```
ALL database mutations (persist, cache, register provider) happen in ONE place:
  → CommandExecutorImpl

Handlers NEVER mutate state. They only:
  1. Parse input
  2. Build command
  3. Call applier.apply(cmd)
  4. Return result
```

### Invariant 2: No Code Duplication
```
Execution logic exists in EXACTLY ONE location:
  → CommandExecutorImpl::execute_*()

The following MUST NOT exist:
  ✗ Same logic in handler AND applier
  ✗ Same logic in standalone AND cluster paths
  ✗ Mode-specific branching (is_cluster_mode) in handlers
```

### Invariant 3: OpenRaft Quorum Model (Fully Delegated)
```
We FULLY TRUST OpenRaft for consensus. No custom replication logic.

OpenRaft's client_write() workflow:
  1. Client calls raft.client_write(command)
  2. OpenRaft appends entry to leader's log
  3. OpenRaft replicates to followers IN PARALLEL (automatic, async)
  4. Once QUORUM acknowledges → entry is COMMITTED
  5. OpenRaft applies entry to RaftStateMachine::apply() on ALL nodes
  6. client_write() returns ClientWriteResponse with log_id

Key Points:
  ✓ We NEVER implement our own follower replication
  ✓ We NEVER wait for "all followers" — quorum is enough
  ✓ We NEVER track replication progress — OpenRaft does this
  ✓ ClientWriteResponse means quorum-committed and applied on leader
  ✓ Followers apply asynchronously via their own RaftStateMachine

OpenRaft APIs we use:
  - raft.client_write(cmd) → propose and wait for quorum commit
  - raft.current_leader() → get leader for forwarding
  - raft.ensure_linearizable() → for consistent reads
  - raft.metrics() → observe cluster state
```

### Invariant 4: Clean Codebase (No Legacy)
```
After implementation:
  ✗ NO deprecated code paths
  ✗ NO backward compatibility shims
  ✗ NO dead code or unused functions
  ✗ NO TODO comments for "old way"

Remove all old code aggressively. Keep codebase minimal.
```

### Invariant 5: Small, Focused Files
```
Each file should:
  - Have ONE clear responsibility
  - Be < 300 lines (prefer < 200)
  - Have methods < 50 lines each
  - Use descriptive names (no generic "utils" or "helpers")
```

---

## 1. Problem Statement

### Current Architecture Issues

The current implementation has **two separate code paths** for command execution:

```
┌─────────────────────────────────────────────────────────────────┐
│                    CURRENT ARCHITECTURE                         │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  Handler (CREATE TABLE)                                         │
│       │                                                         │
│       ├──► Standalone Mode: Execute locally via helpers         │
│       │         └──► persist, cache, register provider          │
│       │                                                         │
│       └──► Cluster Mode: Build definition, propose to Raft      │
│                 └──► Applier executes on all nodes              │
│                                                                 │
│  PROBLEM: Different code paths = potential inconsistencies      │
│  PROBLEM: Handlers contain mode-specific logic                  │
│  PROBLEM: Applier code duplicates handler logic                 │
└─────────────────────────────────────────────────────────────────┘
```

**Specific Issues:**

1. **Code Duplication**: Handler logic and Applier logic are separate implementations
2. **Mode Branching**: Every handler has `if cluster_mode { ... } else { ... }`
3. **Testing Complexity**: Must test both paths separately
4. **Bug Risk**: Fix in one path may not be applied to the other
5. **Maintenance Burden**: Changes require updates in multiple places

### Evidence from Codebase

From `create.rs`:
```rust
let message = if self.app_context.executor().is_cluster_mode() {
    // Build definition, propose to Raft
    let table_def = table_creation::build_table_definition(...)?;
    self.app_context.executor().execute_meta(cmd).await?;
    format!("...")
} else {
    // Execute directly via helper
    table_creation::create_table(...)?
};
```

From `alter.rs`:
```rust
if self.app_context.executor().is_cluster_mode() {
    // Propose to Raft
    self.app_context.executor().execute_meta(cmd).await?;
} else {
    // Apply locally
    self.apply_altered_table_locally(&table_id, &table_def)?;
}
```

---

## 2. Proposed Architecture: Unified Command Applier

### Core Principle

**"All commands flow through the Applier, regardless of mode."**

```
┌─────────────────────────────────────────────────────────────────┐
│                    PROPOSED ARCHITECTURE                        │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  Handler (CREATE TABLE)                                         │
│       │                                                         │
│       ▼                                                         │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │              UNIFIED COMMAND APPLIER                     │   │
│  │                                                          │   │
│  │  1. Validate command                                     │   │
│  │  2. Route based on mode:                                 │   │
│  │     ├── Standalone: Execute immediately                  │   │
│  │     └── Cluster:                                         │   │
│  │         ├── Leader: Execute + Replicate                  │   │
│  │         └── Follower: Forward to Leader, wait            │   │
│  │  3. Return result                                        │   │
│  │                                                          │   │
│  │  SINGLE EXECUTION PATH for all modes                     │   │
│  └─────────────────────────────────────────────────────────┘   │
│       │                                                         │
│       ▼                                                         │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │              COMMAND EXECUTOR (shared logic)             │   │
│  │                                                          │   │
│  │  - Persist to system.tables                              │   │
│  │  - Update schema cache                                   │   │
│  │  - Register DataFusion provider                          │   │
│  │  - Emit events (for live queries, audit, etc.)           │   │
│  └─────────────────────────────────────────────────────────┘   │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### Command Flow Diagrams

#### Standalone Mode
```
Client ─► Handler ─► Applier ─► Executor ─► Response
                        │
                        └── Same code path as cluster
```

#### Cluster Mode (Leader) — OpenRaft Flow
```
Client ─► Handler ─► ClusterApplier ─► raft.client_write(cmd)
                                              │
                                 ┌──────────────┴──────────────┐
                                 │    OpenRaft Internal     │
                                 ├─────────────────────────────┤
                                 │ 1. Append to leader log  │
                                 │ 2. Replicate in parallel │
                                 │    ├─► Follower 1 (async) │
                                 │    ├─► Follower 2 (async) │
                                 │    └─► Follower N (async) │
                                 │ 3. Wait for QUORUM acks  │
                                 │ 4. Mark COMMITTED        │
                                 │ 5. Call apply() on SM    │
                                 └─────────────────────────────┘
                                              │
                        RaftStateMachine::apply(entries)
                                              │
                                 CommandExecutorImpl.execute()
                                              │
                                 ClientWriteResponse { data }
                                              │
                                           Response

Followers apply via their own RaftStateMachine::apply() ─ asynchronously!
Leader does NOT wait for followers to apply, only for quorum to ACK.
```

**Key OpenRaft Guarantees:**
- Replication to followers happens IN PARALLEL (OpenRaft internal)
- We wait for QUORUM (not all followers) before committing
- All nodes apply via the SAME `CommandExecutorImpl` in `RaftStateMachine::apply()`
- `client_write()` returns after leader applies (post-quorum-commit)
- Followers apply asynchronously — may lag slightly behind leader

#### Cluster Mode (Follower) — Forward to Leader
```
Client ─► Handler ─► ClusterApplier ─► check metrics.current_leader
                                              │
                                     (Not leader ─ forward!)
                                              │
                           CommandForwarder.forward_and_wait(leader, cmd)
                                              │
                                     ┌───────┴───────┐
                                     │  gRPC to Leader │
                                     ├─────────────────┤
                                     │ Leader does:    │
                                     │ client_write()  │
                                     │ (full Raft)     │
                                     └─────────────────┘
                                              │
                                     Leader returns result
                                              │
                                     Follower returns to Client
```

**Note:** The follower does NOT wait for its own `RaftStateMachine::apply()` call.
It gets the result directly from the leader after the leader's Raft commit.
The follower will eventually apply the entry when OpenRaft replicates it.

---

## 3. Design Details

### 3.1 Command Trait

```rust
/// A command that can be applied to the database
pub trait Command: Send + Sync + 'static {
    /// The result type of this command
    type Result: Send + Sync;
    
    /// Unique command type identifier for routing
    fn command_type(&self) -> CommandType;
    
    /// Validate the command before execution
    fn validate(&self, ctx: &ValidationContext) -> Result<(), CommandError>;
    
    /// Serialize for Raft replication
    fn serialize(&self) -> Result<Vec<u8>, CommandError>;
    
    /// Deserialize from Raft log
    fn deserialize(bytes: &[u8]) -> Result<Self, CommandError> where Self: Sized;
}
```

### 3.2 CommandType Enum

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandType {
    // DDL Commands
    CreateNamespace,
    DropNamespace,
    CreateTable,
    AlterTable,
    DropTable,
    CreateStorage,
    DropStorage,
    
    // DML Commands (routed by shard)
    InsertRows { shard_id: u8 },
    UpdateRows { shard_id: u8 },
    DeleteRows { shard_id: u8 },
    
    // User Management
    CreateUser,
    UpdateUser,
    DeleteUser,
    
    // Job Management
    CreateJob,
    UpdateJobStatus,
    
    // Live Queries
    RegisterLiveQuery { shard_id: u8 },
    UnregisterLiveQuery { shard_id: u8 },
}
```

### 3.3 Unified Applier Trait

```rust
#[async_trait]
pub trait UnifiedApplier: Send + Sync {
    /// Apply a command through the unified execution path
    /// 
    /// This method handles:
    /// - Standalone: Direct execution
    /// - Cluster Leader: Execute + replicate
    /// - Cluster Follower: Forward to leader
    async fn apply<C: Command>(&self, cmd: C) -> Result<C::Result, ApplierError>;
    
    /// Check if this node can accept writes for a command type
    fn can_accept_writes(&self, cmd_type: CommandType) -> bool;
    
    /// Get the leader address for forwarding (cluster mode only)
    fn get_leader_for(&self, cmd_type: CommandType) -> Option<NodeAddr>;
}
```

### 3.4 Applier Implementations

#### StandaloneApplier
```rust
pub struct StandaloneApplier {
    executor: Arc<CommandExecutorImpl>,
}

#[async_trait]
impl UnifiedApplier for StandaloneApplier {
    async fn apply<C: Command>(&self, cmd: C) -> Result<C::Result, ApplierError> {
        // Validate
        cmd.validate(&self.validation_context())?;
        
        // Execute directly (single node, no replication)
        self.executor.execute(cmd).await
    }
    
    fn can_accept_writes(&self, _cmd_type: CommandType) -> bool {
        true // Standalone always accepts writes
    }
    
    fn get_leader_for(&self, _cmd_type: CommandType) -> Option<NodeAddr> {
        None // No leader in standalone mode
    }
}
```

#### ClusterApplier
```rust
pub struct ClusterApplier {
    raft_manager: Arc<RaftManager>,
    forwarder: Arc<CommandForwarder>,
}

// NOTE: No executor field! Execution happens inside RaftStateMachine::apply()
// The executor is owned by the state machine, not the applier.

#[async_trait]
impl UnifiedApplier for ClusterApplier {
    async fn apply<C: Command>(&self, cmd: C) -> Result<C::Result, ApplierError> {
        let group_id = self.route_to_group(cmd.command_type());
        
        // Validate first (cheap, can do on any node)
        cmd.validate(&self.validation_context())?;
        
        // Get the Raft instance for this group
        let raft = self.raft_manager.get_raft(group_id)?;
        
        // Check if we're the leader using OpenRaft metrics
        let metrics = raft.metrics().borrow().clone();
        
        if metrics.current_leader == Some(self.raft_manager.node_id()) {
            // LEADER PATH: Use OpenRaft's client_write()
            // This is the ONLY way to propose — never manually append logs
            
            let serialized = cmd.serialize()?;
            
            // client_write() does everything:
            // 1. Appends to log
            // 2. Replicates to followers IN PARALLEL (OpenRaft internal)
            // 3. Waits for QUORUM acknowledgment
            // 4. Commits the entry
            // 5. Applies to state machine via RaftStateMachine::apply()
            // 6. Returns when leader has applied
            let response: ClientWriteResponse<TypeConfig> = raft
                .client_write(serialized)
                .await
                .map_err(|e| match e {
                    RaftError::APIError(ClientWriteError::ForwardToLeader(info)) => {
                        // OpenRaft tells us to forward — leader changed!
                        ApplierError::ForwardToLeader(info.leader_node)
                    }
                    other => ApplierError::Raft(other.to_string()),
                })?;
            
            // response.data contains the result from state machine apply()
            // Deserialize and return
            C::Result::deserialize(&response.data)
                .map_err(|e| ApplierError::Deserialization(e.to_string()))
        } else {
            // FOLLOWER PATH: Forward to leader and wait for response
            let leader_node = metrics.current_leader
                .and_then(|id| self.raft_manager.get_node_addr(id))
                .ok_or(ApplierError::NoLeader)?;
            
            self.forwarder.forward_and_wait(leader_node, cmd).await
        }
    }
    
    fn can_accept_writes(&self, cmd_type: CommandType) -> bool {
        let group_id = self.route_to_group(cmd_type);
        if let Ok(raft) = self.raft_manager.get_raft(group_id) {
            let metrics = raft.metrics().borrow().clone();
            metrics.current_leader == Some(self.raft_manager.node_id())
        } else {
            false
        }
    }
    
    fn get_leader_for(&self, cmd_type: CommandType) -> Option<NodeAddr> {
        let group_id = self.route_to_group(cmd_type);
        let raft = self.raft_manager.get_raft(group_id).ok()?;
        let metrics = raft.metrics().borrow().clone();
        metrics.current_leader
            .and_then(|id| self.raft_manager.get_node_addr(id))
    }
}
```

### 3.5 Command Executor (Shared Logic)

```rust
/// Executes commands after validation and consensus
/// This is the SINGLE place where database mutations happen
pub struct CommandExecutorImpl {
    app_context: Arc<AppContext>,
}

impl CommandExecutorImpl {
    /// Execute a CREATE TABLE command
    pub async fn execute_create_table(&self, cmd: CreateTableCommand) -> Result<String, CommandError> {
        // 1. Persist to system.tables
        self.app_context.system_tables().tables()
            .create_table(&cmd.table_id, &cmd.table_def)?;
        
        // 2. Prime schema cache
        self.prime_schema_cache(&cmd.table_id, &cmd.table_def)?;
        
        // 3. Register DataFusion provider
        self.register_table_provider(&cmd.table_id, &cmd.table_def)?;
        
        // 4. Emit event (for audit log, live queries, etc.)
        self.emit_event(DatabaseEvent::TableCreated {
            table_id: cmd.table_id.clone(),
            table_type: cmd.table_def.table_type,
        }).await;
        
        Ok(format!("Table {} created successfully", cmd.table_id))
    }
    
    /// Execute an ALTER TABLE command
    pub async fn execute_alter_table(&self, cmd: AlterTableCommand) -> Result<String, CommandError> {
        // Same logic for both standalone and cluster modes
        // ...
    }
}
```

---

## 4. Handler Refactoring

### Before (Current)
```rust
impl TypedStatementHandler<CreateTableStatement> for CreateTableHandler {
    async fn execute(&self, statement: CreateTableStatement, ...) -> Result<ExecutionResult, KalamDbError> {
        // Validation and table type resolution...
        
        let message = if self.app_context.executor().is_cluster_mode() {
            let table_def = table_creation::build_table_definition(...)?;
            // ... cluster-specific code
            self.app_context.executor().execute_meta(cmd).await?;
            format!("...")
        } else {
            table_creation::create_table(...)?
        };
        
        // Audit logging...
        Ok(ExecutionResult::Success { message })
    }
}
```

### After (Unified)
```rust
impl TypedStatementHandler<CreateTableStatement> for CreateTableHandler {
    async fn execute(&self, statement: CreateTableStatement, ...) -> Result<ExecutionResult, KalamDbError> {
        // Build command (validation happens inside applier)
        let cmd = CreateTableCommand::from_statement(statement, context)?;
        
        // Apply through unified path (works same for standalone/cluster)
        let message = self.app_context.applier().apply(cmd).await?;
        
        // Audit logging happens inside executor via events
        Ok(ExecutionResult::Success { message })
    }
}
```

---

## 5. Event System Integration

### 5.1 Database Events

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DatabaseEvent {
    // DDL Events
    NamespaceCreated { namespace_id: NamespaceId },
    NamespaceDropped { namespace_id: NamespaceId },
    TableCreated { table_id: TableId, table_type: TableType },
    TableAltered { table_id: TableId, old_version: u32, new_version: u32 },
    TableDropped { table_id: TableId },
    
    // DML Events
    RowsInserted { table_id: TableId, count: usize, user_id: Option<UserId> },
    RowsUpdated { table_id: TableId, count: usize, user_id: Option<UserId> },
    RowsDeleted { table_id: TableId, count: usize, user_id: Option<UserId> },
    
    // User Events
    UserCreated { user_id: UserId },
    UserUpdated { user_id: UserId },
    UserDeleted { user_id: UserId },
}
```

### 5.2 Event Handlers

```rust
/// Subscribe to database events for side effects
#[async_trait]
pub trait EventHandler: Send + Sync {
    async fn handle(&self, event: &DatabaseEvent) -> Result<(), EventError>;
}

// Built-in handlers:
pub struct AuditLogHandler { ... }      // Writes to system.audit_logs
pub struct LiveQueryHandler { ... }     // Notifies live query subscribers
pub struct MetricsHandler { ... }       // Updates Prometheus metrics
pub struct CacheInvalidationHandler { ... } // Invalidates caches
```

---

## 6. Library Recommendations

### Option A: Custom Implementation (Recommended)

**Build a lightweight applier using existing infrastructure:**

| Component | Implementation |
|-----------|----------------|
| Command Routing | Existing `GroupId` enum + `ShardRouter` |
| Consensus | Existing `RaftManager` + OpenRaft |
| Forwarding | Existing gRPC infrastructure (tonic) |
| Events | `tokio::sync::broadcast` for in-process events |
| Parallel Replication | OpenRaft handles this automatically |

**Pros:**
- Full control over behavior
- No new dependencies
- Tailored to KalamDB's architecture
- Reuses existing Raft infrastructure
- OpenRaft already does parallel async replication

**Cons:**
- More implementation effort
- Must handle edge cases ourselves

### OpenRaft Best Practices to Follow

1. **Use `Raft::client_write()` for proposals** — don't manually manage log
2. **Trust OpenRaft's replication** — it parallelizes follower updates automatically
3. **Wait for `ClientWriteResponse`** — this means quorum committed
4. **Apply in state machine** — all nodes apply the same way via `RaftStateMachine::apply()`
5. **Use `RaftMetrics` for leader detection** — don't roll your own
6. **Handle `ForwardToLeader` error** — forward and retry transparently

---

## 6.5 OpenRaft Integration Details

### Key OpenRaft APIs We Use

```rust
// 1. PROPOSING WRITES (Leader only)
// client_write() handles everything: log append, replication, quorum wait, apply
let response: ClientWriteResponse<C> = raft.client_write(command_bytes).await?;
// response.log_id: LogId of the committed entry
// response.data: Result from RaftStateMachine::apply()

// 2. LEADER DETECTION
// Use metrics channel for real-time leader info
let metrics: RaftMetrics<NodeId, Node> = raft.metrics().borrow().clone();
let current_leader: Option<NodeId> = metrics.current_leader;
let am_i_leader: bool = current_leader == Some(my_node_id);

// 3. LINEARIZABLE READS (for consistent reads)
// Ensures we're still the leader and state machine is up-to-date
let read_log_id = raft.ensure_linearizable().await?;
// Now safe to read from local state machine

// 4. ERROR HANDLING
// ClientWriteError::ForwardToLeader tells us to forward to actual leader
match raft.client_write(cmd).await {
    Ok(response) => /* success */,
    Err(RaftError::APIError(ClientWriteError::ForwardToLeader(info))) => {
        // info.leader_id and info.leader_node have leader info
        // Forward the request there
    }
    Err(other) => /* handle error */,
}
```

### State Machine Integration

```rust
// RaftStateMachine::apply() is called by OpenRaft after commit
// This is where CommandExecutorImpl lives!

impl RaftStateMachine<TypeConfig> for KalamStateMachine {
    async fn apply<I>(&mut self, entries: I) -> Vec<Response>
    where
        I: IntoIterator<Item = Entry<TypeConfig>>,
    {
        let mut results = Vec::new();
        for entry in entries {
            match entry.payload {
                EntryPayload::Normal(command_bytes) => {
                    // Deserialize command
                    let cmd = Command::deserialize(&command_bytes)?;
                    
                    // Execute via CommandExecutorImpl
                    // THIS IS THE SINGLE EXECUTION POINT!
                    let result = self.executor.execute(cmd).await;
                    
                    // Serialize result for response
                    results.push(result.serialize());
                }
                EntryPayload::Membership(_) => {
                    // Membership changes handled by OpenRaft
                    results.push(Vec::new());
                }
            }
        }
        results
    }
}
```

### What We DON'T Implement

| Feature | OpenRaft Handles It |
|---------|--------------------|
| Log replication | ✓ Automatic parallel async replication |
| Quorum counting | ✓ Majority acknowledgment tracking |
| Commit index advancement | ✓ Tracks committed vs applied |
| Follower catch-up | ✓ Automatic log/snapshot sync |
| Leader election | ✓ Raft protocol with configurable timeouts |
| Heartbeats | ✓ Periodic AppendEntries RPCs |
| Snapshot transfer | ✓ InstallSnapshot RPC |

### Configuration (server.toml)

```toml
[cluster]
# These are the ONLY cluster settings needed:
cluster_id = "cluster"
node_id = 1
rpc_addr = "0.0.0.0:2910"
api_addr = "0.0.0.0:2900"

# OpenRaft Config (passed to openraft::Config)
heartbeat_interval_ms = 50           # How often leader sends heartbeats
election_timeout_ms = [150, 300]     # Random timeout for election
snapshot_threshold = 10000           # Entries before snapshot

# Peer nodes
[[cluster.peers]]
node_id = 2
rpc_addr = "10.0.0.2:2910"
api_addr = "http://10.0.0.2:2900"

# NOTE: No min_replication_nodes setting!
# OpenRaft uses standard Raft quorum: (N/2)+1
# For 3 nodes: quorum = 2
# For 5 nodes: quorum = 3
# This is hardcoded in Raft and cannot be configured.
```

### Removed Settings (Not Needed)

| Setting | Why Removed |
|---------|-------------|
| `min_replication_nodes` | OpenRaft uses fixed quorum: (N/2)+1 |
| `replication_timeout` | OpenRaft handles internally |
| `max_entries_per_request` | OpenRaft batches automatically |
| `wait_for_all_followers` | Raft never waits for all, only quorum |

### Option B: Actix Actors

Use Actix actor model for command routing:

```rust
// Example with actix
pub struct ApplierActor {
    executor: Arc<CommandExecutorImpl>,
    raft: Option<Arc<RaftManager>>,
}

impl Actor for ApplierActor {
    type Context = Context<Self>;
}

impl Handler<CreateTableCommand> for ApplierActor {
    type Result = ResponseFuture<Result<String, ApplierError>>;
    
    fn handle(&mut self, cmd: CreateTableCommand, _ctx: &mut Context<Self>) -> Self::Result {
        // Route and execute...
    }
}
```

**Pros:**
- Battle-tested actor model
- Built-in message passing
- Supervision and fault tolerance

**Cons:**
- Heavy dependency (already using Actix-Web though)
- Actor overhead for simple commands
- Different programming model

### Option C: Tower Service Pattern

Use Tower's `Service` trait for composable middleware:

```rust
pub struct ApplierService<S> {
    inner: S,
    raft: Option<Arc<RaftManager>>,
}

impl<S, Cmd> Service<Cmd> for ApplierService<S>
where
    S: Service<Cmd>,
    Cmd: Command,
{
    type Response = S::Response;
    type Error = ApplierError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;
    
    fn call(&mut self, cmd: Cmd) -> Self::Future {
        // Validation layer, routing layer, execution layer...
    }
}
```

**Pros:**
- Composable middleware (validation, logging, metrics)
- Industry-standard pattern
- Works well with async

**Cons:**
- Adds tower dependency
- More abstract than needed for our use case

### Recommendation: Option A (Custom)

Given that:
1. We already have `RaftManager`, `CommandExecutor`, and gRPC infrastructure
2. We need tight integration with existing state machines
3. The applier logic is specific to KalamDB's architecture
4. We want minimal new dependencies

**A custom `UnifiedApplier` implementation is the best choice.**

---

## 7. Implementation Plan

### Phase 1: Foundation (Est. 2-3 days)
- [ ] Create `applier/` module structure with small, focused files
- [ ] Define `Command` trait and `CommandType` enum in `command.rs`
- [ ] Define `ApplierError` in `error.rs`
- [ ] Define `UnifiedApplier` trait in `traits.rs`

### Phase 2: Command Executor (Est. 3-4 days)
- [ ] Create `executor/mod.rs` with `CommandExecutorImpl`
- [ ] Create `executor/ddl.rs` — CREATE/ALTER/DROP TABLE execution
- [ ] Create `executor/namespace.rs` — namespace execution
- [ ] Create `executor/storage.rs` — storage execution
- [ ] Create `executor/user.rs` — user management execution
- [ ] **DELETE** duplicate logic from handlers and old appliers

### Phase 3: Applier Implementations (Est. 3-4 days)
- [ ] Create `standalone.rs` — `StandaloneApplier` (direct execution)
- [ ] Create `cluster.rs` — `ClusterApplier` (Raft + forwarding)
- [ ] Create `forwarder.rs` — gRPC forwarding for followers
- [ ] Wire to `AppContext.applier()`
- [ ] **DELETE** `is_cluster_mode()` checks from all handlers

### Phase 4: Handler Simplification (Est. 2-3 days)
- [ ] Simplify CREATE TABLE handler (build command → call applier)
- [ ] Simplify ALTER TABLE handler
- [ ] Simplify DROP TABLE handler
- [ ] Simplify all DDL/DML/User handlers
- [ ] **DELETE** `table_creation.rs` (logic moved to executor)
- [ ] **DELETE** `build_table_definition()`, `apply_altered_table_locally()`

### Phase 5: Cleanup & Delete Old Code (Est. 2-3 days)
- [ ] **DELETE** `ProviderMetaApplier` (replaced by `CommandExecutorImpl`)
- [ ] **DELETE** all mode-specific code paths
- [ ] **DELETE** unused helper functions
- [ ] **DELETE** dead imports and modules
- [ ] Run `cargo clippy` — fix all warnings
- [ ] Run `cargo fmt` — ensure consistent style

### Phase 6: Event System (Est. 1-2 days)
- [ ] Create `events/mod.rs` with `DatabaseEvent` enum
- [ ] Create `events/broadcast.rs` with channel setup
- [ ] Create `events/handlers/audit.rs`
- [ ] Create `events/handlers/live_query.rs`
- [ ] **DELETE** inline audit logging from handlers

### Phase 7: Testing & Validation (Est. 2-3 days)
- [ ] Verify 99 smoke tests pass
- [ ] Verify 47 cluster tests pass
- [ ] Add new unit tests for `CommandExecutorImpl`
- [ ] Benchmark latency (must be < 5% regression)
- [ ] **DELETE** obsolete tests for removed code

**Total Estimated Effort: 15-22 days**

### Code Deletion Checklist

After implementation, these MUST be deleted:

| File/Function | Reason |
|--------------|--------|
| `provider_meta_applier.rs` | Replaced by `CommandExecutorImpl` |
| `table_creation::create_table()` | Logic in executor |
| `table_creation::build_table_definition()` | Logic in executor |
| `alter.rs::apply_altered_table_locally()` | Logic in executor |
| All `is_cluster_mode()` in handlers | No mode branching |
| Handler-level audit logging | Moved to event handler |
| Duplicate cache priming code | Single source in executor |

---

## 8. Migration Strategy

### Direct Replacement (No Feature Flags)

We will NOT use feature flags or incremental rollout. Instead:

1. **Build complete new applier** with all command support
2. **Switch entirely** in one commit
3. **Delete all old code** immediately
4. **Run full test suite** (99 smoke + 47 cluster)
5. **No backward compatibility** — clean break

**Rationale:**
- Feature flags add complexity and maintenance burden
- Parallel paths lead to divergence and bugs
- A clean break is simpler to reason about
- Tests catch any regressions immediately

### What Gets Deleted

| Category | Items to Delete |
|----------|----------------|
| Handlers | All `is_cluster_mode()` branching |
| Helpers | `table_creation.rs` (most of it), `build_table_definition()` |
| Appliers | `ProviderMetaApplier` (entire file) |
| Handler methods | `apply_altered_table_locally()`, etc. |
| Dead code | Any unused functions after migration |

---

## 9. Benefits

| Benefit | Impact |
|---------|--------|
| **Single code path** | Fewer bugs, easier maintenance |
| **Simpler handlers** | Handlers just build commands |
| **Consistent behavior** | Same code for standalone/cluster |
| **Better testing** | Test executor once, not twice |
| **Event-driven** | Easy to add new side effects |
| **Cleaner architecture** | Clear separation of concerns |

---

## 10. Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| Performance regression | Benchmark before/after; optimize hot paths |
| Forwarding latency | Connection pooling in `CommandForwarder`; reuse gRPC channels |
| Leader failover during command | Handle `ForwardToLeader` error; retry once with new leader |
| Non-deterministic state machine | Strict testing; same input → same output always |
| Slow follower delays leader | OpenRaft already handles this — leader doesn't wait for slow nodes |

**Removed Risks (OpenRaft handles):**
- Quorum calculation errors → OpenRaft hardcodes (N/2)+1
- Replication ordering → OpenRaft log ensures order
- Partial replication → OpenRaft tracks matched index per follower

---

## 11. Success Criteria

- [ ] All 47 cluster tests pass
- [ ] All 99 smoke tests pass
- [ ] No mode-specific branching in handlers
- [ ] Single `CommandExecutorImpl` for all execution
- [ ] Event-based audit logging working
- [ ] Latency regression < 5%

---

## 12. Open Questions (Resolved)

### 1. Command batching: Should we support batching multiple commands in one Raft proposal?
**Answer**: Not initially. OpenRaft handles batching internally when multiple `client_write()` calls happen concurrently. Start simple with single commands; add explicit batching later if benchmarks show need.

### 2. Read-your-writes: How to ensure a follower sees its own writes immediately after forwarding?
**Answer**: Use `raft.ensure_linearizable()` before reads. This confirms leadership and waits for state machine to catch up to committed index. For followers, they should either:
- Route reads to leader (simplest)
- Wait for their local `last_applied` to reach the `log_id` returned by the forwarded write

### 3. Partial failures: If execution succeeds on leader but fails on a follower, how to handle?
**Answer**: This is handled by Raft's determinism requirement. The same command applied to the same state MUST produce the same result. If a follower's apply fails, it's a bug in our state machine (non-determinism) and the follower should crash and recover via snapshot. We never have "partial success" in a correct Raft implementation.

### 4. Idempotency: How to handle duplicate commands (client retry)?
**Answer**: Use command IDs (UUIDs) and track in state machine. OpenRaft docs recommend:
```rust
// In state machine
struct KalamStateMachine {
    // Track last command per client to detect duplicates
    client_responses: HashMap<ClientId, (CommandId, Response)>,
}

// In apply()
if let Some((last_cmd_id, cached_response)) = self.client_responses.get(&cmd.client_id) {
    if *last_cmd_id == cmd.command_id {
        return cached_response.clone(); // Return cached, don't re-execute
    }
}
```

---

## Appendix A: File Structure

```
backend/crates/kalamdb-core/src/
├── applier/
│   ├── mod.rs                    # Module exports only (< 50 lines)
│   ├── traits.rs                 # UnifiedApplier trait (< 100 lines)
│   ├── command.rs                # Command trait, CommandType enum (< 150 lines)
│   ├── error.rs                  # ApplierError types (< 100 lines)
│   ├── standalone.rs             # StandaloneApplier (< 100 lines)
│   ├── cluster.rs                # ClusterApplier (< 200 lines)
│   ├── forwarder.rs              # gRPC forwarding (< 150 lines)
│   │
│   ├── executor/                 # Shared execution logic
│   │   ├── mod.rs                # CommandExecutorImpl struct (< 100 lines)
│   │   ├── ddl.rs                # CREATE/ALTER/DROP TABLE (< 200 lines)
│   │   ├── namespace.rs          # Namespace operations (< 100 lines)
│   │   ├── storage.rs            # Storage operations (< 100 lines)
│   │   ├── user.rs               # User management (< 150 lines)
│   │   └── dml.rs                # INSERT/UPDATE/DELETE (< 200 lines)
│   │
│   └── events/                   # Event system
│       ├── mod.rs                # DatabaseEvent enum (< 100 lines)
│       ├── broadcast.rs          # Channel setup (< 50 lines)
│       └── handlers/
│           ├── mod.rs
│           ├── audit.rs          # Audit log handler (< 100 lines)
│           └── live_query.rs     # Live query notifications (< 100 lines)
│
├── sql/executor/handlers/        # SIMPLIFIED handlers
│   ├── table/
│   │   ├── create.rs             # Build command → call applier (< 80 lines)
│   │   ├── alter.rs              # Build command → call applier (< 80 lines)
│   │   └── drop.rs               # Build command → call applier (< 80 lines)
│   └── ...                       # All handlers become thin wrappers
```

### File Size Guidelines

| File Type | Max Lines | Rationale |
|-----------|-----------|----------|
| `mod.rs` | 50 | Just exports |
| Trait definitions | 100 | Interface only |
| Error types | 100 | Just enums |
| Applier implementations | 200 | Routing + forwarding |
| Executor methods | 200 | Business logic per command type |
| Handlers | 80 | Thin wrappers only |

---

## Appendix B: Example Command Definitions

```rust
// commands/ddl.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTableCommand {
    pub table_id: TableId,
    pub table_def: TableDefinition,
    pub created_by: UserId,
}

impl Command for CreateTableCommand {
    type Result = String;
    
    fn command_type(&self) -> CommandType {
        CommandType::CreateTable
    }
    
    fn validate(&self, ctx: &ValidationContext) -> Result<(), CommandError> {
        // Validate namespace exists
        if !ctx.namespace_exists(&self.table_id.namespace_id()) {
            return Err(CommandError::NamespaceNotFound(self.table_id.namespace_id()));
        }
        
        // Validate table doesn't exist
        if ctx.table_exists(&self.table_id) {
            return Err(CommandError::TableAlreadyExists(self.table_id.clone()));
        }
        
        // Validate RBAC
        if !ctx.can_create_table(self.table_def.table_type) {
            return Err(CommandError::Unauthorized);
        }
        
        Ok(())
    }
    
    fn serialize(&self) -> Result<Vec<u8>, CommandError> {
        bincode::serde::encode_to_vec(self, bincode::config::standard())
            .map_err(|e| CommandError::Serialization(e.to_string()))
    }
    
    fn deserialize(bytes: &[u8]) -> Result<Self, CommandError> {
        bincode::serde::decode_from_slice(bytes, bincode::config::standard())
            .map(|(cmd, _)| cmd)
            .map_err(|e| CommandError::Deserialization(e.to_string()))
    }
}
```

---

## References

### OpenRaft (Primary)
- [OpenRaft Docs](https://docs.rs/openraft/latest/openraft/) — Main documentation
- [OpenRaft Getting Started](https://docs.rs/openraft/latest/openraft/docs/getting_started/index.html) — Tutorial
- [OpenRaft FAQ](https://docs.rs/openraft/latest/openraft/docs/faq/index.html) — Common questions
- [Raft::client_write()](https://docs.rs/openraft/latest/openraft/raft/struct.Raft.html#method.client_write) — Write API
- [Raft::ensure_linearizable()](https://docs.rs/openraft/latest/openraft/raft/struct.Raft.html#method.ensure_linearizable) — Consistent reads
- [RaftMetrics](https://docs.rs/openraft/latest/openraft/metrics/struct.RaftMetrics.html) — Cluster observability
- [ClientWriteResponse](https://docs.rs/openraft/latest/openraft/raft/struct.ClientWriteResponse.html) — Write result
- [RaftStateMachine](https://docs.rs/openraft/latest/openraft/storage/trait.RaftStateMachine.html) — State machine trait

### Raft Protocol
- [Raft Paper (PDF)](https://raft.github.io/raft.pdf) — Original paper
- [Raft Visualization](https://raft.github.io/) — Interactive demo

### KalamDB Internal
- [AGENTS.md](../../AGENTS.md) — Development guidelines
- [kalamdb-raft/src/group_id.rs](../../backend/crates/kalamdb-raft/src/group_id.rs) — GroupId routing
- [kalamdb-raft/src/manager/raft_manager.rs](../../backend/crates/kalamdb-raft/src/manager/raft_manager.rs) — Raft group management
