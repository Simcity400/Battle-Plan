import { compareTags } from '../../dashboard/tagOrder.ts';
import type { FlowSide } from './layout';

export const OTHER = '\u0000other';

export interface SummedNode {
  id: string; side: FlowSide; tag: string | null; members: string[];
  memberValues: Map<string, number>; value: number;
}

export interface FlowPart<T> { part: T; source: string; target: string; value: number; group?: string | number }
export interface SummedLink<T> { id: string; source: string; target: string; parts: T[]; value: number }
export interface SummedDiagram<T> { nodes: SummedNode[]; links: SummedLink<T>[]; total: number }

const nodeId = (side: FlowSide, tag: string | null) => `${side === 'source' ? 's' : 't'}:${tag == null ? OTHER
  : tag.startsWith('\u0000') ? `\u0000country:${JSON.stringify(tag)}` : tag}`;

export function sumFlowParts<T>(parts: readonly FlowPart<T>[], perSide: number): SummedDiagram<T> {
  const valued = parts.filter(item => item.value > 0);
  const totals = (side: FlowSide) => {
    const sums = new Map<string, number>();
    for (const item of valued) sums.set(item[side], (sums.get(item[side]) ?? 0) + item.value);
    return sums;
  };
  const kept = (sums: Map<string, number>) => {
    const ranked = [...sums].sort((a, b) => b[1] - a[1] || compareTags(a[0], b[0]));
    const limit = ranked.length <= perSide + 1 ? ranked.length : Math.max(1, perSide);
    return new Set(ranked.slice(0, limit).map(([tag]) => tag));
  };
  const sources = kept(totals('source'));
  const targets = kept(totals('target'));
  const nodes = new Map<string, SummedNode>();
  const links = new Map<string, SummedLink<T>>();
  const node = (side: FlowSide, tag: string, value: number, keep: ReadonlySet<string>) => {
    const own = keep.has(tag) ? tag : null;
    const id = nodeId(side, own);
    const entry = nodes.get(id) ?? { id, side, tag: own, members: [], memberValues: new Map<string, number>(), value: 0 };
    entry.memberValues.set(tag, (entry.memberValues.get(tag) ?? 0) + value);
    entry.value += value;
    nodes.set(id, entry);
    return id;
  };
  let total = 0;
  for (const item of valued) {
    total += item.value;
    const source = node('source', item.source, item.value, sources);
    const target = node('target', item.target, item.value, targets);
    const id = JSON.stringify([source, target, item.group ?? null]);
    const link = links.get(id) ?? { id, source, target, parts: [], value: 0 };
    link.parts.push(item.part);
    link.value += item.value;
    links.set(id, link);
  }
  for (const entry of nodes.values()) entry.members = [...entry.memberValues]
    .sort((a, b) => b[1] - a[1] || compareTags(a[0], b[0])).map(([tag]) => tag);
  return { nodes: [...nodes.values()], links: [...links.values()], total };
}
