//! Wrap CREATE PROCEDURE bodies into the ABI v2 `kalamInvoke` surface.

use std::sync::OnceLock;

/// If the body already defines `kalamInvoke`, keep it. Otherwise wrap it as
/// `(ctx, input) => { body }` so dollar-quoted JS can use host objects.
pub fn wrap_procedure_source(body: &str) -> String {
    if body.contains("function kalamInvoke") {
        return body.to_string();
    }
    format!(
        "function kalamInvoke(name, args) {{\nconst ctx = globalThis.__kalamCtx;\nconst input = \
         args.length === 1 ? args[0] : Array.from(args);\nreturn (function (ctx, input) \
         {{\n{body}\n}})(ctx, input);\n}}\n"
    )
}

pub fn host_bootstrap() -> &'static str {
    static BOOTSTRAP: OnceLock<String> = OnceLock::new();
    BOOTSTRAP.get_or_init(kalamdb_functions_host::emit_js_bootstrap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_leaves_kalam_invoke_alone() {
        let source = "function kalamInvoke(name, args) { return args[0]; }";
        assert_eq!(wrap_procedure_source(source), source);
    }

    #[test]
    fn wrap_dollar_quoted_body() {
        let wrapped = wrap_procedure_source("return ctx.db.query('SELECT 1');");
        assert!(wrapped.contains("function kalamInvoke"));
        assert!(wrapped.contains("ctx.db.query"));
    }

    #[test]
    fn bootstrap_is_abi_v2_only() {
        let bootstrap = host_bootstrap();
        assert!(bootstrap.contains("kalamAsyncOp(\"query\""));
        assert!(!bootstrap.contains("db.sql"));
        assert!(bootstrap.contains("httpEnabled"));
        assert!(bootstrap.contains("kalamHostRoutineMap"));
        assert!(bootstrap.contains("__kalamSerializeLogArg"));
        assert!(bootstrap.contains("Object.defineProperty(globalThis, \"console\""));
    }
}
