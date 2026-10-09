import type { ChordData, ChordFlow, ChordNode } from './layout.ts';
import { checkChord } from './check.ts';

export interface FoldedChord { data: ChordData; nodes: Map<string, readonly string[]>; flows: Map<string, readonly string[]> }

/** The widget chooses which nodes to keep and supplies the aggregate's identity,
 * label and colour. Everything else folds, including flows inside the aggregate.
 * Membership is returned for exact tooltip totals and constituent read dates. */
export function foldChord(data: ChordData, keep: ReadonlySet<string>, other: ChordNode): FoldedChord {
  checkChord(data);
  if (data.nodes.some(node => node.id === other.id)) throw new Error('Fold node identity collides with a node');
  const nodes = new Map<string, string[]>(), flows = new Map<string, string[]>();
  const shown = data.nodes.filter(node => keep.has(node.id));
  const folded = data.nodes.filter(node => !keep.has(node.id));
  for (const node of shown) nodes.set(node.id, [node.id]);
  if (folded.length) { shown.push(other); nodes.set(other.id, folded.map(node => node.id)); }
  const grouped = new Map<string, Map<string, ChordFlow>>();
  const result: ChordFlow[] = [];
  for (const flow of data.flows) {
    const source = keep.has(flow.source) ? flow.source : other.id, target = keep.has(flow.target) ? flow.target : other.id;
    let targets = grouped.get(source);
    if (!targets) { targets = new Map(); grouped.set(source, targets); }
    let aggregate = targets.get(target);
    if (!aggregate) {
      // JSON tuples preserve opaque identities, including delimiters and spaces.
      aggregate = { id: JSON.stringify([source, target]), source, target, value: 0 };
      targets.set(target, aggregate); result.push(aggregate); flows.set(aggregate.id, []);
    }
    aggregate.value += flow.value;
    if (!Number.isFinite(aggregate.value)) throw new Error('Folded chord total exceeds numeric geometry range');
    flows.get(aggregate.id)!.push(flow.id);
  }
  return { data: { nodes: shown, flows: result }, nodes, flows };
}
