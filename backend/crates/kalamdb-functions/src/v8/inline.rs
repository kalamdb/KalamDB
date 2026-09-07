//! CREATE PROCEDURE checks for inline JavaScript: host-API lint + V8 parse.

use crate::{
    error::{FunctionsError, Result},
    v8_adapter::compile_javascript_source,
    wrap_procedure_source,
};

/// Lint host usage, wrap the dollar-quoted body, and parse it in V8.
///
/// Does not invoke the procedure. `ctx.log('x')` is valid JavaScript, so the
/// lint exists to catch host objects that are not callable.
pub fn prepare_inline_javascript(body: &str) -> Result<String> {
    lint_inline_javascript(body)?;
    let source = wrap_procedure_source(body);
    compile_javascript_source(&source)?;
    Ok(source)
}

pub fn lint_inline_javascript(body: &str) -> Result<()> {
    if has_bare_call(body, "ctx.log") {
        return Err(FunctionsError::Invalid(
            "ctx.log is not a function; use ctx.log.info(...), ctx.log.debug(...), \
             ctx.log.warn(...), or ctx.log.error(...)"
                .into(),
        ));
    }
    if has_bare_call(body, "ctx.db") {
        return Err(FunctionsError::Invalid(
            "ctx.db is not a function; use ctx.db.query(...) or ctx.db.execute(...)".into(),
        ));
    }
    if has_bare_call(body, "ctx.functions") {
        return Err(FunctionsError::Invalid(
            "ctx.functions is not a function; use ctx.functions.call(...) or \
             ctx.functions.<schema>.<method>(...)"
                .into(),
        ));
    }
    if has_bare_call(body, "ctx.topics") {
        return Err(FunctionsError::Invalid(
            "ctx.topics is not a function; use ctx.topics.publish(...)".into(),
        ));
    }
    if has_console_access(body) {
        return Err(FunctionsError::Invalid(
            "console is not available in the functions isolate; use ctx.log.info(...)".into(),
        ));
    }
    Ok(())
}

fn has_bare_call(body: &str, callee: &str) -> bool {
    let bytes = body.as_bytes();
    let mut index = 0;
    while index + callee.len() <= body.len() {
        if body[index..].starts_with(callee) {
            let ident_before = index > 0 && is_ident_continue(bytes[index - 1]);
            if !ident_before {
                let after = body[index + callee.len()..].trim_start();
                if after.starts_with('(') {
                    return true;
                }
            }
            index += 1;
        } else {
            index += 1;
        }
    }
    false
}

fn has_console_access(body: &str) -> bool {
    let bytes = body.as_bytes();
    let mut index = 0;
    while let Some(relative) = body[index..].find("console") {
        let at = index + relative;
        let ident_before = at > 0 && is_ident_continue(bytes[at - 1]);
        let after_index = at + "console".len();
        let ident_after = after_index < bytes.len() && is_ident_continue(bytes[after_index]);
        if !ident_before && !ident_after {
            let after = body[after_index..].trim_start();
            if after.starts_with('.') || after.starts_with('[') || after.starts_with('(') {
                return true;
            }
        }
        index = at + 1;
    }
    false
}

fn is_ident_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lint_rejects_ctx_log_as_function() {
        let error = lint_inline_javascript("ctx.log('ttt'); return 'ok';").unwrap_err().to_string();
        assert!(error.contains("ctx.log is not a function"), "{error}");
        assert!(error.contains("ctx.log.info"), "{error}");
    }

    #[test]
    fn lint_allows_ctx_log_info() {
        lint_inline_javascript("ctx.log.info('ttt'); return 'ok';").unwrap();
    }

    #[test]
    fn lint_rejects_console() {
        let error = lint_inline_javascript("console.log('this is a test'); return 'ok';")
            .unwrap_err()
            .to_string();
        assert!(error.contains("console is not available"), "{error}");
    }

    #[test]
    fn lint_rejects_ctx_db_as_function() {
        let error = lint_inline_javascript("return ctx.db('SELECT 1');").unwrap_err().to_string();
        assert!(error.contains("ctx.db is not a function"), "{error}");
    }

    #[test]
    fn lint_allows_nested_log_name() {
        lint_inline_javascript("const my_ctx_log = 1; return my_ctx_log;").unwrap();
    }

    #[test]
    #[ntest::timeout(15000)]
    fn prepare_parses_valid_javascript() {
        prepare_inline_javascript("ctx.log.info('ok'); return 'ok';").unwrap();
    }

    #[test]
    #[ntest::timeout(15000)]
    fn prepare_rejects_syntax_error() {
        let error = prepare_inline_javascript("return 'ok").unwrap_err().to_string();
        assert!(
            error.to_ascii_lowercase().contains("javascript") || error.contains("Unexpected"),
            "{error}"
        );
    }
}
