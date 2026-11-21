export function compareTags(a: string, b: string): number {
  if (a === b) return 0;
  const left = a[Symbol.iterator]();
  const right = b[Symbol.iterator]();
  for (;;) {
    const x = left.next();
    const y = right.next();
    if (x.done || y.done) return x.done && y.done ? 0 : x.done ? -1 : 1;
    const difference = x.value.codePointAt(0)! - y.value.codePointAt(0)!;
    if (difference !== 0) return difference < 0 ? -1 : 1;
  }
}
