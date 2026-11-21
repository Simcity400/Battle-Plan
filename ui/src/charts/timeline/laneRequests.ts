export function laneRequests<L>(lanes: readonly L[], inView: readonly [number, number], margin = 8): { names: readonly L[]; looks: readonly L[] } {
  return { names: lanes, looks: lanes.slice(Math.max(0, inView[0] - margin), inView[1] + 1 + margin) };
}
