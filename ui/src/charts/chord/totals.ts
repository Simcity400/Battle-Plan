export function chordTotals<T>(flows: readonly T[], source: (flow: T) => string,
  target: (flow: T) => string, value: (flow: T) => bigint): Map<string, { incoming: bigint; outgoing: bigint }> {
  const totals = new Map<string, { incoming: bigint; outgoing: bigint }>();
  const at = (id: string) => {
    let total = totals.get(id);
    if (!total) { total = { incoming: 0n, outgoing: 0n }; totals.set(id, total); }
    return total;
  };
  for (const flow of flows) {
    const raw = value(flow);
    at(source(flow)).outgoing += raw;
    at(target(flow)).incoming += raw;
  }
  return totals;
}
