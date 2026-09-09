//! TypeScript `.d.ts` for inline shims and project `defineProcedure` handlers.

use std::collections::BTreeMap;

use crate::idents::{method_ident, namespace_object_ident};

pub struct TypedRoutine<'a> {
    pub namespace:   &'a str,
    pub name:        &'a str,
    pub type_ident:  &'a str,
    pub param_count: usize,
}

pub fn emit_typescript() -> String {
    r#"import type { ProcedureOrm } from "@kalamdb/orm";

export interface Actor {
  readonly id: string;
  readonly role: string;
}

export interface ProcedureContext {
  readonly actor: Actor;
  readonly principal: Actor;
  readonly source: { readonly kind: string };
  readonly parent: string | null;
  readonly db: DbHost;
  readonly orm: ProcedureOrm;
  readonly functions: FunctionsHost;
  readonly topics: TopicsHost;
  readonly log: LogHost;
  readonly http: HttpHost | null;
  sleep(ms: number): Promise<void>;
}

export interface DbHost {
  query(sql: string, params?: unknown[]): Promise<unknown>;
  execute(sql: string, params?: unknown[]): Promise<unknown>;
}

export interface FunctionsHost {
  call(name: string, args?: unknown): Promise<unknown>;
}

export interface TopicsHost {
  publish(topic: string, payload: unknown): Promise<void>;
}

export interface LogHost {
  debug(message: string, ...args: unknown[]): void;
  info(message: string, ...args: unknown[]): void;
  warn(message: string, ...args: unknown[]): void;
  error(message: string, ...args: unknown[]): void;
  error(error: unknown, message?: string, ...args: unknown[]): void;
}

export interface HttpHost {
  readonly request: {
    readonly method: string;
    readonly path: string;
    headers: { get(name: string): string | null };
    query: { get(name: string): string | null };
  };
  readonly response: {
    status(code: number): void;
    header(name: string, value: string): void;
    contentType(value: string): void;
  };
}

export function defineProcedure<TRequest = unknown, TResult = void>(
  handler: (ctx: ProcedureContext, input: TRequest) => Promise<TResult> | TResult,
): (ctx: ProcedureContext, input: TRequest) => Promise<TResult> | TResult;
// Isolate `console.debug|log|info|warn|error` forwards to the process logger
// with channel=console. Prefer ctx.log for structured procedure logs.
"#
    .to_string()
}

pub fn emit_typed_functions_host(routines: &[TypedRoutine<'_>]) -> String {
    if routines.is_empty() {
        return String::new();
    }
    let mut by_namespace: BTreeMap<&str, Vec<&TypedRoutine<'_>>> = BTreeMap::new();
    for routine in routines {
        by_namespace.entry(routine.namespace).or_default().push(routine);
    }
    let mut out = String::from("\n");
    let mut imports: Vec<String> = Vec::new();
    for routine in routines {
        if routine.param_count > 0 {
            imports.push(format!("{}Request", routine.type_ident));
        }
        imports.push(format!("{}Result", routine.type_ident));
    }
    imports.sort();
    imports.dedup();
    if !imports.is_empty() {
        out.push_str("import type { ");
        out.push_str(&imports.join(", "));
        out.push_str(" } from \"./contracts\";\n\n");
    }
    out.push_str("export interface FunctionsHost {\n");
    for (namespace, methods) in by_namespace {
        let ident = namespace_object_ident(namespace);
        out.push_str("  ");
        out.push_str(&ident);
        out.push_str(": {\n");
        for routine in methods {
            let method = method_ident(routine.name);
            out.push_str("    ");
            out.push_str(&method);
            if routine.param_count == 0 {
                out.push_str("(): Promise<");
            } else {
                out.push_str("(input: ");
                out.push_str(routine.type_ident);
                out.push_str("Request): Promise<");
            }
            out.push_str(routine.type_ident);
            out.push_str("Result>;\n");
        }
        out.push_str("  };\n");
    }
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typescript_lists_log_overloads_and_define_procedure() {
        let dts = emit_typescript();
        assert!(dts.contains("export interface ProcedureContext"));
        assert!(dts.contains("import type { ProcedureOrm } from \"@kalamdb/orm\""));
        assert!(dts.contains("readonly orm: ProcedureOrm"));
        assert!(dts.contains("sleep(ms: number): Promise<void>"));
        assert!(dts.contains("error(error: unknown, message?: string, ...args: unknown[]): void"));
        assert!(dts.contains("defineProcedure"));
        assert!(dts.contains("ctx: ProcedureContext"));
        assert!(dts.contains("input: TRequest"));
        assert!(dts.contains("TRequest = unknown, TResult = void"));
        assert!(dts.contains("channel=console"));
    }

    #[test]
    fn typed_host_uses_namespace_and_camel_method() {
        let dts = emit_typed_functions_host(&[TypedRoutine {
            namespace:   "chat",
            name:        "create_message",
            type_ident:  "ChatCreateMessage",
            param_count: 2,
        }]);
        assert!(dts.contains(
            "import type { ChatCreateMessageRequest, ChatCreateMessageResult } from \
             \"./contracts\""
        ));
        assert!(dts.contains("createMessage(input: ChatCreateMessageRequest)"));
        assert!(dts.contains("Promise<ChatCreateMessageResult>"));
        assert!(dts.contains("chat:"));
    }

    #[test]
    fn empty_arg_emits_no_input_param() {
        let dts = emit_typed_functions_host(&[TypedRoutine {
            namespace:   "api",
            name:        "health",
            type_ident:  "ApiHealth",
            param_count: 0,
        }]);
        assert!(dts.contains("health(): Promise<ApiHealthResult>"));
    }
}
