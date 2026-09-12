//! Single source of truth for the JS `ctx` host surface.
//!
//! Add a portable method here (JS path, TS signature, native or async op).
//! Emitters and V8 bind lists read this table. `CoreFunctionHost` is not edited
//! for portable methods.

/// How a host method is dispatched from V8.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostDispatch {
    AsyncOp(&'static str),
    Native(&'static str),
}

/// One portable `ctx` method. Nested HTTP/db objects are listed as their leaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostMethod {
    pub path:     &'static str,
    pub ts:       &'static str,
    pub dispatch: HostDispatch,
}

/// Async `kalamAsyncOp` kind names the V8 adapter must handle.
pub const ASYNC_OPS: &[&str] = &["query", "execute", "call", "publish", "sleep"];

/// Native functions installed on the isolate (order is documentation only).
pub const NATIVE_FNS: &[&str] = &[
    "kalamHostSql",
    "kalamHostCall",
    "kalamHostPublish",
    "kalamHostHttpHeader",
    "kalamHostHttpSetStatus",
    "kalamHostHttpSetHeader",
    "kalamHostIsHttpRoot",
    "kalamHostHasHttp",
    "kalamHostHttpMethod",
    "kalamHostHttpPath",
    "kalamHostHttpQuery",
    "kalamHostSource",
    "kalamHostParent",
    "kalamHostMetadata",
    "kalamHostLog",
    "kalamHostRoutineMap",
    "kalamAsyncOp",
];

/// Leaf methods on `ctx`. Adding a portable helper is one row plus a default
/// `FunctionHost` method — not a `CoreFunctionHost` impl.
pub const HOST_METHODS: &[HostMethod] = &[
    HostMethod {
        path:     "db.query",
        ts:       "query(sql: string, params?: unknown[]): Promise<unknown>",
        dispatch: HostDispatch::AsyncOp("query"),
    },
    HostMethod {
        path:     "db.execute",
        ts:       "execute(sql: string, params?: unknown[]): Promise<unknown>",
        dispatch: HostDispatch::AsyncOp("execute"),
    },
    HostMethod {
        path:     "functions.call",
        ts:       "call(name: string, args?: unknown): Promise<unknown>",
        dispatch: HostDispatch::AsyncOp("call"),
    },
    HostMethod {
        path:     "topics.publish",
        ts:       "publish(topic: string, payload: unknown): Promise<void>",
        dispatch: HostDispatch::AsyncOp("publish"),
    },
    HostMethod {
        path:     "sleep",
        ts:       "sleep(ms: number): Promise<void>",
        dispatch: HostDispatch::AsyncOp("sleep"),
    },
    HostMethod {
        path:     "log.debug",
        ts:       "debug(message: string, ...args: unknown[]): void",
        dispatch: HostDispatch::Native("kalamHostLog"),
    },
    HostMethod {
        path:     "log.info",
        ts:       "info(message: string, ...args: unknown[]): void",
        dispatch: HostDispatch::Native("kalamHostLog"),
    },
    HostMethod {
        path:     "log.warn",
        ts:       "warn(message: string, ...args: unknown[]): void",
        dispatch: HostDispatch::Native("kalamHostLog"),
    },
    HostMethod {
        path:     "log.error",
        ts:       "error(message: string, ...args: unknown[]): void",
        dispatch: HostDispatch::Native("kalamHostLog"),
    },
    HostMethod {
        path:     "http.request.headers.get",
        ts:       "get(name: string): string | null",
        dispatch: HostDispatch::Native("kalamHostHttpHeader"),
    },
    HostMethod {
        path:     "http.request.query.get",
        ts:       "get(name: string): string | null",
        dispatch: HostDispatch::Native("kalamHostHttpQuery"),
    },
    HostMethod {
        path:     "http.response.status",
        ts:       "status(code: number): void",
        dispatch: HostDispatch::Native("kalamHostHttpSetStatus"),
    },
    HostMethod {
        path:     "http.response.header",
        ts:       "header(name: string, value: string): void",
        dispatch: HostDispatch::Native("kalamHostHttpSetHeader"),
    },
    HostMethod {
        path:     "http.response.contentType",
        ts:       "contentType(value: string): void",
        dispatch: HostDispatch::Native("kalamHostHttpSetHeader"),
    },
];

pub fn is_async_op(kind: &str) -> bool {
    ASYNC_OPS.contains(&kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn async_ops_are_listed_on_methods() {
        for kind in ASYNC_OPS {
            assert!(
                HOST_METHODS.iter().any(|method| method.dispatch == HostDispatch::AsyncOp(kind)),
                "missing HOST_METHODS row for async op {kind}"
            );
        }
    }
}
