import type { ChordData } from './layout.ts';

/** Validate exact identities and numeric geometry before motion changes. */
export function checkChord(data: ChordData): number {
  const nodes = new Set<string>(), flows = new Set<string>();
  for (const node of data.nodes) {
    if (nodes.has(node.id)) throw new Error(`Duplicate chord node: ${node.id}`);
    nodes.add(node.id);
  }
  let total = 0;
  for (const flow of data.flows) {
    if (flows.has(flow.id)) throw new Error(`Duplicate chord flow: ${flow.id}`);
    flows.add(flow.id);
    if (!nodes.has(flow.source) || !nodes.has(flow.target)) throw new Error(`Unknown chord endpoint: ${flow.id}`);
    if (!Number.isFinite(flow.value) || flow.value < 0) throw new Error(`Invalid chord value: ${flow.id}`);
    total += flow.value * 2;
  }
  if (!Number.isFinite(total)) throw new Error('Chord total exceeds numeric geometry range');
  return total;
}
