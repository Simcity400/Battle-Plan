const joins = new Map<string, (pages: readonly unknown[]) => unknown>();

export function pageJoin(kind: string): ((pages: readonly unknown[]) => unknown) | undefined {
  return joins.get(kind);
}

