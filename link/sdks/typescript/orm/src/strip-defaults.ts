export function stripDefaults(sql: string, params: unknown[]): { sql: string; params: unknown[] } {
  if (/on conflict/i.test(sql)) {
    return { sql, params };
  }

  const match = sql.match(/^(INSERT\s+INTO\s+\S+)\s*\(([^)]+)\)\s*VALUES\s*/i);
  if (!match) return { sql, params };

  const prefix = match[1];
  const columns = match[2].split(',').map((c) => c.trim());
  const valuesSql = sql.slice(match[0].length);

  const valueGroups: string[][] = [];
  const remaining = valuesSql.trim();
  const groupRegex = /\(([^)]+)\)/g;
  let groupMatch;
  let lastIndex = 0;
  while ((groupMatch = groupRegex.exec(remaining)) !== null) {
    valueGroups.push(groupMatch[1].split(',').map((v) => v.trim()));
    lastIndex = groupRegex.lastIndex;
  }
  if (valueGroups.length === 0) return { sql, params };

  const suffix = remaining.slice(lastIndex).trim();
  const firstGroup = valueGroups[0];
  const keepIndices: number[] = [];
  for (let i = 0; i < firstGroup.length; i++) {
    if (firstGroup[i].toUpperCase() !== 'DEFAULT') keepIndices.push(i);
  }
  if (keepIndices.length === columns.length) return { sql, params };

  const newColumns = keepIndices.map((i) => columns[i]);
  const newParams: unknown[] = [];
  const newValueGroups = valueGroups.map((group) => {
    const vals = keepIndices.map((i) => {
      const val = group[i];
      const paramMatch = val.match(/^\$(\d+)$/);
      if (paramMatch) {
        newParams.push(params[parseInt(paramMatch[1], 10) - 1]);
        return `$${newParams.length}`;
      }
      return val;
    });
    return `(${vals.join(', ')})`;
  });

  const rewritten = `${prefix} (${newColumns.join(', ')}) VALUES ${newValueGroups.join(', ')}`;
  return {
    sql: suffix ? `${rewritten} ${suffix}` : rewritten,
    params: newParams,
  };
}
