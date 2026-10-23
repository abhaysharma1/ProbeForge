function isControlCode(code: number): boolean {
  return code <= 0x1f || code === 0x7f;
}

/** Rejects ASCII control characters, including NUL, CR, and LF. */
export function containsControlCharacter(value: string): boolean {
  for (const character of value) {
    const code = character.codePointAt(0);
    if (code !== undefined && isControlCode(code)) return true;
  }
  return false;
}

/** Rejects header-injection characters while allowing ordinary punctuation. */
export function containsHeaderControl(value: string): boolean {
  return value.includes("\0") || value.includes("\r") || value.includes("\n");
}

/** Replaces control characters with spaces, preserving tab, CR, and LF. */
export function stripUnsafeControls(value: string): string {
  let result = "";
  for (const character of value) {
    const code = character.codePointAt(0);
    if (code === undefined) continue;
    if (code === 0x09 || code === 0x0a || code === 0x0d) {
      result += character;
    } else {
      result += isControlCode(code) ? " " : character;
    }
  }
  return result;
}
