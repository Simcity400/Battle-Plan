/** Country tags in the order the data service sorts them: by their UTF-8 bytes, as
 * Rust orders `String`s (a `BTreeMap` of tags), which is code point order. Neither
 * `localeCompare` (which puts `aaa` before `AAA`) nor JavaScript's own `<` (UTF-16
 * code units, which put an astral character before U+E000–U+FFFF) agrees with it,
 * and a tie broken otherwise than the product breaks it can change who is ranked
 * first when N changes. */
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
