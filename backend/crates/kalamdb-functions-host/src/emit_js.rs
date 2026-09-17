//! V8 bootstrap that builds the frozen `ctx` object (ABI v2).

/// ABI v2 operations always return Promises. Metadata comes only from the host.
/// `console` and `ctx.log` share `kalamHostLog`; the third argument is the
/// log channel (`console` or `ctx.log`).
pub fn emit_js_bootstrap() -> String {
    r#"
function __kalamSerializeLogArg(arg) {
  if (arg instanceof Error) {
    return { name: arg.name, message: arg.message, stack: arg.stack };
  }
  return arg;
}
function __kalamEmitLog(channel, level, args) {
  kalamHostLog(level, JSON.stringify(Array.from(args).map(__kalamSerializeLogArg)), channel);
}
(function () {
  const consoleObject = Object.freeze({
    debug: (...args) => __kalamEmitLog("console", "debug", args),
    log: (...args) => __kalamEmitLog("console", "info", args),
    info: (...args) => __kalamEmitLog("console", "info", args),
    warn: (...args) => __kalamEmitLog("console", "warn", args),
    error: (...args) => __kalamEmitLog("console", "error", args),
  });
  try {
    Object.defineProperty(globalThis, "console", {
      value: consoleObject,
      writable: false,
      configurable: false,
      enumerable: true,
    });
  } catch (e) {
    globalThis.console = consoleObject;
  }
})();
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
  const call = (name, args = []) => kalamAsyncOp("call", name, Array.isArray(args) ? args : [args]);
  let typed = {};
  try {
    typed = JSON.parse(typeof kalamHostRoutineMap === "function" ? kalamHostRoutineMap() : "{}");
  } catch (e) {
    typed = {};
  }
  const namespaces = {};
  for (const namespace of Object.keys(typed)) {
    const methods = typed[namespace] || {};
    const boxed = {};
    for (const jsName of Object.keys(methods)) {
      const routineId = methods[jsName];
      boxed[jsName] = (input) => call(routineId, input === undefined ? [] : [input]);
    }
    namespaces[namespace] = Object.freeze(boxed);
  }
  return Object.freeze({
    ...metadata,
    source: Object.freeze(kalamHostSource()),
    parent: kalamHostParent(),
    sleep: (ms) => kalamAsyncOp("sleep", "", [ms]),
    db: Object.freeze({
      query: (sql, params = []) => kalamAsyncOp("query", sql, params),
      execute: (sql, params = []) => kalamAsyncOp("execute", sql, params),
    }),
    functions: Object.freeze(Object.assign({ call: call }, namespaces)),
    topics: Object.freeze({publish: (topic, payload) => kalamAsyncOp("publish", topic, [payload])}),
    log: Object.freeze({
      debug: (...args) => __kalamEmitLog("ctx.log", "debug", args),
      info: (...args) => __kalamEmitLog("ctx.log", "info", args),
      warn: (...args) => __kalamEmitLog("ctx.log", "warn", args),
      error: (...args) => __kalamEmitLog("ctx.log", "error", args),
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
        assert!(js.contains("__kalamSerializeLogArg"));
        assert!(js.contains("__kalamEmitLog"));
        assert!(!js.contains("db.sql"));
    }

    #[test]
    fn bootstrap_mentions_log_and_routine_map_natives() {
        let js = emit_js_bootstrap();
        assert!(NATIVE_FNS.contains(&"kalamHostLog"));
        assert!(NATIVE_FNS.contains(&"kalamHostRoutineMap"));
        assert!(js.contains("kalamHostLog"));
        assert!(js.contains("Object.defineProperty(globalThis, \"console\""));
        assert!(js.contains("__kalamEmitLog(\"console\""));
        assert!(js.contains("__kalamEmitLog(\"ctx.log\""));
    }
}
