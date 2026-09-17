import { classifyFieldKind } from "@/components/sql-studio-v2/shared/value-validation";
import { formatSqlType } from "./format";
import type { ProcedureMetadata, ProcedureParameter, ResolvedKalamType } from "./types";

const UUID_REGEX = /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/;

export type TestValue = unknown;

export interface TestFieldError {
  path: string;
  message: string;
}

function isBlank(value: unknown): boolean {
  return value === undefined || value === null || (typeof value === "string" && value.trim() === "");
}

function builtinKind(type: ResolvedKalamType): string {
  if (type.kind !== "builtin") {
    return "text";
  }
  return classifyFieldKind(type.sqlName);
}

export function defaultValueForType(type: ResolvedKalamType): TestValue {
  if (!type.notNull) {
    return null;
  }

  switch (type.kind) {
    case "array":
      return type.nonempty ? [defaultValueForType({ ...type.element, notNull: type.element.notNull })] : [];
    case "composite": {
      const value: Record<string, TestValue> = {};
      for (const field of type.fields) {
        value[field.name] = defaultValueForType(field.type);
      }
      return value;
    }
    case "enum":
      return type.values[0] ?? "";
    case "builtin": {
      const kind = builtinKind(type);
      if (kind === "boolean") {
        return false;
      }
      if (kind === "json") {
        return {};
      }
      if (kind === "int" || kind === "smallint" || kind === "bigint" || kind === "float" || kind === "decimal") {
        return "";
      }
      return "";
    }
    case "unknown":
      return "";
  }
}

export function defaultTestValues(procedure: ProcedureMetadata): Record<string, TestValue> {
  const values: Record<string, TestValue> = {};
  for (const parameter of procedure.parameters) {
    values[parameter.name] = defaultValueForType(parameter.type);
  }
  return values;
}

function validateType(path: string, type: ResolvedKalamType, value: TestValue, errors: TestFieldError[]) {
  if (isBlank(value)) {
    if (type.notNull) {
      errors.push({ path, message: `${path} is required` });
    }
    return;
  }

  if (value === null) {
    if (type.notNull) {
      errors.push({ path, message: `${path} is required` });
    }
    return;
  }

  switch (type.kind) {
    case "array": {
      if (!Array.isArray(value)) {
        errors.push({ path, message: `${path} must be an array of ${formatSqlType(type.element)}` });
        return;
      }
      if (type.nonempty && value.length === 0) {
        errors.push({ path, message: `${path} must contain at least one item` });
      }
      value.forEach((item, index) => {
        validateType(`${path}[${index}]`, type.element, item, errors);
      });
      return;
    }
    case "composite": {
      if (typeof value !== "object" || Array.isArray(value)) {
        errors.push({ path, message: `${path} must be an object of type ${type.sqlName}` });
        return;
      }
      const record = value as Record<string, TestValue>;
      for (const field of type.fields) {
        validateType(`${path}.${field.name}`, field.type, record[field.name], errors);
      }
      return;
    }
    case "enum": {
      if (typeof value !== "string" || !type.values.includes(value)) {
        errors.push({
          path,
          message: `${path} must be one of: ${type.values.map((entry) => `'${entry}'`).join(", ")}`,
        });
      }
      return;
    }
    case "builtin": {
      const kind = builtinKind(type);
      if (kind === "boolean" && typeof value !== "boolean") {
        errors.push({ path, message: `${path} must be true or false` });
        return;
      }
      if (kind === "uuid" && (typeof value !== "string" || !UUID_REGEX.test(value.trim()))) {
        errors.push({ path, message: `${path} must be a UUID` });
        return;
      }
      if ((kind === "int" || kind === "smallint" || kind === "bigint") && typeof value === "string") {
        if (!/^-?\d+$/.test(value.trim())) {
          errors.push({ path, message: `${path} must be a whole number` });
        }
        return;
      }
      if ((kind === "float" || kind === "decimal") && typeof value === "string") {
        if (!/^-?(\d+\.?\d*|\.\d+)([eE][+\-]?\d+)?$/.test(value.trim())) {
          errors.push({ path, message: `${path} must be a number` });
        }
        return;
      }
      if (kind === "json" && typeof value === "string") {
        try {
          JSON.parse(value);
        } catch {
          errors.push({ path, message: `${path} must be valid JSON` });
        }
      }
      return;
    }
    case "unknown":
      return;
  }
}

export function validateTestValues(
  procedure: ProcedureMetadata,
  values: Record<string, TestValue>,
): TestFieldError[] {
  const errors: TestFieldError[] = [];
  for (const parameter of procedure.parameters) {
    validateType(parameter.name, parameter.type, values[parameter.name], errors);
  }
  return errors;
}

