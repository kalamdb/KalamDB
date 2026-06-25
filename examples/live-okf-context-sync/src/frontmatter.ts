export type Frontmatter = Record<string, string | string[] | boolean | number>;

const FRONTMATTER_RE = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?/;

export function parseFrontmatter(markdown: string): Frontmatter | null {
  const match = markdown.match(FRONTMATTER_RE);
  if (!match) {
    return null;
  }

  const frontmatter: Frontmatter = {};
  for (const line of match[1].split('\n')) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith('#')) {
      continue;
    }

    const colon = trimmed.indexOf(':');
    if (colon === -1) {
      continue;
    }

    const key = trimmed.slice(0, colon).trim();
    let value = trimmed.slice(colon + 1).trim();

    if (value.startsWith('[') && value.endsWith(']')) {
      frontmatter[key] = value
        .slice(1, -1)
        .split(',')
        .map((item) => item.trim().replace(/^['"]|['"]$/g, ''))
        .filter(Boolean);
      continue;
    }

    if (value === 'true' || value === 'false') {
      frontmatter[key] = value === 'true';
      continue;
    }

    if (/^-?\d+(\.\d+)?$/.test(value)) {
      frontmatter[key] = Number(value);
      continue;
    }

    frontmatter[key] = value.replace(/^['"]|['"]$/g, '');
  }

  return Object.keys(frontmatter).length > 0 ? frontmatter : null;
}

export function stripFrontmatter(markdown: string): string {
  return markdown.replace(FRONTMATTER_RE, '');
}
