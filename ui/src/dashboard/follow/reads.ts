const SEPARATOR = '\u0005';
const held = new WeakMap<object, string>();

export function supplyingReads(reply: unknown): string {
  if (!reply || typeof reply !== 'object') return '';
  const saved = held.get(reply);
  if (saved !== undefined) return saved;
  const reads: unknown[] = [];
  const stack: object[] = [reply];
  while (stack.length) {
    const item = stack.pop()!;
    if (Array.isArray(item)) {
      for (let at = item.length - 1; at >= 0; at--) { const value: unknown = item[at]; if (value && typeof value === 'object') stack.push(value); }
      continue;
    }
    const keys = Object.keys(item);
    for (let at = keys.length - 1; at >= 0; at--) {
      const key = keys[at]!, value = (item as Record<string, unknown>)[key];
      if (key === 'readHour') reads.push(value ?? null);
      else if (value && typeof value === 'object') stack.push(value);
    }
  }
  const key = reads.length ? JSON.stringify(reads) : '';
  held.set(reply, key);
  return key;
}

export const followShape = (reply: unknown, structure: string) => supplyingReads(reply) + SEPARATOR + structure;

export function shapeWithin(reply: unknown, shape: string): string | null {
  const at = shape.indexOf(SEPARATOR);
  return at >= 0 && shape.slice(0, at) === supplyingReads(reply) ? shape.slice(at + 1) : null;
}
