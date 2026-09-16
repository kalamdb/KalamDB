//! Persist isolate and deployment lifecycle into `system.procedure_logs`.
//!
//! The engine already rate-limits the per-call kinds (`reused`/`idle`), so this observer only runs
//! on cold start, drop, deploy, and at most once a minute per warm isolate.

use std::sync::Arc;

use kalamdb_functions::{now_ms, FunctionLifecycleEvent, FunctionLifecycleObserver};

use crate::procedure_log_logger::{ProcedureLogLogger, ProcedureLogRecord};

pub struct FunctionRuntimeLogObserver {
    pub logger:  Arc<ProcedureLogLogger>,
    pub node_id: Arc<str>,
}

impl FunctionLifecycleObserver for FunctionRuntimeLogObserver {
    fn on_lifecycle(&self, event: FunctionLifecycleEvent) {
        let FunctionLifecycleEvent {
            kind,
            reason,
            instance_id,
            worker,
            module_id,
            revision_id,
            procedure_id,
            reserved_bytes,
            used_heap_bytes,
            invocations,
        } = event;
        let kind_str = kind.as_str();
        let procedure_id =
            procedure_id.filter(|id| !id.is_empty()).unwrap_or_else(|| module_id.clone());
        let message = format!(
            "isolate {instance_id} {kind_str} ({reason}) worker={worker} calls={invocations} \
             reserved={reserved_bytes} used={used_heap_bytes}"
        );
        self.logger.record(ProcedureLogRecord {
            execution_id: format!("lifecycle:{kind_str}:{instance_id}"),
            request_id: format!("isolate-{instance_id}"),
            procedure_id,
            module_id: Some(module_id),
            revision_id: Some(revision_id),
            actor: "system".into(),
            origin: "runtime".into(),
            outcome: kind_str.into(),
            channel: "lifecycle".into(),
            level: kind.log_level().into(),
            error_code: None,
            message: Some(message),
            duration_ms: 0,
            timestamp: now_ms(),
            node_id: self.node_id.to_string(),
        });
    }
}

pub fn record_deployment(
    logger: &ProcedureLogLogger,
    node_id: impl Into<String>,
    procedure_id: &str,
    module_id: Option<String>,
    revision_id: Option<String>,
    generation: u64,
    procedure_count: usize,
) {
    logger.record(ProcedureLogRecord {
        execution_id: format!("lifecycle:deploy:{generation}"),
        request_id: format!("generation-{generation}"),
        procedure_id: procedure_id.to_string(),
        module_id,
        revision_id,
        actor: "system".into(),
        origin: "runtime".into(),
        outcome: "deployed".into(),
        channel: "lifecycle".into(),
        level: "info".into(),
        error_code: None,
        message: Some(format!(
            "active function set generation {generation} published ({procedure_count} procedures)"
        )),
        duration_ms: 0,
        timestamp: now_ms(),
        node_id: node_id.into(),
    });
}
