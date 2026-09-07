//! TypeScript `.d.ts` for inline shims and project `defineProcedure` handlers.

use std::collections::BTreeMap;

use crate::idents::{method_ident, schema_object_ident};

pub struct TypedRoutine<'a> {
    pub schema:      &'a str,
    pub name:        &'a str,
    pub type_ident:  &'a str,
    pub param_count: usize,
}

pub fn emit_typescript() -> String {
    r#"export interface Actor {
  readonly id: string;
  readonly role: string;
}

export interface ProcedureContext {
  readonly actor: Actor;
  readonly principal: Actor;
  readonly source: { readonly kind: string };
  readonly parent: string | null;
  readonly db: DbHost;
  readonly functions: FunctionsHost;
  readonly topics: TopicsHost;
  readonly log: LogHost;
  readonly http: HttpHost | null;
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

export function defineProcedure<C extends { input: unknown; output: unknown }>(
  handler: (ctx: ProcedureContext, input: C["input"]) => Promise<C["output"]> | C["output"],
): (ctx: ProcedureContext, input: C["input"]) => Promise<C["output"]> | C["output"];
"#
    .to_string()
}

pub fn emit_typed_functions_host(routines: &[TypedRoutine<'_>]) -> String {
    if routines.is_empty() {
        return String::new();
    }
    let mut by_schema: BTreeMap<&str, Vec<&TypedRoutine<'_>>> = BTreeMap::new();
    for routine in routines {
        by_schema.entry(routine.schema).or_default().push(routine);
    }
    let mut out = String::from("\n");
    let mut imports: Vec<String> =
        routines.iter().map(|routine| routine.type_ident.to_string()).collect();
    imports.sort();
    imports.dedup();
    if !imports.is_empty() {
        out.push_str("import type { ");
        out.push_str(&imports.join(", "));
        out.push_str(" } from \"./contracts\";\n\n");
    }
    out.push_str("export interface FunctionsHost {\n");
    for (schema, methods) in by_schema {
        let ident = schema_object_ident(schema);
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
                out.push_str("[\"input\"]): Promise<");
            }
            out.push_str(routine.type_ident);
            out.push_str("[\"output\"]>;\n");
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
        assert!(dts.contains("error(error: unknown, message?: string, ...args: unknown[]): void"));
        assert!(dts.contains("defineProcedure"));
    }

    #[test]
    fn typed_host_uses_schema_and_camel_method() {
        let dts = emit_typed_functions_host(&[TypedRoutine {
            schema:      "chat",
            name:        "create_message",
            type_ident:  "ChatCreateMessage",
            param_count: 2,
        }]);
        assert!(dts.contains("import type { ChatCreateMessage } from \"./contracts\""));
        assert!(dts.contains("createMessage(input: ChatCreateMessage[\"input\"])"));
        assert!(dts.contains("chat:"));
    }

    #[test]
    fn empty_arg_emits_no_input_param() {
        let dts = emit_typed_functions_host(&[TypedRoutine {
            schema:      "api",
            name:        "health",
            type_ident:  "ApiHealth",
            param_count: 0,
        }]);
        assert!(dts.contains("health(): Promise<ApiHealth[\"output\"]>"));
    }
}
