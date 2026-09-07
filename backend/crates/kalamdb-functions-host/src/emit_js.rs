//! V8 bootstrap that builds the frozen `ctx` object (ABI v2).

/// ABI v2 operations always return Promises. Metadata comes only from the host.
pub fn emit_js_bootstrap() -> String {
    r#"
function __kalamMakeCtx() {
  const metadata = JSON.parse(kalamHostMetadata());
  const httpEnabled = typeof kalamHostHasHttp === "function" && kalamHostHasHttp();
  const http = httpEnabled ? Object.freeze({
    request: Object.freeze({
      method: kalamHostHttpMethod(),
      path: kalamHostHttpPath(),
      headers: Object.freeze({
        get: name => kalamHostHttpHeader(String(name)),
      }),
      query: Object.freeze({
        get: name => kalamHostHttpQuery(String(name)),
      }),
    }),
    response: Object.freeze({
      status: code => kalamHostHttpSetStatus(code),
      header: (name, value) => kalamHostHttpSetHeader(String(name), String(value)),
      contentType: value => kalamHostHttpSetHeader("content-type", String(value)),
    }),
  }) : null;
  const serializeLogArg = (arg) => {
    if (arg instanceof Error) {
      return { name: arg.name, message: arg.message, stack: arg.stack };
    }
    return arg;
  };
  const call = (name, args = []) => kalamAsyncOp("call", name, Array.isArray(args) ? args : [args]);
  let typed = {};
  try {
    typed = JSON.parse(typeof kalamHostRoutineMap === "function" ? kalamHostRoutineMap() : "{}");
  } catch (e) {
    typed = {};
  }
  const namespaces = {};
  for (const schema of Object.keys(typed)) {
    const methods = typed[schema] || {};
    const boxed = {};
    for (const jsName of Object.keys(methods)) {
      const routineId = methods[jsName];
      boxed[jsName] = (input) => call(routineId, input === undefined ? [] : [input]);
    }
    namespaces[schema] = Object.freeze(boxed);
  }
  return Object.freeze({
    ...metadata,
    source: Object.freeze(kalamHostSource()),
    parent: kalamHostParent(),
    db: Object.freeze({
      query: (sql, params = []) => kalamAsyncOp("query", sql, params),
      execute: (sql, params = []) => kalamAsyncOp("execute", sql, params),
    }),
    functions: Object.freeze(Object.assign({ call: call }, namespaces)),
    topics: Object.freeze({publish: (topic, payload) => kalamAsyncOp("publish", topic, [payload])}),
    log: Object.freeze({
      debug: (...args) => kalamHostLog("debug", JSON.stringify(Array.from(args).map(serializeLogArg))),
      info: (...args) => kalamHostLog("info", JSON.stringify(Array.from(args).map(serializeLogArg))),
      warn: (...args) => kalamHostLog("warn", JSON.stringify(Array.from(args).map(serializeLogArg))),
      error: (...args) => kalamHostLog("error", JSON.stringify(Array.from(args).map(serializeLogArg))),
    }),
    http,
  });
}
try { delete globalThis.eval; } catch (e) {}
try { delete globalThis.Function; } catch (e) {}
"#
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{ASYNC_OPS, NATIVE_FNS};

    #[test]
    fn bootstrap_mentions_every_async_op() {
        let js = emit_js_bootstrap();
        for kind in ASYNC_OPS {
            assert!(js.contains(&format!("kalamAsyncOp(\"{kind}\"")), "{kind}");
        }
        assert!(js.contains("kalamHostRoutineMap"));
        assert!(js.contains("serializeLogArg"));
        assert!(!js.contains("db.sql"));
    }

    #[test]
    fn bootstrap_mentions_log_and_routine_map_natives() {
        let js = emit_js_bootstrap();
        assert!(NATIVE_FNS.contains(&"kalamHostLog"));
        assert!(NATIVE_FNS.contains(&"kalamHostRoutineMap"));
        assert!(js.contains("kalamHostLog"));
    }
}
