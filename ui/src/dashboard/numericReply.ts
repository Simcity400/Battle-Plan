function sameNumbers(a: unknown, b: unknown): boolean {
  if (Object.is(a, b)) return true;
  if (!a || !b || typeof a !== 'object' || typeof b !== 'object') return false;
  if (Array.isArray(a)) return Array.isArray(b) && a.length === b.length && a.every((value, index) => sameNumbers(value, b[index]));
  if (Array.isArray(b)) return false;
  const left = a as Record<string, unknown>, right = b as Record<string, unknown>;
  const keys = Object.keys(left);
  return keys.length === Object.keys(right).length && keys.every(key => Object.hasOwn(right, key) && sameNumbers(left[key], right[key]));
}

export class NumericReply<T> {
  private key: string | undefined;
  private reply: T | undefined;
  private numbers: T | undefined;

  update(key: string, reply: T | undefined, project: (reply: T) => T): T | undefined {
    if (this.key !== key) { this.key = key; this.reply = undefined; this.numbers = undefined; }
    if (reply !== undefined && reply !== this.reply) {
      const numbers = project(reply);
      if (this.numbers === undefined || !sameNumbers(this.numbers, numbers)) this.numbers = numbers;
      this.reply = reply;
    }
    return this.numbers;
  }
}
