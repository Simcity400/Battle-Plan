const joins = new Map<string, (pages: readonly unknown[]) => unknown>();

export function registerPageJoin(kinds: readonly string[], join: (pages: readonly unknown[]) => unknown): void {
  for (const kind of kinds) joins.set(kind, join);
}

export function pageJoin(kind: string): ((pages: readonly unknown[]) => unknown) | undefined {
  return joins.get(kind);
}

export function pagePreparation(page: unknown): Record<string, unknown> {
  const fields = page as Record<string, unknown>;
  return Object.fromEntries(['coverage', 'preparedCoverage', 'pending', 'approximate'].filter(name => name in fields).map(name => [name, fields[name]]));
}

export function countryPageJoin<R extends { summaries?: readonly { tag: string }[] }>(merge: (replies: readonly R[]) => object | undefined) {
  return (pages: readonly unknown[]): unknown => {
    const replies = pages as readonly R[];
    const seen = new Set<string>();
    return { ...merge(replies), ...pagePreparation(replies[replies.length - 1]), cursor: null,
      summaries: replies.flatMap(reply => reply.summaries ?? []).filter(line => !seen.has(line.tag) && !!seen.add(line.tag)) };
  };
}
