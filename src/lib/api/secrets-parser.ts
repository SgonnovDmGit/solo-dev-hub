export interface ParsedSecret {
  name: string;
  value: string;
}

export interface ParseResult {
  secrets: ParsedSecret[];
  errors: string[];
}

const SECRET_NAME_RE = /^[A-Z_][A-Z0-9_]*$/;
const TRIPLE_QUOTE = '"""';

function validateName(name: string, lineNo: number, errors: string[]): boolean {
  if (!SECRET_NAME_RE.test(name)) {
    errors.push(`Line ${lineNo}: invalid key '${name}' (must be A-Z, 0-9, _ only, start with letter or _)`);
    return false;
  }
  if (name.startsWith('GITHUB_')) {
    errors.push(`Line ${lineNo}: key '${name}' cannot start with GITHUB_`);
    return false;
  }
  return true;
}

export function parseEnvText(text: string): ParseResult {
  const secrets: ParsedSecret[] = [];
  const errors: string[] = [];
  const lines = text.split('\n');

  let i = 0;
  while (i < lines.length) {
    const rawLine = lines[i];
    const line = rawLine.trim();

    if (line === '' || line.startsWith('#')) {
      i++;
      continue;
    }

    const eqIndex = line.indexOf('=');
    if (eqIndex === -1) {
      errors.push(`Line ${i + 1}: missing '=' separator`);
      i++;
      continue;
    }

    const name = line.substring(0, eqIndex).trim();
    const afterEq = line.substring(eqIndex + 1);

    // Triple-quoted multi-line: KEY="""...
    if (afterEq.trimStart().startsWith(TRIPLE_QUOTE)) {
      const startLineNo = i + 1;
      if (!validateName(name, startLineNo, errors)) { i++; continue; }

      // Strip whitespace before """ and remove """ prefix.
      const firstTail = afterEq.trimStart().substring(TRIPLE_QUOTE.length);

      // Check for inline close on the same line: KEY="""value"""
      const inlineCloseIdx = firstTail.indexOf(TRIPLE_QUOTE);
      if (inlineCloseIdx !== -1) {
        const value = firstTail.substring(0, inlineCloseIdx);
        if (value === '') {
          errors.push(`Line ${startLineNo}: empty value for '${name}'`);
        } else {
          secrets.push({ name, value });
        }
        i++;
        continue;
      }

      // Multi-line: collect lines until closing """
      const collected: string[] = [];
      // Content after """ on the opening line (if any and no inline close) — ignored unless non-empty
      const openingTail = firstTail;
      if (openingTail.length > 0) collected.push(openingTail);

      i++;
      let closed = false;
      while (i < lines.length) {
        const current = lines[i];
        const closeIdx = current.indexOf(TRIPLE_QUOTE);
        if (closeIdx !== -1) {
          if (closeIdx > 0) collected.push(current.substring(0, closeIdx));
          closed = true;
          i++;
          break;
        }
        collected.push(current);
        i++;
      }

      if (!closed) {
        errors.push(`Line ${startLineNo}: unclosed triple-quoted value for '${name}'`);
        continue;
      }

      const value = collected.join('\n');
      if (value === '') {
        errors.push(`Line ${startLineNo}: empty value for '${name}'`);
      } else {
        secrets.push({ name, value });
      }
      continue;
    }

    // Single-line KEY=value
    const value = afterEq.trim();
    if (!validateName(name, i + 1, errors)) { i++; continue; }
    if (value === '') {
      errors.push(`Line ${i + 1}: empty value for '${name}'`);
      i++;
      continue;
    }

    secrets.push({ name, value });
    i++;
  }

  return { secrets, errors };
}
