import sodium from 'libsodium-wrappers';

/**
 * Normalize a secret value before encryption.
 *
 * - Strips `\r` (Windows CRLF → LF). Classic trap when pasting keys from Windows clipboard:
 *   the textarea accepts `\r\n` silently, but appleboy/ssh-action writes the key verbatim
 *   to a temp file and OpenSSH rejects the mixed-lineendings key with
 *   "unable to authenticate, attempted methods [none publickey]".
 * - For single-line values: trim surrounding whitespace (paths, tokens, IDs).
 * - For multi-line values: preserve body, strip surrounding blank lines/spaces,
 *   ensure exactly one trailing `\n` (PEM-friendly).
 */
export function normalizeSecretValue(raw: string): string {
  // Decide single vs multi-line by presence of real `\n` in the ORIGINAL input.
  // A lone trailing `\r` (single-line clipboard paste from Windows) must NOT
  // promote the value to multi-line and gain a trailing newline.
  const isMultiLine = raw.includes('\n');
  if (!isMultiLine) return raw.replace(/\r/g, '').trim();
  const normalized = raw.replace(/\r\n/g, '\n').replace(/\r/g, '');
  const trimmed = normalized.replace(/^[\s\n]+/, '').replace(/[\s\n]+$/, '');
  return trimmed + '\n';
}

export async function encryptSecret(publicKeyBase64: string, secretValue: string): Promise<string> {
  await sodium.ready;
  const publicKey = sodium.from_base64(publicKeyBase64, sodium.base64_variants.ORIGINAL);
  const messageBytes = sodium.from_string(normalizeSecretValue(secretValue));
  const encrypted = sodium.crypto_box_seal(messageBytes, publicKey);
  return sodium.to_base64(encrypted, sodium.base64_variants.ORIGINAL);
}
