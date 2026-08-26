import type {
  DraftColumn,
  DraftPolicyCommand,
  DraftTable,
  DraftTablePolicy,
  DraftPolicyTarget,
} from "./types";

function quoteIdent(name: string): string {
  if (/^[a-zA-Z_][a-zA-Z0-9_]*$/.test(name)) return name;
  return `"${name.replace(/"/g, '""')}"`;
}

function qualifiedName(namespace: string, name: string): string {
  return `${quoteIdent(namespace)}.${quoteIdent(name)}`;
}

function columnClause(col: DraftColumn): string {
  const parts: string[] = [quoteIdent(col.name), col.type];
  if (col.isPrimaryKey) parts.push("PRIMARY KEY");
  if (col.isNotNull) parts.push("NOT NULL");
  if (col.isUnique) parts.push("UNIQUE");
  if (col.defaultExpr.trim().length > 0)
    parts.push(`DEFAULT ${col.defaultExpr.trim()}`);
  return parts.join(" ");
}

function quoteLiteral(value: string): string {
  return `'${value.replace(/'/g, "''")}'`;
}

function upperOption(value: string): string {
  return value.trim().toUpperCase();
}

function positiveInteger(value: string): number | null {
  const parsed = Number(value);
  return Number.isInteger(parsed) && parsed > 0 ? parsed : null;
}

function nonNegativeInteger(value: string): number | null {
  const parsed = Number(value);
  return Number.isInteger(parsed) && parsed >= 0 ? parsed : null;
}

function flushPolicySql(draft: DraftTable): string | null {
  const { flushPolicyKind, flushRows, flushIntervalSeconds } = draft.options;
  if (flushPolicyKind === "none") return null;
  if (flushPolicyKind === "rows") return `rows:${flushRows.trim()}`;
  if (flushPolicyKind === "interval")
    return `interval:${flushIntervalSeconds.trim()}`;
  return `rows:${flushRows.trim()},interval:${flushIntervalSeconds.trim()}`;
}

function tablePropertyMap(
  draft: DraftTable,
  includeType: boolean,
  includeFlushNull: boolean,
): Map<string, string> {
  const props = new Map<string, string>();
  const options = draft.options;
  if (includeType)
    props.set("TYPE", quoteLiteral(upperOption(draft.tableType)));

  if (draft.tableType === "user") {
    props.set("STORAGE_ID", quoteLiteral(options.storageId.trim()));
    props.set("USE_USER_STORAGE", options.useUserStorage ? "true" : "false");
    const flushPolicy = flushPolicySql(draft);
    if (flushPolicy) props.set("FLUSH_POLICY", quoteLiteral(flushPolicy));
    else if (includeFlushNull) props.set("FLUSH_POLICY", "NULL");
    props.set("COMPRESSION", quoteLiteral(options.compression));
  } else if (draft.tableType === "shared") {
    props.set("STORAGE_ID", quoteLiteral(options.storageId.trim()));
    const flushPolicy = flushPolicySql(draft);
    if (flushPolicy) props.set("FLUSH_POLICY", quoteLiteral(flushPolicy));
    else if (includeFlushNull) props.set("FLUSH_POLICY", "NULL");
    props.set("COMPRESSION", quoteLiteral(options.compression));
  } else {
    props.set("TTL_SECONDS", options.ttlSeconds.trim());
    props.set("EVICTION_STRATEGY", quoteLiteral(options.evictionStrategy));
    props.set("MAX_STREAM_SIZE_BYTES", options.maxStreamSizeBytes.trim());
  }

  return props;
}

function formatProperties(props: Map<string, string>): string {
  return Array.from(props.entries())
    .map(([key, value]) => `${key} = ${value}`)
    .join(", ");
}

export interface DraftValidation {
  table: string[];
  name: string | null;
  columns: Record<string, string>;
  policies: Record<string, string>;
  hasAny: boolean;
}

