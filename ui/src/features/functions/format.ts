import type { ProcedureMetadata, ProcedureParameter, ResolvedKalamType } from "./types";

const BUILTIN_SQL_TYPES = new Set([
  "BOOLEAN",
  "BOOL",
  "SMALLINT",
  "INT",
  "INTEGER",
  "BIGINT",
  "FLOAT",
  "REAL",
  "DOUBLE",
  "DECIMAL",
  "NUMERIC",
  "TEXT",
  "VARCHAR",
  "CHAR",
  "STRING",
  "UUID",
  "JSON",
  "JSONB",
  "TIMESTAMP",
  "TIMESTAMPTZ",
  "DATETIME",
  "DATE",
  "TIME",
  "BYTES",
  "BYTEA",
  "FILE",
  "VOID",
]);

export function revisionExports(value: string): string[] {
  if (!value.trim()) {
    return [];
  }
  return value
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
}

export function isBuiltinSqlTypeName(sqlName: string): boolean {
  const normalized = sqlName.trim().toUpperCase();
  if (BUILTIN_SQL_TYPES.has(normalized)) {
    return true;
  }
  const base = normalized.split("(")[0]?.trim() ?? normalized;
  return BUILTIN_SQL_TYPES.has(base) || base.startsWith("DECIMAL") || base.startsWith("EMBEDDING");
}

export function formatNullability(type: ResolvedKalamType): string {
  return type.notNull ? "NOT NULL" : "";
}

export function formatSqlType(type: ResolvedKalamType): string {
  switch (type.kind) {
    case "array":
      return `${formatSqlType(type.element)}[]`;
    case "builtin":
    case "enum":
    case "composite":
    case "unknown":
      return type.sqlName;
  }
}

export function formatDeclaredType(type: ResolvedKalamType): string {
  const sqlName = formatSqlType(type);
  const nullability = formatNullability(type);
  if (type.kind === "array" && type.nonempty) {
    return nullability ? `${sqlName} NONEMPTY ${nullability}` : `${sqlName} NONEMPTY`;
  }
  return nullability ? `${sqlName} ${nullability}` : sqlName;
}

export function formatParameterDeclaration(parameter: ProcedureParameter): string {
  return `${parameter.name} ${formatDeclaredType(parameter.type)}`;
}

export function formatSignatureDetail(procedure: ProcedureMetadata): string {
  const args = procedure.parameters.map(formatParameterDeclaration).join(", ");
  return `(${args}) → ${formatSqlType(procedure.returnType)}`;
}

export function formatCallSnippet(procedure: ProcedureMetadata): string {
  if (procedure.parameters.length === 0) {
    return `CALL ${procedure.id}();`;
  }

  const args = procedure.parameters
    .map((parameter, index) => `\${${index + 1}:${parameter.name}}`)
    .join(",\n    ");
  return `CALL ${procedure.id}(\n    ${args}\n);`;
}

export function shortenRevisionId(revisionId: string, head = 16): string {
  if (revisionId.length <= head + 1) {
    return revisionId;
  }
  return `${revisionId.slice(0, head)}…`;
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) {
    return "—";
  }
  if (bytes < 1024) {
    return `${bytes} B`;
  }
  const kib = bytes / 1024;
  if (kib < 1024) {
    return `${kib < 10 ? kib.toFixed(1) : Math.round(kib)} KB`;
  }
  const mib = kib / 1024;
  return `${mib < 10 ? mib.toFixed(1) : Math.round(mib)} MB`;
}

export function formatDurationMs(durationMs: number | null | undefined): string {
  if (durationMs === null || durationMs === undefined || !Number.isFinite(durationMs)) {
    return "—";
  }
  if (durationMs < 1000) {
    return `${Math.round(durationMs)} ms`;
  }
  return `${(durationMs / 1000).toFixed(2)} s`;
}

export function formatCompositeOutline(type: ResolvedKalamType): string[] {
  if (type.kind === "array") {
    return formatCompositeOutline(type.element);
  }
  if (type.kind !== "composite") {
    return [];
  }

  const lines = [`${type.sqlName}`];
  type.fields.forEach((field, index) => {
    const prefix = index === type.fields.length - 1 ? "└──" : "├──";
    lines.push(`${prefix} ${field.name} ${formatDeclaredType(field.type)}`);
    if (field.type.kind === "composite" || (field.type.kind === "array" && field.type.element.kind === "composite")) {
      for (const nested of formatCompositeOutline(field.type).slice(1)) {
        lines.push(`    ${nested}`);
      }
    }
  });
  return lines;
}

export function normalizeLogLevel(level: string): string {
  return level.trim().toLowerCase();
}

export function displayLogLevel(level: string): string {
  const normalized = normalizeLogLevel(level);
  if (normalized === "warn" || normalized === "warning") {
    return "WARN";
  }
  if (normalized === "error") {
    return "ERROR";
  }
  if (normalized === "debug") {
    return "DEBUG";
  }
  return "INFO";
}

export function displaySecurityPolicy(security: string): string {
  const normalized = security.trim().toUpperCase().replace(/\s+/g, " ");
  if (normalized === "DEFINER" || normalized === "SECURITY DEFINER") {
    return "SECURITY DEFINER";
  }
  if (normalized === "INVOKER" || normalized === "SECURITY INVOKER" || !normalized) {
    return "SECURITY INVOKER";
  }
  return security.trim();
}

export function displayImplementation(implementation: string | null | undefined): string {
  switch (implementation?.trim().toLowerCase()) {
    case "inline":
      return "Inline script";
    case "module":
      return "Project module";
    case "missing":
      return "Unimplemented";
    default:
      return implementation?.trim() || "—";
  }
}

export function displayLanguage(language: string | null | undefined): string {
  const trimmed = language?.trim();
  return trimmed ? trimmed.toUpperCase() : "—";
}

export function displayLogOrigin(origin: string | null | undefined): string {
  const normalized = origin?.trim().toLowerCase() ?? "";
  if (normalized === "sql") {
    return "SQL";
  }
  if (normalized === "http") {
    return "HTTP";
  }
  if (normalized === "topic") {
    return "Topic";
  }
  return origin?.trim() || "—";
}

export function displayLogActor(actor: string | null | undefined): string {
  const trimmed = actor?.trim() ?? "";
  return trimmed || "—";
}
