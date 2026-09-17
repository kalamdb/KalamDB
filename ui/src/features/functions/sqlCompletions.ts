import type { SqlCompletionEntry } from "@/components/sql-studio-v2/input-form/sqlCompletionCatalog";
import { formatCompositeOutline, formatDeclaredType, formatParameterDeclaration, formatSignatureDetail } from "./format";
import type { ProcedureMetadata, ProcedureParameter, ResolvedKalamType } from "./types";

export interface CallNameContext {
  kind: "name";
  partial: string;
  replaceFrom: number;
}

export interface CallArgumentContext {
  kind: "arguments";
  procedureId: string;
  activeParameter: number;
}

export type CallCompletionContext = CallNameContext | CallArgumentContext;

export function parseCallCompletionContext(textUntilPosition: string): CallCompletionContext | null {
  const callKeyword = lastCallKeyword(textUntilPosition);
  if (!callKeyword) {
    return null;
  }

  const afterCall = textUntilPosition.slice(callKeyword.end);
  const nameMatch = /^\s*([a-zA-Z_][\w]*(?:\.[a-zA-Z_][\w]*)*)?/.exec(afterCall);
  const name = nameMatch?.[1] ?? "";
  const leadingWhitespace = /^\s*/.exec(afterCall)?.[0].length ?? 0;
  const afterName = afterCall.slice(nameMatch?.[0].length ?? 0);
  const openParen = afterName.trimStart().startsWith("(");

  if (!openParen) {
    return {
      kind: "name",
      partial: name.toLowerCase(),
      replaceFrom: callKeyword.end + leadingWhitespace,
    };
  }

  const argsStart = afterCall.indexOf("(");
  if (argsStart < 0) {
    return {
      kind: "name",
      partial: name.toLowerCase(),
      replaceFrom: callKeyword.end + leadingWhitespace,
    };
  }

  const argumentSource = afterCall.slice(argsStart + 1);
  return {
    kind: "arguments",
    procedureId: name,
    activeParameter: countActiveParameter(argumentSource),
  };
}

function lastCallKeyword(text: string): { start: number; end: number } | null {
  const matcher = /\bCALL\b/gi;
  let last: { start: number; end: number } | null = null;
  let match = matcher.exec(text);
  while (match) {
    if (!isInsideString(text, match.index)) {
      last = { start: match.index, end: match.index + match[0].length };
    }
    match = matcher.exec(text);
  }
  return last;
}

function isInsideString(text: string, index: number): boolean {
  let inSingle = false;
  let inDouble = false;
  for (let i = 0; i < index; i += 1) {
    const char = text[i];
    if (char === "'" && !inDouble) {
      inSingle = !inSingle;
    } else if (char === '"' && !inSingle) {
      inDouble = !inDouble;
    }
  }
  return inSingle || inDouble;
}

function countActiveParameter(source: string): number {
  let depth = 1;
  let parameterIndex = 0;
  let inSingle = false;
  let inDouble = false;

  for (let i = 0; i < source.length; i += 1) {
    const char = source[i];
    if (char === "'" && !inDouble) {
      inSingle = !inSingle;
      continue;
    }
    if (char === '"' && !inSingle) {
      inDouble = !inDouble;
      continue;
    }
    if (inSingle || inDouble) {
      continue;
    }
    if (char === "(" || char === "[" || char === "{") {
      depth += 1;
      continue;
    }
    if (char === ")" || char === "]" || char === "}") {
      depth -= 1;
      if (depth === 0) {
        return parameterIndex;
      }
      continue;
    }
    if (char === "," && depth === 1) {
      parameterIndex += 1;
    }
  }

  return parameterIndex;
}

export function procedureCompletionEntries(procedures: ProcedureMetadata[]): SqlCompletionEntry[] {
  return procedures.map((procedure) => ({
    label: procedure.id,
    detail: formatSignatureDetail(procedure),
    category: "function",
    insertText: callInsertText(procedure),
    isSnippet: true,
    sortText: `0_call_${procedure.id}`,
  }));
}

export function callInsertText(procedure: ProcedureMetadata, alreadyHasCall = false): string {
  const args = procedure.parameters
    .map((parameter, index) => `\${${index + 1}:${parameter.name}}`)
    .join(", ");
  const call = alreadyHasCall ? "" : "CALL ";
  if (procedure.parameters.length === 0) {
    return `${call}${procedure.id}()`;
  }
  return `${call}${procedure.id}(${args})`;
}