export function validateDraft(draft: DraftTable): DraftValidation {
  const result: DraftValidation = {
    table: [],
    name: null,
    columns: {},
    policies: {},
    hasAny: false,
  };

  if (!draft.name.trim()) {
    result.name = "Table name is required";
  }

  if (
    (draft.tableType === "user" || draft.tableType === "shared") &&
    !draft.options.storageId.trim()
  ) {
    result.table.push("Storage ID is required for user and shared tables");
  }

  const liveCols = draft.columns.filter((c) => !c.isDeleted);
  if (liveCols.length === 0) {
    result.table.push("Table must have at least one column");
  }

  const seen = new Map<string, string[]>();
  for (const c of liveCols) {
    const trimmed = c.name.trim();
    if (!trimmed) {
      result.columns[c.id] = "Name is required";
      continue;
    }
    if (!c.type.trim()) {
      result.columns[c.id] = "Type is required";
      continue;
    }
    const lower = trimmed.toLowerCase();
    if (!seen.has(lower)) seen.set(lower, []);
    seen.get(lower)!.push(c.id);
  }
  for (const [, ids] of seen) {
    if (ids.length > 1) {
      for (const id of ids.slice(1)) {
        result.columns[id] = "Duplicate column name";
      }
    }
  }

  const pkCount = liveCols.filter((c) => c.isPrimaryKey).length;
  if (
    (draft.tableType === "user" || draft.tableType === "shared") &&
    pkCount === 0
  ) {
    result.table.push("User and shared tables require one PRIMARY KEY column");
  }
  if (pkCount > 1) {
    result.table.push("Only one column can be marked as PRIMARY KEY");
  }

  if (draft.tableType === "user" || draft.tableType === "shared") {
    if (
      draft.options.flushPolicyKind === "rows" ||
      draft.options.flushPolicyKind === "combined"
    ) {
      const rows = positiveInteger(draft.options.flushRows);
      if (rows === null || rows >= 1_000_000) {
        result.table.push("Flush row limit must be between 1 and 999999");
      }
    }
    if (
      draft.options.flushPolicyKind === "interval" ||
      draft.options.flushPolicyKind === "combined"
    ) {
      const interval = positiveInteger(draft.options.flushIntervalSeconds);
      if (interval === null || interval >= 86_400) {
        result.table.push("Flush interval must be between 1 and 86399 seconds");
      }
    }
  }

  if (draft.tableType === "stream") {
    if (positiveInteger(draft.options.ttlSeconds) === null) {
      result.table.push("TTL seconds must be greater than 0");
    }
    if (nonNegativeInteger(draft.options.maxStreamSizeBytes) === null) {
      result.table.push("Max stream size must be 0 or greater");
    }
  }

  if (draft.tableType === "shared") {
    validatePolicies(draft, result);
  }

  result.hasAny =
    result.table.length > 0 ||
    result.name !== null ||
    Object.keys(result.columns).length > 0 ||
    Object.keys(result.policies).length > 0;
  return result;
}

export function policyUsesUsing(command: DraftPolicyCommand): boolean {
  return command !== "insert";
}

export function policyUsesWithCheck(command: DraftPolicyCommand): boolean {
  return command === "all" || command === "insert" || command === "update";
}

function validatePolicies(draft: DraftTable, result: DraftValidation): void {
  const live = draft.policies.filter((policy) => !policy.isDeleted);
  const seen = new Map<string, string[]>();
  for (const policy of live) {
    const name = policy.name.trim();
    if (!name) {
      result.policies[policy.id] = "Policy name is required";
      continue;
    }
    const lower = name.toLowerCase();
    if (!seen.has(lower)) seen.set(lower, []);
    seen.get(lower)!.push(policy.id);
    if (policy.targets.length === 0) {
      result.policies[policy.id] = "Select at least one role";
      continue;
    }
    if (policyUsesUsing(policy.command) && !policy.usingExpr.trim()) {
      result.policies[policy.id] = "USING expression is required";
      continue;
    }
    if (policyUsesWithCheck(policy.command) && !policy.withCheckExpr.trim()) {
      if (policy.command === "insert") {
        result.policies[policy.id] = "WITH CHECK expression is required";
      } else if (!policy.usingExpr.trim()) {
        result.policies[policy.id] =
          "USING or WITH CHECK expression is required";
      }
    }
  }
  for (const [, ids] of seen) {
    if (ids.length > 1) {
      for (const id of ids.slice(1)) {
        result.policies[id] = "Duplicate policy name";
      }
    }
  }
}