function coerceBuiltin(type: ResolvedKalamType, value: TestValue): TestValue {
  if (value === null || value === undefined) {
    return null;
  }
  if (type.kind !== "builtin") {
    return value;
  }
  const kind = builtinKind(type);
  if (kind === "json" && typeof value === "string") {
    return JSON.parse(value) as TestValue;
  }
  if ((kind === "int" || kind === "smallint") && typeof value === "string") {
    return Number(value);
  }
  if (kind === "bigint" && typeof value === "string") {
    return value.trim();
  }
  if ((kind === "float" || kind === "decimal") && typeof value === "string") {
    return Number(value);
  }
  if (kind === "datetime" && typeof value === "string") {
    const trimmed = value.trim();
    const localMatch = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})(?::(\d{2}))?$/.exec(trimmed);
    if (localMatch) {
      const [, year, month, day, hour, minute, second] = localMatch;
      return `${year}-${month}-${day}T${hour}:${minute}:${second ?? "00"}Z`;
    }
    return trimmed;
  }
  if (kind === "uuid" && typeof value === "string") {
    return value.trim();
  }
  if (typeof value === "string") {
    return value;
  }
  return value;
}

function coerceValue(type: ResolvedKalamType, value: TestValue): TestValue {
  if (value === undefined || value === null || (typeof value === "string" && value.trim() === "")) {
    return null;
  }

  switch (type.kind) {
    case "array":
      return Array.isArray(value) ? value.map((item) => coerceValue(type.element, item)) : value;
    case "composite": {
      if (typeof value !== "object" || Array.isArray(value)) {
        return value;
      }
      const record = value as Record<string, TestValue>;
      const next: Record<string, TestValue> = {};
      for (const field of type.fields) {
        next[field.name] = coerceValue(field.type, record[field.name]);
      }
      return next;
    }
    case "enum":
      return value;
    case "builtin":
      return coerceBuiltin(type, value);
    case "unknown":
      return value;
  }
}

export function buildInvocationBody(
  procedure: ProcedureMetadata,
  values: Record<string, TestValue>,
): Record<string, TestValue> {
  const body: Record<string, TestValue> = {};
  for (const parameter of procedure.parameters) {
    body[parameter.name] = coerceValue(parameter.type, values[parameter.name]);
  }
  return body;
}

export function setPathValue(
  values: Record<string, TestValue>,
  path: string,
  nextValue: TestValue,
): Record<string, TestValue> {
  const tokens = tokenizePath(path);
  const clone = structuredClone(values) as Record<string, TestValue>;
  writePath(clone, tokens, nextValue);
  return clone;
}

function tokenizePath(path: string): Array<string | number> {
  const tokens: Array<string | number> = [];
  const matcher = /([^[.\]]+)|\[(\d+)\]/g;
  let match = matcher.exec(path);
  while (match) {
    if (match[1]) {
      tokens.push(match[1]);
    } else if (match[2]) {
      tokens.push(Number(match[2]));
    }
    match = matcher.exec(path);
  }
  return tokens;
}

function writePath(target: Record<string, TestValue> | TestValue[], tokens: Array<string | number>, nextValue: TestValue) {
  const [head, ...rest] = tokens;
  if (head === undefined) {
    return;
  }
  if (rest.length === 0) {
    if (typeof head === "number" && Array.isArray(target)) {
      target[head] = nextValue;
      return;
    }
    if (typeof head === "string" && !Array.isArray(target)) {
      target[head] = nextValue;
    }
    return;
  }

  if (typeof head === "number" && Array.isArray(target)) {
    const nested = target[head];
    if (nested && typeof nested === "object") {
      writePath(nested as Record<string, TestValue> | TestValue[], rest, nextValue);
    }
    return;
  }

  if (typeof head === "string" && !Array.isArray(target)) {
    const nested = target[head];
    if (nested && typeof nested === "object") {
      writePath(nested as Record<string, TestValue> | TestValue[], rest, nextValue);
    }
  }
}

export function parameterByPath(
  procedure: ProcedureMetadata,
  path: string,
): ProcedureParameter | ResolvedKalamType | null {
  const tokens = tokenizePath(path);
  if (tokens.length === 0 || typeof tokens[0] !== "string") {
    return null;
  }
  const parameter = procedure.parameters.find((item) => item.name === tokens[0]);
  if (!parameter) {
    return null;
  }
  if (tokens.length === 1) {
    return parameter;
  }
  return typeAtTokens(parameter.type, tokens.slice(1));
}

function typeAtTokens(type: ResolvedKalamType, tokens: Array<string | number>): ResolvedKalamType | null {
  if (tokens.length === 0) {
    return type;
  }
  const [head, ...rest] = tokens;
  if (typeof head === "number") {
    if (type.kind !== "array") {
      return null;
    }
    return typeAtTokens(type.element, rest);
  }
  if (type.kind !== "composite") {
    return null;
  }
  const field = type.fields.find((item) => item.name === head);
  if (!field) {
    return null;
  }
  return typeAtTokens(field.type, rest);
}
