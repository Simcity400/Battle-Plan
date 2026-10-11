import { dayLabel } from '../../charts/draw.ts';
import { foldChord } from '../../charts/chord/fold.ts';
import type { FoldedChord } from '../../charts/chord/fold';
import type { ChordHit } from '../../charts/chord/layout';
import { chordTotals } from '../../charts/chord/totals.ts';
import type { TooltipContent } from '../../charts/tooltip';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import { compareTags } from '../../dashboard/tagOrder.ts';
import { pairAmount } from './casualtyPairsData.ts';
import type { PairFlow, PairSnapshot } from './casualtyPairsData';

const nodeId = (tag: string) => JSON.stringify([tag]);
export const OTHER_ID = JSON.stringify([]);
export interface CasualtyChord extends FoldedChord {
  exact: ReadonlyMap<string, PairFlow>;
  countries: ReadonlyMap<string, string>;
  labels: CountryLabel;
  hour: number;
}

/** Only geometry converts to Number. Signed corrections remain exact in the
 * detail table; they do not turn into positive casualty ribbons. */
export function casualtyChord(snapshot: PairSnapshot, most: number, hour: number,
  labels: CountryLabel, color: (tag: string) => string, muted: string): CasualtyChord {
  const totals = chordTotals(snapshot.flows, flow => flow.from, flow => flow.to,
    flow => flow.value > 0n ? flow.value : 0n);
  const total = (tag: string) => { const value = totals.get(tag); return value ? value.incoming + value.outgoing : 0n; };
  const ranked = [...snapshot.countries].sort((a, b) => total(a) === total(b) ? compareTags(a, b) : total(a) > total(b) ? -1 : 1);
  const countries = new Map(snapshot.countries.map(tag => [nodeId(tag), tag]));
  const data = { nodes: [...countries].map(([id, tag]) => {
    const label = labels(tag);
    return { id, label: label.name, flag: label.flag, countryFlag: label, color: color(tag) };
  }),
    flows: snapshot.flows.map(flow => ({ id: flow.id, source: nodeId(flow.from), target: nodeId(flow.to),
      value: flow.value > 0n ? Number(flow.value) : 0 })) };
  const folded = foldChord(data, new Set(ranked.slice(0, most).map(nodeId)), { id: OTHER_ID, label: 'Other countries', color: muted });
  return { ...folded, exact: new Map(snapshot.flows.map(flow => [flow.id, flow])), countries, labels, hour };
}

export function chordPairs(hit: ChordHit, chord: CasualtyChord): PairFlow[] {
  if (hit.kind === 'flow') return (chord.flows.get(hit.id) ?? []).map(id => chord.exact.get(id)!);
  const members = new Set((chord.nodes.get(hit.id) ?? []).map(id => chord.countries.get(id)!));
  return [...chord.exact.values()].filter(flow => members.has(flow.from) || members.has(flow.to));
}

export function pairTooltip(flows: readonly PairFlow[], labels: CountryLabel, hour: number): TooltipContent {
  return { title: 'Casualties dealt', subtitle: dayLabel(hour),
    rows: flows.map(flow => {
      const from = labels(flow.from);
      return { label: `${from.name} → ${labels(flow.to).name}`,
        value: pairAmount(flow), icon: from.flag, countryFlag: from };
    }) };
}
