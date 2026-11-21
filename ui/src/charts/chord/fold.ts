import type { ChordData, ChordFlow, ChordNode } from './layout.ts';
import { checkChord } from './check.ts';

export interface FoldedChord { readonly data: ChordData; readonly nodes: ReadonlyMap<string, readonly string[]>; readonly flows: ReadonlyMap<string, readonly string[]> }

export function foldChord(data: ChordData, keep: ReadonlySet<string>, other: ChordNode): FoldedChord {
  checkChord(data);
  if (data.nodes.some(node => node.id === other.id)) throw new Error('Fold node identity collides with a node');
  const nodes = new Map<string, string[]>(), flows = new Map<string, string[]>();
  const shown = data.nodes.filter(node => keep.has(node.id));
  const folded = data.nodes.filter(node => !keep.has(node.id));
  for (const node of shown) nodes.set(node.id, [node.id]);
  if (folded.length) { shown.push(other); nodes.set(other.id, folded.map(node => node.id)); }
  const grouped = new Map<string, Map<string, number>>();
  const result: Omit<ChordFlow, 'value'>[] = [], values: number[] = [];
  for (const flow of data.flows) {
    const source = keep.has(flow.source) ? flow.source : other.id, target = keep.has(flow.target) ? flow.target : other.id;
    let targets = grouped.get(source);
    if (!targets) { targets = new Map(); grouped.set(source, targets); }
    let index = targets.get(target);
    if (index === undefined) {
      index = result.length;
      const aggregate = { id: JSON.stringify([source, target]), source, target };
      targets.set(target, index); result.push(aggregate); values.push(0); flows.set(aggregate.id, []);
    }
    values[index] += flow.value;
    if (!Number.isFinite(values[index])) throw new Error('Folded chord total exceeds numeric geometry range');
    flows.get(result[index].id)!.push(flow.id);
  }
  return { data: { nodes: shown, flows: result.map((flow, index) => ({ ...flow, value: values[index] })) }, nodes, flows };
}
