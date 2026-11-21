import { countryKey } from '../../dashboard/countryKey.ts';
import type { ChartSummary } from '../../charts/chartSummary';
import type { LiveShares, ShareRun } from '../../charts/share';
import type { TiledLine } from '../../dashboard/tiles/view';
import { formatFixed, rescale } from '../../charts/valueAxis.ts';
import { usNumber } from '../../charts/numberFormat.ts';
import type { TooltipContent } from '../../charts/tooltip';
import type { MissingSpan, SeriesState } from '../../generated/dashboard';

export const STATES = ['noSupply', 'strained', 'inSupply'] as const;
export type SupplyState = (typeof STATES)[number];
export const STATE_LABELS = { noSupply: 'No supply', strained: 'Strained', inSupply: 'In supply' } as const;
export const SHARE_MEASURES = ['noSupplyShare', 'strainedShare', 'inSupplyShare'] as const;
export const SUPPLY_FAMILY = 'supply-health';
export const DIVISIONS = 'divisions';

export type SupplyCounts = Readonly<Record<SupplyState, number>>;
export type SupplyAt = { counts: SupplyCounts } | { missing: string } | undefined;

const SHARE_UNITS = 1_000_000_000;

export const totalDivisions = (counts: SupplyCounts) => STATES.reduce((sum, state) => sum + counts[state], 0);
export const formatShare = (count: number, total: number) =>
  total ? `${formatFixed(rescale(BigInt(count) * 100n, total, 2), 2, 2)}%` : 'Not known';

export function shareRunsFromLines(lines: readonly (ChartSummary | undefined)[], head: number | null = null,
  missing: readonly MissingSpan[] = []): ShareRun[] {
  const [first, ...rest] = lines;
  if (!first || rest.length !== STATES.length - 1 || rest.some(line => !line)) return [];
  const others = rest.map(line => new Map(line!.buckets.flatMap(bucket => bucket ? [[bucket.firstHour, bucket] as const] : [])));
  const cut = head !== null && first.tiles ? Math.floor(head / first.tiles.binHours) * first.tiles.binHours : Infinity;
  const runs: ShareRun[] = [];
  for (const bucket of first.buckets) {
    if (!bucket || bucket.firstHour >= cut) continue;
    const matched = others.map(other => other.get(bucket.firstHour));
    if (bucket.hasBreak || matched.some(other => !other || other.lastHour !== bucket.lastHour || other.hasBreak)) continue;
    const counts = [bucket, ...matched].map(item => Math.max(0, Math.round(item!.firstValue * SHARE_UNITS)));
    const breakBefore = bucket.firstBreak || matched.some(other => other!.firstBreak);
    const previous = runs.at(-1);
    if (previous && !breakBefore && previous.lastHour + 1 === bucket.firstHour && previous.counts.every((count, at) => count === counts[at])) {
      runs[runs.length - 1] = { ...previous, lastHour: bucket.lastHour };
    } else runs.push({ firstHour: bucket.firstHour, lastHour: bucket.lastHour, startsSegment: breakBefore, breakBefore, counts });
  }
  const last = head === null ? 0 : Math.floor(head);
  const from = missing.reduce((from, span) => span.firstHour <= last && span.lastHour >= from ? Math.max(from, span.lastHour + 1) : from, cut);
  if (cut !== Infinity && from <= last) {
    const values = lines.map(line => line!.tiles?.valueAt(head!) ?? null);
    if (values.every(value => value !== null && Number.isFinite(value))) {
      const previous = runs.at(-1);
      const breakBefore = !previous || previous.lastHour + 1 !== from;
      runs.push({ firstHour: from, lastHour: last, startsSegment: breakBefore, breakBefore,
        counts: values.map(value => Math.max(0, Math.round(value! * SHARE_UNITS))) });
    }
  }
  return runs;
}

type Live<T> = { read(): T; subscribe(listener: () => void): () => void };

export function liveShareRuns(tag: string, clip: (runs: ShareRun[]) => ShareRun[], lines: readonly Live<ReadonlyMap<string, TiledLine> | null>[],
  answers: Live<readonly unknown[]>): LiveShares {
  let seen: { held: unknown[]; head: number | null; runs: readonly ShareRun[] } | null = null;
  const held = (): unknown[] => [...lines.map(line => line.read()), ...answers.read()];
  return {
    read: head => {
      const now = held();
      if (seen && seen.head === head && seen.held.length === now.length && seen.held.every((value, at) => value === now[at])) return seen.runs;
      const shares = now.slice(0, lines.length) as (ReadonlyMap<string, TiledLine> | null)[];
      const runs = clip(shareRunsFromLines(shares.map(line => line?.get(tag)?.summary), head, shares[0]?.get(tag)?.missing));
      seen = { held: now, head, runs };
      return runs;
    },
    revision: held,
    subscribe: listener => { const stops = [...lines, answers].map(source => source.subscribe(listener)); return () => stops.forEach(stop => stop()); },
  };
}

export function mostDivisions(line: ChartSummary | undefined): number | null {
  let most: number | null = null;
  for (const bucket of line?.buckets ?? []) if (bucket) most = Math.max(most ?? 0, bucket.maxValue);
  return most;
}

export function supplyAt(replies: readonly (readonly SeriesState[])[] | undefined, tag: string, hour: number): SupplyAt {
  if (!replies || replies.length !== STATES.length) return undefined;
  const found = replies.map(states => states.find(state => countryKey(state.country) === tag && state.hour === hour));
  if (found.some(state => !state)) return undefined;
  const missing = found.find(state => !state!.run);
  if (missing) return { missing: missing!.missing ?? 'Not recorded at this hour' };
  return { counts: Object.fromEntries(STATES.map((state, at) => [state, Number(found[at]!.run!.valueRaw)])) as SupplyCounts };
}

export function supplyTooltip(date: string, name: string, countryFlag: TooltipContent['countryFlag'], at: SupplyAt,
  colors: readonly string[]): TooltipContent {
  const title = `${date} · ${name}`;
  if (!at) return { title, countryFlag, rows: [{ label: 'Supply health', value: 'Loading…', tone: 'muted' }] };
  if ('missing' in at) return { title, countryFlag, rows: [{ label: 'Supply health', value: 'Not known', detail: at.missing, tone: 'muted' }],
    note: 'Supply was not recorded at this time.' };
  const total = totalDivisions(at.counts);
  if (!total) return { title, countryFlag, rows: [{ label: 'Divisions', value: '0' }], note: 'No divisions.' };
  return { title, countryFlag,
    rows: [...STATES].reverse().map(state => ({ label: STATE_LABELS[state], swatch: colors[STATES.indexOf(state)],
      value: `${formatShare(at.counts[state], total)} · ${usNumber.format(at.counts[state])} ${at.counts[state] === 1 ? 'division' : 'divisions'}` })) };
}

export function supplyParts(counts: SupplyCounts, colors: readonly string[]) {
  const total = totalDivisions(counts);
  return STATES.map((state, index) => ({ id: state, label: STATE_LABELS[state], value: counts[state], color: colors[index],
    text: usNumber.format(counts[state]), shareText: formatShare(counts[state], total) }));
}