export function valueSuggestionsForType(type: ResolvedKalamType): SqlCompletionEntry[] {
  const suggestions: SqlCompletionEntry[] = [];
  if (!type.notNull) {
    suggestions.push({
      label: "NULL",
      detail: `${formatDeclaredType(type)} null value`,
      category: "keyword",
      insertText: "NULL",
      sortText: "0_null",
    });
  }

  switch (type.kind) {
    case "builtin": {
      const builtin = type.builtin.toUpperCase();
      if (builtin === "BOOLEAN" || builtin === "BOOL") {
        suggestions.push(
          {
            label: "TRUE",
            detail: "BOOLEAN",
            category: "keyword",
            insertText: "TRUE",
            sortText: "1_true",
          },
          {
            label: "FALSE",
            detail: "BOOLEAN",
            category: "keyword",
            insertText: "FALSE",
            sortText: "1_false",
          },
        );
      }
      if (builtin === "TIMESTAMP" || builtin === "TIMESTAMPTZ" || builtin === "DATETIME") {
        suggestions.push(
          {
            label: "NOW()",
            detail: type.sqlName,
            category: "function",
            insertText: "NOW()",
            sortText: "1_now",
          },
          {
            label: "CURRENT_TIMESTAMP",
            detail: type.sqlName,
            category: "function",
            insertText: "CURRENT_TIMESTAMP",
            sortText: "1_current_timestamp",
          },
        );
      }
      if (builtin === "JSON" || builtin === "JSONB") {
        suggestions.push({
          label: "'{}'",
          detail: `${type.sqlName} placeholder`,
          category: "snippet",
          insertText: "'{}'",
          isSnippet: true,
          sortText: "1_json",
        });
      }
      break;
    }
    case "enum":
      type.values.forEach((value, index) => {
        suggestions.push({
          label: `'${value}'`,
          detail: type.sqlName,
          category: "snippet",
          insertText: `'${value}'`,
          sortText: `1_enum_${index}_${value}`,
        });
      });
      break;
    case "array":
    case "composite":
    case "unknown":
      break;
  }

  return suggestions;
}

export function signatureHelpLabel(procedure: ProcedureMetadata): string {
  const args = procedure.parameters.map(formatParameterDeclaration).join(", ");
  return `${procedure.id}(${args})\nRETURNS ${procedure.returnTypeName}`;
}

export function signatureParameterLabel(parameter: ProcedureParameter): string {
  return formatParameterDeclaration(parameter);
}

export function hoverDocumentation(procedure: ProcedureMetadata): string {
  const lines = [
    procedure.id,
    "",
    "Parameters:",
  ];
  if (procedure.parameters.length === 0) {
    lines.push("(none)");
  } else {
    for (const parameter of procedure.parameters) {
      lines.push(formatParameterDeclaration(parameter));
      lines.push(...parameterTypeOutline(parameter.type));
    }
  }
  lines.push("", "Returns:", procedure.returnTypeName || "VOID");
  lines.push(...parameterTypeOutline(procedure.returnType));
  if (procedure.security) {
    lines.push("", "Security:", procedure.security);
  }
  if (procedure.comment) {
    lines.push("", "Description:", procedure.comment);
  }
  return lines.join("\n");
}

export function parameterTypeOutline(type: ResolvedKalamType): string[] {
  const outline = formatCompositeOutline(type);
  if (outline.length <= 1) {
    return [];
  }
  return outline;
}

export function parameterSignatureDocumentation(parameter: ProcedureParameter): string {
  const outline = parameterTypeOutline(parameter.type);
  if (outline.length === 0) {
    return formatDeclaredType(parameter.type);
  }
  return [formatDeclaredType(parameter.type), "", ...outline].join("\n");
}

export function completionsForCallContext(
  context: CallCompletionContext,
  procedures: ProcedureMetadata[],
): SqlCompletionEntry[] {
  if (context.kind === "name") {
    return procedureCompletionEntries(filterProceduresByPartial(procedures, context.partial)).map((entry) => ({
      ...entry,
      insertText: entry.insertText?.replace(/^CALL\s+/i, ""),
    }));
  }

  const procedure = findProcedure(procedures, context.procedureId);
  const parameter = procedure?.parameters[context.activeParameter];
  if (!parameter) {
    return [];
  }
  return valueSuggestionsForType(parameter.type);
}

export interface ProcedureSignatureHelp {
  label: string;
  documentation: string;
  activeParameter: number;
  parameters: Array<{ label: string; documentation: string }>;
}

export function signatureHelpForCall(
  context: CallArgumentContext,
  procedures: ProcedureMetadata[],
): ProcedureSignatureHelp | null {
  const procedure = findProcedure(procedures, context.procedureId);
  if (!procedure) {
    return null;
  }

  const lastIndex = Math.max(0, procedure.parameters.length - 1);
  return {
    label: signatureHelpLabel(procedure),
    documentation: procedure.comment ?? formatSignatureDetail(procedure),
    activeParameter: Math.min(context.activeParameter, lastIndex),
    parameters: procedure.parameters.map((parameter) => ({
      label: signatureParameterLabel(parameter),
      documentation: parameterSignatureDocumentation(parameter),
    })),
  };
}

export function hoverForIdentifier(
  identifier: string,
  procedures: ProcedureMetadata[],
): string | null {
  const procedure = findProcedure(procedures, identifier);
  if (!procedure) {
    return null;
  }
  return hoverDocumentation(procedure);
}

export function findProcedure(
  procedures: ProcedureMetadata[],
  procedureId: string,
): ProcedureMetadata | undefined {
  const needle = procedureId.toLowerCase();
  return procedures.find((procedure) => procedure.id.toLowerCase() === needle);
}

export function filterProceduresByPartial(
  procedures: ProcedureMetadata[],
  partial: string,
): ProcedureMetadata[] {
  const needle = partial.trim().toLowerCase();
  if (!needle) {
    return procedures;
  }
  return procedures.filter((procedure) => procedure.id.toLowerCase().includes(needle));
}

export function readQualifiedIdentifier(text: string, offset: number): string {
  let start = offset;
  let end = offset;
  while (start > 0 && /[A-Za-z0-9_.]/.test(text[start - 1] ?? "")) {
    start -= 1;
  }
  while (end < text.length && /[A-Za-z0-9_.]/.test(text[end] ?? "")) {
    end += 1;
  }
  return text.slice(start, end);
}
