/** Each prepared reply's digest of its data, as the host sent it: two replies with one digest
 * show the same, compared without reading either. Kept by the data object and by whatever a
 * reader makes of it. */
const digests = new WeakMap<object, string>();

/** The digest of `data`, when it or what it was made from came with one. */
export function replyDigest(data: unknown): string | undefined {
  return data !== null && typeof data === 'object' ? digests.get(data) : undefined;
}

/** Gives `made` the digest `from` carries (a string is the digest itself); returns `made`. */
export function keepDigest<T>(from: unknown, made: T): T {
  const digest = typeof from === 'string' ? from : replyDigest(from);
  if (digest !== undefined && made !== null && typeof made === 'object') digests.set(made, digest);
  return made;
}