function wrapExpr(expr: string): string {
  const trimmed = expr.trim();
  if (trimmed.startsWith("(") && trimmed.endsWith(")")) return trimmed;
  return `(${trimmed})`;
}

function formatPolicyTargets(targets: DraftPolicyTarget[]): string {
  if (targets.includes("public")) return "PUBLIC";
  return targets.join(", ");
}

export function generateCreatePolicySql(
  namespace: string,
  tableName: string,
  policy: DraftTablePolicy,
): string {
  const fqn = qualifiedName(namespace, tableName);
  const parts = [
    `CREATE POLICY ${quoteIdent(policy.name.trim())} ON ${fqn}`,
    `FOR ${upperOption(policy.command)}`,
    `TO ${formatPolicyTargets(policy.targets)}`,
  ];
  if (policyUsesUsing(policy.command) && policy.usingExpr.trim()) {
    parts.push(`USING ${wrapExpr(policy.usingExpr)}`);
  }
  if (policyUsesWithCheck(policy.command) && policy.withCheckExpr.trim()) {
    parts.push(`WITH CHECK ${wrapExpr(policy.withCheckExpr)}`);
  }
  return `${parts.join(" ")};`;
}

function generateDropPolicySql(
  namespace: string,
  tableName: string,
  policyName: string,
): string {
  return `DROP POLICY ${quoteIdent(policyName.trim())} ON ${qualifiedName(namespace, tableName)};`;
}

function generateRenamePolicySql(
  namespace: string,
  tableName: string,
  fromName: string,
  toName: string,
): string {
  return `ALTER POLICY ${quoteIdent(fromName.trim())} ON ${qualifiedName(namespace, tableName)} RENAME TO ${quoteIdent(toName.trim())};`;
}

function generateAlterPolicyApplySql(
  namespace: string,
  tableName: string,
  policy: DraftTablePolicy,
  original: DraftTablePolicy,
): string | null {
  const clauses: string[] = [];
  const sameTargets =
    policy.targets.length === original.targets.length &&
    policy.targets.every((target) => original.targets.includes(target));
  if (!sameTargets) {
    clauses.push(`TO ${formatPolicyTargets(policy.targets)}`);
  }
  if (
    policyUsesUsing(policy.command) &&
    policy.usingExpr.trim() !== original.usingExpr.trim()
  ) {
    clauses.push(`USING ${wrapExpr(policy.usingExpr)}`);
  }
  if (
    policyUsesWithCheck(policy.command) &&
    policy.withCheckExpr.trim() !== original.withCheckExpr.trim() &&
    policy.withCheckExpr.trim()
  ) {
    clauses.push(`WITH CHECK ${wrapExpr(policy.withCheckExpr)}`);
  }
  if (clauses.length === 0) return null;
  return `ALTER POLICY ${quoteIdent(policy.name.trim())} ON ${qualifiedName(namespace, tableName)} ${clauses.join(" ")};`;
}

export function generatePolicyDdl(
  original: DraftTable | null,
  draft: DraftTable,
): string[] {
  if (draft.tableType !== "shared") return [];
  const liveCreates = draft.policies.filter(
    (policy) => !policy.isDeleted && (original == null || policy.isNew),
  );
  if (original == null) {
    return liveCreates.map((policy) =>
      generateCreatePolicySql(draft.namespace, draft.name, policy),
    );
  }
  const stmts: string[] = [];
  const originalById = new Map<string, DraftTablePolicy>();
  for (const policy of original?.policies ?? []) {
    originalById.set(policy.id, policy);
  }

  for (const policy of draft.policies) {
    if (policy.isDeleted && !policy.isNew) {
      const orig = originalById.get(policy.id);
      stmts.push(
        generateDropPolicySql(
          draft.namespace,
          draft.name,
          orig?.name ?? policy.name,
        ),
      );
    }
  }

  for (const policy of draft.policies) {
    if (policy.isNew && !policy.isDeleted) {
      stmts.push(generateCreatePolicySql(draft.namespace, draft.name, policy));
      continue;
    }
    if (policy.isNew || policy.isDeleted) continue;
    const orig = originalById.get(policy.id);
    if (!orig) continue;
    const nameChanged = orig.name.trim() !== policy.name.trim();
    const working: DraftTablePolicy = nameChanged
      ? { ...policy, name: orig.name }
      : policy;
    const apply = generateAlterPolicyApplySql(
      draft.namespace,
      draft.name,
      working,
      orig,
    );
    if (apply) stmts.push(apply);
    if (nameChanged) {
      stmts.push(
        generateRenamePolicySql(
          draft.namespace,
          draft.name,
          orig.name,
          policy.name,
        ),
      );
    }
  }

  return stmts;
}

