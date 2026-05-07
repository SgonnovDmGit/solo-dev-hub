#!/usr/bin/env node
/**
 * Extract a version section from Changelog.md.
 * Usage: node scripts/extract-changelog.mjs <version>
 * Example: node scripts/extract-changelog.mjs 0.15.0
 * Prints the section body (without the "## [X.Y.Z] — DATE" header) to stdout.
 */
import { readFileSync } from 'node:fs';

const version = process.argv[2];
if (!version) {
  console.error('Usage: extract-changelog.mjs <version>');
  process.exit(1);
}

const content = readFileSync('Changelog.md', 'utf8');
const escaped = version.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
// Matches the section header "## [X.Y.Z]" (optionally followed by date) up to the next "## [" or EOF.
const re = new RegExp(
  `^## \\[${escaped}\\][^\\n]*\\n([\\s\\S]*?)(?=^## \\[|\\Z)`,
  'm'
);
const match = content.match(re);
if (!match) {
  console.error(`Section ## [${version}] not found in Changelog.md`);
  process.exit(1);
}
process.stdout.write(match[1].trim() + '\n');
