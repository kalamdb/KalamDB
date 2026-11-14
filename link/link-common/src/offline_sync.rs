//! Portable, storage-agnostic offline synchronization decisions.
//!
//! This module is deliberately free of SQLite, Tokio, Flutter and browser APIs.
//! The caller must apply each decision and its durable progress in one local
//! transaction. It is NOT itself a durable sync engine or an idempotency service.

use std::collections::{HashMap, HashSet};

use kalamdb_commons::ids::{VersionDomain, VersionId};
use serde::{Deserialize, Serialize};

/// Mirrors the durable outbox state persisted by platform-specific adapters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboxStatus {
    Queued,
    Running,
    RetryScheduled,
    Failed,
    Succeeded,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxCandidate {
    pub id: String,
    pub account_key: String,
    pub ordering_key: Option<String>,
    pub queue_position: u64,
    pub status: OutboxStatus,
    /// UTC epoch milliseconds, not a row VersionId.
    pub next_attempt_at_ms: Option<i64>,
}

/// Pass at most the earliest *nonterminal* action per key when the local
/// database can calculate these heads. The planner also works on full queues.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxPlanRequest {
    pub account_key: String,
    pub now_ms: i64,
    pub max_claims: usize,
    #[serde(default)]
    pub in_flight_keys: Vec<String>,
    pub actions: Vec<OutboxCandidate>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxPlan {
    /// At most one claimed action per ordering key, ordered by queue position.
    pub action_ids: Vec<String>,
    /// Earliest known retry time among waiting key heads.
    pub next_retry_at_ms: Option<i64>,
    /// Failed heads block subsequent dependent actions until explicitly resolved.
    pub failed_keys: Vec<String>,
    /// Malformed retry records fail closed instead of being executed early.
    pub missing_retry_deadline_keys: Vec<String>,
}

/// Compute a deterministic set of safe-to-claim actions.
///
/// A retry-delayed, running, or failed head is a barrier: later actions with the
/// same ordering key must not pass it. Missing keys share the empty-string
/// bucket, matching the current Dart outbox contract. The caller must claim
/// returned IDs atomically in storage before dispatch and fence other runners.
pub fn plan_outbox(request: &OutboxPlanRequest) -> OutboxPlan {
    let blocked: HashSet<&str> =
        request.in_flight_keys.iter().map(String::as_str).collect();
    let mut heads: HashMap<&str, &OutboxCandidate> = HashMap::new();

    for action in &request.actions {
        if action.account_key != request.account_key
            || matches!(action.status, OutboxStatus::Succeeded | OutboxStatus::Cancelled)
        {
            continue;
        }

        let key = action.ordering_key.as_deref().unwrap_or("");
        let replace = heads.get(key).is_none_or(|current| {
            (action.queue_position, action.id.as_str())
                < (current.queue_position, current.id.as_str())
        });
        if replace {
            heads.insert(key, action);
        }
    }

    let mut ordered: Vec<(&str, &OutboxCandidate)> = heads.into_iter().collect();
    ordered.sort_unstable_by(|(_, left), (_, right)| {
        (left.queue_position, left.id.as_str())
            .cmp(&(right.queue_position, right.id.as_str()))
    });

    let mut result = OutboxPlan::default();
    for (key, action) in ordered {
        if blocked.contains(key) {
            continue;
        }

        match action.status {
            OutboxStatus::Queued => {
                if result.action_ids.len() < request.max_claims {
                    result.action_ids.push(action.id.clone());
                }
            }
            OutboxStatus::RetryScheduled => match action.next_attempt_at_ms {
                Some(when) if when <= request.now_ms => {
                    if result.action_ids.len() < request.max_claims {
                        result.action_ids.push(action.id.clone());
                    }
                }
                Some(when) => {
                    result.next_retry_at_ms = Some(
                        result.next_retry_at_ms.map_or(when, |earliest| earliest.min(when)),
                    );
                }
                None => result.missing_retry_deadline_keys.push(key.to_string()),
            },
            OutboxStatus::Failed => result.failed_keys.push(key.to_string()),
            OutboxStatus::Running | OutboxStatus::Succeeded | OutboxStatus::Cancelled => {}
        }
    }
    result.failed_keys.sort();
    result.missing_retry_deadline_keys.sort();
    result
}

/// JSON ABI suitable for one batched Dart FRB or TypeScript/WASM call.
pub fn plan_outbox_json(request_json: &str) -> Result<String, String> {
    let request: OutboxPlanRequest =
        serde_json::from_str(request_json).map_err(|e| e.to_string())?;
    serde_json::to_string(&plan_outbox(&request)).map_err(|e| e.to_string())
}

