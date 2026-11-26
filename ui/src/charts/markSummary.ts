export interface MarkCluster {
  firstHour: number;
  lastHour: number;
  firstOrdinal: string;
  count: string;
  bins?: readonly (readonly [number, number])[];
}
export interface MarkSummary {
  from: number;
  to: number;
  width: number;
  clusters: readonly MarkCluster[];
  tiles?: true;
}
