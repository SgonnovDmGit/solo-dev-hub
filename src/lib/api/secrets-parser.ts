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

    // Single-line KEY=value (dotenv-compatible: strips quotes, decodes escapes
    // in double-quoted values, strips inline `# comment` after value).
    if (!validateName(name, i + 1, errors)) { i++; continue; }
    const value = parseSingleLineValue(afterEq, i + 1, errors);
    if (value === null) { i++; continue; }
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

/// v0.30.0: dotenv-style single-line value parser.
/// Returns the parsed value, or `null` if parsing failed (errors pushed).
///
/// - `KEY=bar` → "bar"
/// - `KEY="bar baz"` → "bar baz" (surrounding quotes stripped)
/// - `KEY="line1\nline2"` → "line1\nline2" (escape decoded inside double-quotes)
/// - `KEY='no\nescape'` → "no\\nescape" (single quotes: literal, no escape decode)
/// - `KEY=22 # comment` → "22" (inline comment, must be preceded by whitespace)
/// - `KEY="value" # comment` → "value" (inline comment after closing quote)
/// - `KEY=v1.0#abc` → "v1.0#abc" (no whitespace before #, kept as literal)
/// - `KEY="url#frag"` → "url#frag" (inside quotes, kept as literal)
function parseSingleLineValue(
  input: string,
  lineNo: number,
  errors: string[]
): string | null {
  const s = input.trimStart();
  if (s === '') return '';

  const first = s[0];

  if (first === '"' || first === "'") {
    const quote = first;
    const decodeEscapes = quote === '"';
    let value = '';
    let i = 1;
    while (i < s.length) {
      const c = s[i];
      if (c === quote) {
        // Closing quote — rest of line must be whitespace and/or a comment.
        const rest = s.substring(i + 1).trimStart();
        if (rest !== '' && !rest.startsWith('#')) {
          errors.push(`Line ${lineNo}: unexpected content after closing quote`);
          return null;
        }
        return value;
      }
      if (decodeEscapes && c === '\\' && i + 1 < s.length) {
        const next = s[i + 1];
        switch (next) {
          case 'n': value += '\n'; i += 2; continue;
          case 'r': value += '\r'; i += 2; continue;
          case 't': value += '\t'; i += 2; continue;
          case '\\': value += '\\'; i += 2; continue;
          case '"': value += '"'; i += 2; continue;
          default: value += c; i += 1; continue;
        }
      }
      value += c;
      i += 1;
    }
    errors.push(`Line ${lineNo}: unclosed quote`);
    return null;
  }

  // Unquoted value — strip inline `# comment` (must be preceded by whitespace).
  // `value#tag` stays intact (no space before #), `value  # tag` becomes `value`.
  let endIdx = s.length;
  for (let i = 1; i < s.length; i++) {
    if (s[i] === '#' && (s[i - 1] === ' ' || s[i - 1] === '\t')) {
      endIdx = i;
      break;
    }
  }
  return s.substring(0, endIdx).trimEnd();
}