/// A checkpoint is valid only for the exact account, query and version domain.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayScope {
    pub account_key: String,
    pub subscription_id: String,
    /// Hash over canonical SQL, bound parameters and the projection policy.
    pub query_fingerprint: String,
    /// Required for SHARED tables; preserve the exact server-provided envelope.
    pub version_domain: Option<VersionDomain>,
}

/// Persist this state with staged snapshot rows, not only in memory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum ReplayState {
    Empty,
    Live {
        scope: ReplayScope,
        cursor: Option<VersionId>,
    },
    Loading {
        scope: ReplayScope,
        generation: String,
        next_page: u32,
        last_page_cursor: Option<VersionId>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReplayFrame {
    /// A *confirmed full* snapshot starts a new staging projection.
    BeginSnapshot {
        scope: ReplayScope,
        generation: String,
    },
    /// Page order is explicit, including a final page with zero rows.
    SnapshotPage {
        scope: ReplayScope,
        generation: String,
        page_index: u32,
        is_last_page: bool,
        /// Server-advertised page progress, NOT guessed from row versions.
        cursor: Option<VersionId>,
    },
    /// Incremental replay or live change: never clear the existing projection.
    Delta {
        scope: ReplayScope,
        /// Server-advertised ordered prefix for this domain.
        cursor: VersionId,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayAction {
    BeginStaging,
    AppendSnapshotPage,
    FinishSnapshotAndSwap,
    ApplyDelta,
    IgnoreDuplicate,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayPlan {
    /// Commit this state in the same transaction as its corresponding rows.
    pub state: ReplayState,
    pub action: ReplayAction,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayPlanRequest {
    pub state: ReplayState,
    pub frame: ReplayFrame,
}

fn validate_scope(scope: &ReplayScope) -> Result<(), String> {
    if scope.account_key.is_empty()
        || scope.subscription_id.is_empty()
        || scope.query_fingerprint.is_empty()
    {
        return Err("replay scope needs account, subscription and query fingerprint".into());
    }
    Ok(())
}

/// Validate snapshot/delta transitions without touching local storage.
///
/// Never interpret a cursor-based replay as a replacement snapshot. A snapshot
/// is staged and swapped only upon the final, server-confirmed page. The
/// storage adapter is responsible for atomic staging, checkpoint persistence,
/// and mutation reconciliation.
pub fn plan_replay(request: &ReplayPlanRequest) -> Result<ReplayPlan, String> {
    match (&request.state, &request.frame) {
        (state, ReplayFrame::BeginSnapshot { scope, generation }) => {
            validate_scope(scope)?;
            if generation.is_empty() {
                return Err("a snapshot requires a stable generation ID".into());
            }
            if let ReplayState::Loading {
                scope: active,
                generation: current,
                ..
            } = state
            {
                if active == scope && current == generation {
                    return Ok(ReplayPlan {
                        state: state.clone(),
                        action: ReplayAction::IgnoreDuplicate,
                    });
                }
            }
            Ok(ReplayPlan {
                state: ReplayState::Loading {
                    scope: scope.clone(),
                    generation: generation.clone(),
                    next_page: 0,
                    last_page_cursor: None,
                },
                action: ReplayAction::BeginStaging,
            })
        }
        (
            ReplayState::Loading {
                scope: active,
                generation: current,
                next_page,
                last_page_cursor,
            },
            ReplayFrame::SnapshotPage {
                scope,
                generation,
                page_index,
                is_last_page,
                cursor,
            },
        ) => {
            if active != scope || current != generation {
                return Err("snapshot scope or generation mismatch; rebootstrap required".into());
            }
            if page_index < next_page {
                return Ok(ReplayPlan {
                    state: request.state.clone(),
                    action: ReplayAction::IgnoreDuplicate,
                });
            }
            if page_index != next_page {
                return Err("snapshot page gap; do not advance the checkpoint".into());
            }
            if let (Some(previous), Some(incoming)) = (last_page_cursor, cursor) {
                if incoming < previous {
                    return Err("snapshot cursor moved backwards".into());
                }
            }
            let latest = cursor.or(*last_page_cursor);
            if *is_last_page {
                Ok(ReplayPlan {
                    state: ReplayState::Live {
                        scope: scope.clone(),
                        cursor: latest,
                    },
                    action: ReplayAction::FinishSnapshotAndSwap,
                })
            } else {
                Ok(ReplayPlan {
                    state: ReplayState::Loading {
                        scope: scope.clone(),
                        generation: generation.clone(),
                        next_page: next_page.checked_add(1).ok_or("snapshot page overflow")?,
                        last_page_cursor: latest,
                    },
                    action: ReplayAction::AppendSnapshotPage,
                })
            }
        }
        (ReplayState::Live { scope: active, cursor: committed }, ReplayFrame::Delta { scope, cursor }) => {
            if active != scope {
                return Err("replay scope or VersionDomain changed; rebootstrap required".into());
            }
            if committed.is_some_and(|old| *cursor <= old) {
                return Ok(ReplayPlan {
                    state: request.state.clone(),
                    action: ReplayAction::IgnoreDuplicate,
                });
            }
            Ok(ReplayPlan {
                state: ReplayState::Live {
                    scope: scope.clone(),
                    cursor: Some(*cursor),
                },
                action: ReplayAction::ApplyDelta,
            })
        }
        _ => Err("invalid replay transition: full bootstrap or valid live cursor required".into()),
    }
}

pub fn plan_replay_json(request_json: &str) -> Result<String, String> {
    let request: ReplayPlanRequest =
        serde_json::from_str(request_json).map_err(|e| e.to_string())?;
    let decision = plan_replay(&request)?;
    serde_json::to_string(&decision).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(id: &str, key: &str, position: u64, status: OutboxStatus) -> OutboxCandidate {
        OutboxCandidate {
            id: id.into(),
            account_key: "account-a".into(),
            ordering_key: Some(key.into()),
            queue_position: position,
            status,
            next_attempt_at_ms: None,
        }
    }

    fn request(actions: Vec<OutboxCandidate>) -> OutboxPlanRequest {
        OutboxPlanRequest {
            account_key: "account-a".into(),
            now_ms: 100,
            max_claims: 8,
            in_flight_keys: vec![],
            actions,
        }
    }

    fn scope() -> ReplayScope {
        ReplayScope {
            account_key: "a".into(),
            subscription_id: "messages".into(),
            query_fingerprint: "query-v1".into(),
            version_domain: None,
        }
    }

    fn version(raw: i64) -> VersionId {
        VersionId::try_from_i64(raw).expect("valid version")
    }

    #[test]
    fn scheduled_retry_blocks_newer_same_key_but_not_other_keys() {
        let mut first = action("a", "chat", 1, OutboxStatus::RetryScheduled);
        first.next_attempt_at_ms = Some(500);
        let plan = plan_outbox(&request(vec![
            first,
            action("b", "chat", 2, OutboxStatus::Queued),
            action("c", "other", 3, OutboxStatus::Queued),
        ]));
        assert_eq!(plan.action_ids, ["c"]);
        assert_eq!(plan.next_retry_at_ms, Some(500));
    }

    #[test]
    fn failed_head_blocks_dependents_until_resolved() {
        let plan = plan_outbox(&request(vec![
            action("a", "chat", 1, OutboxStatus::Failed),
            action("b", "chat", 2, OutboxStatus::Queued),
            action("c", "other", 3, OutboxStatus::Queued),
        ]));
        assert_eq!(plan.action_ids, ["c"]);
        assert_eq!(plan.failed_keys, ["chat"]);
    }

    #[test]
    fn in_flight_and_running_keys_never_claim_twice() {
        let mut req = request(vec![
            action("a", "chat", 1, OutboxStatus::Queued),
            action("b", "other", 2, OutboxStatus::Running),
            action("c", "other", 3, OutboxStatus::Queued),
        ]);
        req.in_flight_keys.push("chat".into());
        assert!(plan_outbox(&req).action_ids.is_empty());
    }

    #[test]
    fn respects_accounts_unordered_bucket_and_max_claims() {
        let mut foreign = action("foreign", "other", 0, OutboxStatus::Queued);
        foreign.account_key = "account-b".into();
        let mut req = request(vec![
            foreign,
            action("a", "", 1, OutboxStatus::Queued),
            action("b", "", 2, OutboxStatus::Queued),
            action("c", "separate", 3, OutboxStatus::Queued),
        ]);
        req.max_claims = 1;
        assert_eq!(plan_outbox(&req).action_ids, ["a"]);
    }

    #[test]
    fn json_abi_round_trips_large_integer_queue_positions() {
        let mut req = request(vec![action("a", "chat", u64::MAX - 1, OutboxStatus::Queued)]);
        req.max_claims = 1;
        let json = serde_json::to_string(&req).unwrap();
        let result: OutboxPlan = serde_json::from_str(&plan_outbox_json(&json).unwrap()).unwrap();
        assert_eq!(result.action_ids, ["a"]);
    }

    #[test]
    fn snapshot_pages_stage_until_final_swap_even_when_final_is_empty() {
        let start = plan_replay(&ReplayPlanRequest {
            state: ReplayState::Empty,
            frame: ReplayFrame::BeginSnapshot {
                scope: scope(),
                generation: "g-1".into(),
            },
        }).unwrap();
        assert_eq!(start.action, ReplayAction::BeginStaging);
        let page = plan_replay(&ReplayPlanRequest {
            state: start.state,
            frame: ReplayFrame::SnapshotPage {
                scope: scope(),
                generation: "g-1".into(),
                page_index: 0,
                is_last_page: false,
                cursor: Some(version(20)),
            },
        }).unwrap();
        assert_eq!(page.action, ReplayAction::AppendSnapshotPage);
        let end = plan_replay(&ReplayPlanRequest {
            state: page.state,
            frame: ReplayFrame::SnapshotPage {
                scope: scope(),
                generation: "g-1".into(),
                page_index: 1,
                is_last_page: true,
                cursor: None,
            },
        }).unwrap();
        assert_eq!(end.action, ReplayAction::FinishSnapshotAndSwap);
        assert_eq!(end.state, ReplayState::Live { scope: scope(), cursor: Some(version(20)) });
    }

    #[test]
    fn empty_snapshot_completes_without_a_row_version() {
        let start = plan_replay(&ReplayPlanRequest {
            state: ReplayState::Empty,
            frame: ReplayFrame::BeginSnapshot { scope: scope(), generation: "g".into() },
        }).unwrap();
        let done = plan_replay(&ReplayPlanRequest {
            state: start.state,
            frame: ReplayFrame::SnapshotPage {
                scope: scope(), generation: "g".into(), page_index: 0,
                is_last_page: true, cursor: None,
            },
        }).unwrap();
        assert_eq!(done.state, ReplayState::Live { scope: scope(), cursor: None });
    }

    #[test]
    fn delta_never_replaces_the_projection_and_deduplicates_replay() {
        let live = ReplayState::Live { scope: scope(), cursor: Some(version(20)) };
        let applied = plan_replay(&ReplayPlanRequest {
            state: live.clone(),
            frame: ReplayFrame::Delta { scope: scope(), cursor: version(21) },
        }).unwrap();
        assert_eq!(applied.action, ReplayAction::ApplyDelta);
        let repeat = plan_replay(&ReplayPlanRequest {
            state: applied.state,
            frame: ReplayFrame::Delta { scope: scope(), cursor: version(21) },
        }).unwrap();
        assert_eq!(repeat.action, ReplayAction::IgnoreDuplicate);
    }

    #[test]
    fn altered_query_and_invalid_page_order_fail_closed() {
        let live = ReplayState::Live { scope: scope(), cursor: Some(version(20)) };
        let mut changed_scope = scope();
        changed_scope.query_fingerprint = "different".into();
        assert!(plan_replay(&ReplayPlanRequest {
            state: live,
            frame: ReplayFrame::Delta { scope: changed_scope, cursor: version(21) },
        }).is_err());

        let loading = ReplayState::Loading {
            scope: scope(), generation: "g".into(), next_page: 1,
            last_page_cursor: Some(version(20)),
        };
        assert!(plan_replay(&ReplayPlanRequest {
            state: loading,
            frame: ReplayFrame::SnapshotPage {
                scope: scope(), generation: "g".into(), page_index: 3,
                is_last_page: false, cursor: Some(version(30)),
            },
        }).is_err());
    }

    #[test]
    fn replay_json_uses_decimal_string_versions_not_js_numbers() {
        let req = ReplayPlanRequest {
            state: ReplayState::Live {
                scope: scope(),
                cursor: Some(VersionId::from_decimal_str("9007199254740993").unwrap()),
            },
            frame: ReplayFrame::Delta {
                scope: scope(),
                cursor: VersionId::from_decimal_str("9007199254740994").unwrap(),
            },
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"9007199254740994\""));
        let out: ReplayPlan = serde_json::from_str(&plan_replay_json(&json).unwrap()).unwrap();
        assert_eq!(out.action, ReplayAction::ApplyDelta);
    }
}