export function generateCreateTableSql(draft: DraftTable): string {
  const cols = draft.columns
    .filter((c) => !c.isDeleted)
    .map((c) => columnClause(c))
    .join(", ");
  const props = tablePropertyMap(draft, true, false);
  const createTable = `CREATE TABLE ${qualifiedName(draft.namespace, draft.name)} (${cols}) WITH (${formatProperties(props)});`;
  const policies = generatePolicyDdl(null, draft);
  return [createTable, ...policies].join("\n");
}

export function generateAlterTableSql(
  original: DraftTable,
  draft: DraftTable,
): string {
  const stmts: string[] = [];
  const fqn = qualifiedName(draft.namespace, draft.name);

  const originalById = new Map<string, DraftColumn>();
  for (const col of original.columns) originalById.set(col.id, col);

  const originalProps = tablePropertyMap(original, false, true);
  const draftProps = tablePropertyMap(draft, false, true);
  const changedProps = new Map<string, string>();
  for (const [key, value] of draftProps) {
    if (originalProps.get(key) !== value) {
      changedProps.set(key, value);
    }
  }
  if (changedProps.size > 0) {
    stmts.push(
      `ALTER TABLE ${fqn} SET TBLPROPERTIES (${formatProperties(changedProps)});`,
    );
  }

  for (const col of draft.columns) {
    if (col.isDeleted && !col.isNew) {
      stmts.push(`ALTER TABLE ${fqn} DROP COLUMN ${quoteIdent(col.name)};`);
    }
  }

  for (const col of draft.columns) {
    if (col.isNew && !col.isDeleted) {
      stmts.push(`ALTER TABLE ${fqn} ADD COLUMN ${columnClause(col)};`);
    }
  }

  for (const col of draft.columns) {
    if (col.isNew || col.isDeleted) continue;
    const orig = originalById.get(col.id);
    if (!orig) continue;

    if (orig.name !== col.name) {
      stmts.push(
        `ALTER TABLE ${fqn} RENAME COLUMN ${quoteIdent(orig.name)} TO ${quoteIdent(col.name)};`,
      );
    }
    if (orig.type !== col.type) {
      stmts.push(
        `ALTER TABLE ${fqn} MODIFY COLUMN ${quoteIdent(col.name)} ${col.type};`,
      );
    }
    if (orig.isNotNull !== col.isNotNull) {
      stmts.push(
        `ALTER TABLE ${fqn} ALTER COLUMN ${quoteIdent(col.name)} ${col.isNotNull ? "SET NOT NULL" : "DROP NOT NULL"};`,
      );
    }
    if (orig.defaultExpr !== col.defaultExpr) {
      if (col.defaultExpr.trim().length === 0) {
        stmts.push(
          `ALTER TABLE ${fqn} ALTER COLUMN ${quoteIdent(col.name)} DROP DEFAULT;`,
        );
      } else {
        stmts.push(
          `ALTER TABLE ${fqn} ALTER COLUMN ${quoteIdent(col.name)} SET DEFAULT ${col.defaultExpr.trim()};`,
        );
      }
    }
  }

  stmts.push(...generatePolicyDdl(original, draft));

  return stmts.join("\n");
}

export function generateDropTableSql(namespace: string, name: string): string {
  return `DROP TABLE ${qualifiedName(namespace, name)};`;
}
