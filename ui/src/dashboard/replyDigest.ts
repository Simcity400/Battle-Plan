const digests = new WeakMap<object, string>();

export function replyDigest(data: unknown): string | undefined {
  return data !== null && typeof data === 'object' ? digests.get(data) : undefined;
}

export function keepDigest<T>(from: unknown, made: T): T {
  const digest = typeof from === 'string' ? from : replyDigest(from);
  if (digest !== undefined && made !== null && typeof made === 'object') digests.set(made, digest);
  return made;
}
