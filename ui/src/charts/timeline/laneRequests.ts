/** Lanes whose country facts a timeline needs: every lane's name, since its orders
 * compare names (as the order itself or to break ties), and flags only for the lanes
 * in view and its margin. */
export function laneRequests<L>(lanes: readonly L[], inView: readonly [number, number], margin = 8): { names: readonly L[]; looks: readonly L[] } {
  return { names: lanes, looks: lanes.slice(Math.max(0, inView[0] - margin), inView[1] + 1 + margin) };
}
