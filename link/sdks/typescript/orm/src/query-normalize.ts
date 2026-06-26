export function stripQuotedIdentifiers(sql: string): string {
  let normalized = '';
  let index = 0;
  let inSingleQuotedString = false;

  while (index < sql.length) {
    const char = sql[index];

    if (char === "'") {
      normalized += char;
      if (inSingleQuotedString && sql[index + 1] === "'") {
        normalized += sql[index + 1];
        index += 2;
        continue;
      }
      inSingleQuotedString = !inSingleQuotedString;
      index += 1;
      continue;
    }

    if (!inSingleQuotedString && char === '"') {
      const prefix = sql.slice(0, index);
      if (/FILE\s*\(\s*$/i.test(prefix)) {
        const end = sql.indexOf('"', index + 1);
        if (end !== -1) {
          normalized += sql.slice(index, end + 1);
          index = end + 1;
          continue;
        }
      }

      const end = sql.indexOf('"', index + 1);
      if (end !== -1) {
        normalized += sql.slice(index + 1, end);
        index = end + 1;
        continue;
      }
    }

    normalized += char;
    index += 1;
  }

  return normalized;
}
