import type { GatedTime } from '../../dashboard/regionTime';
import type { PinnedSpec } from '../../charts/drill/drillState';
import { pinnedBreakdown } from '../../charts/drill/drillState.ts';
import { addStackAmount, forEachCompactStackSegment, stackEdges } from '../../charts/stack.ts';
import type { CompactStackModel, StackRun, StackSegment, StackSum } from '../../charts/stack';
import { contentKey, setToken } from '../../dashboard/options.ts';
import type { OptionValue } from '../../dashboard/options';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import type { HourSpan } from '../../generated/dashboard';
import type { CountryIdentityIndex } from '../../dashboard/countryIdentity';
import { flowPrefix, flowValue, knownHours, knownSpanAt, measureAmount } from './tradeData.ts';
import type { Flow, Measure, SideNode } from './tradeData';

export const TRADE_BREAKDOWN = { id: 'contributions', label: 'Trade contributors' };
export const COUNTRY_BREAKDOWN = { id: 'contributions', label: 'Country trade' };

export type TradeSelection =
  | { kind: 'all' }
  | { kind: 'node'; side: 'source' | 'target'; holders: readonly string[] }
  | { kind: 'link'; sources: readonly string[]; targets: readonly string[]; resource: number }
  | { kind: 'country'; holder: string }
  | { kind: 'market'; key: string };

export function contributionCountryTag(index: CountryIdentityIndex | null, tag: string, selection: TradeSelection | null, hour: number | null): string | null {
  if (!index || hour === null || !selection || selection.kind === 'market') return null;
  let keys = index.selected([tag]);
  if (selection.kind !== 'all') {
    const cohort = new Set(selection.kind === 'node' ? selection.holders : selection.kind === 'country' ? [selection.holder] : [...selection.sources, ...selection.targets]);
    const selected = new Set([...keys].filter(key => cohort.has(key)));
    if (selected.size) keys = selected;
  }
  if (keys.size !== 1) return null;
  const aliases = index.aliasesAt(keys, hour);
  return aliases.length === 1 ? aliases[0] : null;
}

const holders = (value: unknown): value is string[] => Array.isArray(value)
  && value.length > 0 && value.every(item => typeof item === 'string' && item.length > 0)
  && new Set(value).size === value.length;

export interface TradeContext { campaignKey: string; branchId: string }
export function tradeContext(track: Pick<BoundTrack, 'source' | 'branch'>): TradeContext {
  return { campaignKey: track.source.metadata.campaignKey, branchId: track.branch.id };
}
export function selectionToken(selection: TradeSelection, context: TradeContext): string { return setToken(JSON.stringify({ ...context, selection })); }

export function readSelection(value: OptionValue | undefined, context: TradeContext): TradeSelection | null {
  if (typeof value !== 'string' || !value) return null;
  try {
    const stored = JSON.parse(contentKey(value));
    if (stored?.campaignKey !== context.campaignKey || stored?.branchId !== context.branchId) return null;
    const item = stored.selection;
    if (!item || typeof item !== 'object') return null;
    if (item.kind === 'all') return item;
    if (item.kind === 'market' && typeof item.key === 'string' && /^(offer|contract|agreement):.+$/.test(item.key)) return { kind: 'market', key: item.key };
    if (item.kind === 'node' && (item.side === 'source' || item.side === 'target') && holders(item.holders)) return item;
    if (item.kind === 'country' && holders([item.holder])) return { kind: 'country', holder: item.holder };
    if (item.kind === 'link' && holders(item.sources) && holders(item.targets) && Number.isSafeInteger(item.resource) && item.resource >= 0) return item;
  } catch {   }
  return null;
}

export function contributionExtent(value: OptionValue | undefined, branch: Pick<BoundTrack['branch'], 'firstHour'>): [number, number] | null {
  if (typeof value !== 'string' || !value) return null;
  try {
    const offsets = JSON.parse(contentKey(value));
    if (Array.isArray(offsets) && offsets.length === 2 && offsets.every(Number.isSafeInteger) && offsets[0] <= offsets[1]) {
      return [branch.firstHour + offsets[0], branch.firstHour + offsets[1]];
    }
  } catch {   }
  return null;
}

export function nodeSelections(flows: readonly Flow[], nodes: readonly SideNode[]): ReadonlyMap<string, TradeSelection> {
  const all = { source: new Set<string>(), target: new Set<string>() };
  for (const flow of flows) { all.source.add(flow.sellerIdentity); all.target.add(flow.buyerIdentity); }
  const named = { source: new Set<string>(), target: new Set<string>() };
  for (const node of nodes) if (node.identity) named[node.side].add(node.identity);
  const other = {
    source: [...all.source].filter(identity => !named.source.has(identity)).sort(),
    target: [...all.target].filter(identity => !named.target.has(identity)).sort(),
  };
  return new Map(nodes.map(node => [node.id, { kind: 'node', side: node.side,
    holders: node.identity ? [node.identity] : other[node.side] } as TradeSelection]));
}

export function linkSelection(source: TradeSelection, target: TradeSelection, resource: number): TradeSelection {
  if (source.kind !== 'node' || target.kind !== 'node') throw new Error('Trade ribbon endpoints must be nodes.');
  return { kind: 'link', sources: source.holders, targets: target.holders, resource };
}

export function selectedContributors(flows: readonly Flow[], selection: TradeSelection): Flow[] {
  if (selection.kind === 'all') return [...flows];
  if (selection.kind === 'market') return [];
  if (selection.kind === 'country') return flows.filter(flow => flow.sellerIdentity === selection.holder || flow.buyerIdentity === selection.holder);
  if (selection.kind === 'node') {
    const wanted = new Set(selection.holders);
    return flows.filter(flow => wanted.has(selection.side === 'source' ? flow.sellerIdentity : flow.buyerIdentity));
  }
  const sources = new Set(selection.sources);
  const targets = new Set(selection.targets);
  return flows.filter(flow => flow.resource.id === selection.resource && sources.has(flow.sellerIdentity) && targets.has(flow.buyerIdentity));
}

export function contributionAmounts(flows: readonly Flow[], measure: Measure, from: number, hour: number,
  known: readonly HourSpan[]): { flow: Flow; raw: bigint | null }[] {
  const missing = measure === 'current' ? knownSpanAt(known, hour) < 0
    : knownHours(known, from, hour) === 0;
  return flows.map(flow => ({ flow, raw: missing ? null : measureAmount([flow], measure, from, hour) }));
}

export function observedTradeSpans(known: readonly HourSpan[], unobserved: readonly HourSpan[] = []): HourSpan[] {
  const spans: HourSpan[] = [];
  let hole = 0;
  for (const span of known) {
    let first = span.firstHour;
    while (hole < unobserved.length && unobserved[hole].lastHour < first) hole++;
    for (let index = hole; index < unobserved.length && unobserved[index].firstHour <= span.lastHour; index++) {
      const gap = unobserved[index];
      if (gap.firstHour > first) spans.push({ firstHour: first, lastHour: gap.firstHour - 1 });
      first = Math.max(first, gap.lastHour + 1);
    }
    if (first <= span.lastHour) spans.push({ firstHour: first, lastHour: span.lastHour });
  }
  return spans;
}

export function contributionHistory(flow: Flow, known: readonly HourSpan[]): StackRun[] {
  const runs: StackRun[] = [];
  let at = 0;
  const push = (firstHour: number, lastHour: number, value: number) => {
    if (lastHour < firstHour) return;
    const last = runs.at(-1);
    if (last && last.lastHour + 1 === firstHour && last.value === value) last.lastHour = lastHour;
    else runs.push({ firstHour, lastHour, value });
  };
  for (const span of known) {
    let first = span.firstHour;
    while (at < flow.runs.length && flow.runs[at].lastHour < first) at++;
    for (let index = at; index < flow.runs.length && flow.runs[index].firstHour <= span.lastHour; index++) {
      const run = flow.runs[index];
      const start = Math.max(first, run.firstHour);
      const end = Math.min(span.lastHour, run.lastHour);
      push(first, start - 1, 0);
      push(start, end, Math.max(0, run.amount));
      first = end + 1;
    }
    push(first, span.lastHour, 0);
  }
  return runs;
}

export interface ContributionMeasureModel {
  flows: readonly Flow[]; measure: 'total' | 'average'; from: number; hours: number[];
  basis: readonly (readonly [number, number])[];
  guide: CompactStackModel; upper: (number | null)[];
  work: { prefixWalks: number; eventUpdates: number; groupEvaluations: number };
}
type ContributionCurve = { constant: number; coefficient: number; offset: number | null; peak: number };
const ZERO_CURVE: ContributionCurve = { constant: 0, coefficient: 0, offset: null, peak: 0 };
const scalar = (): StackSum => ({ sum: 0, error: 0 });

function contributionCurve(flow: Flow, index: number, measure: 'total' | 'average', basis: readonly [number, number], hour: number): ContributionCurve {
  if (index < 0) return ZERO_CURVE;
  const run = flow.runs[index];
  const priorAmount = flow.amountBefore[index] - basis[0];
  const priorHours = flow.hoursBefore[index] - basis[1];
  const duration = Math.min(hour, run.lastHour) - run.firstHour + 1;
  const elapsed = priorHours + duration;
  const numerator = priorAmount + run.amount * duration;
  if (elapsed <= 0) return ZERO_CURVE;
  if (hour >= run.lastHour) {
    const value = Math.max(0, measure === 'total' ? numerator / 24 : numerator / elapsed);
    return { constant: value, coefficient: 0, offset: null, peak: value };
  }
  const crossing = run.firstHour - 1 - priorAmount / run.amount;
  const positive = run.amount > 0 ? hour >= crossing : run.amount < 0 ? hour < crossing : numerator > 0;
  if (!positive) return ZERO_CURVE;
  const lastDuration = run.lastHour - run.firstHour + 1;
  const lastAmount = priorAmount + run.amount * lastDuration;
  const value = measure === 'total' ? numerator / 24 : numerator / elapsed;
  const last = measure === 'total' ? lastAmount / 24 : lastAmount / (priorHours + lastDuration);
  const peak = Math.max(0, value, last);
  return measure === 'total'
    ? { constant: (priorAmount + run.amount * (1 - run.firstHour)) / 24, coefficient: run.amount / 24, offset: null, peak }
    : { constant: run.amount, coefficient: priorAmount - run.amount * priorHours, offset: priorHours + 1 - run.firstHour, peak };
}

export function contributionMeasureModel(flows: readonly Flow[], measure: 'total' | 'average', from: number,
  known: readonly HourSpan[]): ContributionMeasureModel {
  const work = { prefixWalks: 0, eventUpdates: 0, groupEvaluations: 0 };
  const basis = flows.map(flow => { work.prefixWalks++; return flowPrefix(flow, from - 1); });
  const boundaries = new Set<number>([from]);
  if (measure === 'average' && knownSpanAt(known, from - 1) >= 0) boundaries.add(from - 1);
  const events = new Map<number, number[]>();
  const change = (hour: number, flow: number) => {
    boundaries.add(hour);
    const rows = events.get(hour) ?? [];
    rows.push(flow); events.set(hour, rows);
  };
  for (const span of known) { boundaries.add(span.firstHour); boundaries.add(span.lastHour); boundaries.add(span.lastHour + 1); }
  flows.forEach((flow, index) => {
    change(from, index);
    flow.runs.forEach((run, at) => {
      boundaries.add(run.firstHour); boundaries.add(run.lastHour); boundaries.add(run.lastHour + 1);
      if (run.firstHour > from) change(run.firstHour, index);
      if (run.lastHour > from && run.lastHour !== run.firstHour) change(run.lastHour, index);
      if (run.amount !== 0) {
        const crossing = run.firstHour - 1 - (flow.amountBefore[at] - basis[index][0]) / run.amount;
        if (crossing > Math.max(from, run.firstHour) && crossing < run.lastHour) change(crossing, index);
      }
    });
  });
  const hours = [...boundaries].sort((a, b) => a - b);
  const guide: CompactStackModel = { bands: [], hours, bounds: [], breakBefore: [] };
  const upper: (number | null)[] = [];
  const constants = scalar(); const linear = scalar(); const peaks = scalar();
  const groups = new Map<number, StackSum>();
  const current = flows.map(() => ZERO_CURVE);
  const positions = flows.map(() => -1);
  const ratio = (curve: ContributionCurve, sign: number) => {
    if (curve.offset == null || curve.coefficient === 0) return;
    const group = groups.get(curve.offset) ?? scalar();
    addStackAmount(group, sign * curve.coefficient);
    if (group.sum + group.error === 0) groups.delete(curve.offset); else groups.set(curve.offset, group);
  };
  for (const hour of hours) {
    for (const index of events.get(hour) ?? []) {
      work.eventUpdates++;
      const flow = flows[index];
      while (positions[index] + 1 < flow.runs.length && flow.runs[positions[index] + 1].firstHour <= hour) positions[index]++;
      const before = current[index];
      const after = contributionCurve(flow, positions[index], measure, basis[index], hour);
      addStackAmount(constants, -before.constant); addStackAmount(constants, after.constant);
      addStackAmount(peaks, -before.peak); addStackAmount(peaks, after.peak);
      if (measure === 'total') { addStackAmount(linear, -before.coefficient); addStackAmount(linear, after.coefficient); }
      else { ratio(before, -1); ratio(after, 1); }
      current[index] = after;
    }
    const recorded = flows.length > 0 && knownSpanAt(known, hour) >= 0;
    if (!recorded) { guide.bounds.push(null); guide.breakBefore.push(false); upper.push(null); continue; }
    const sum = scalar();
    addStackAmount(sum, constants.sum); addStackAmount(sum, constants.error);
    if (measure === 'total') { addStackAmount(sum, linear.sum * hour); addStackAmount(sum, linear.error * hour); }
    else for (const [offset, group] of groups) {
      work.groupEvaluations++;
      addStackAmount(sum, group.sum / (hour + offset)); addStackAmount(sum, group.error / (hour + offset));
    }
    guide.bounds.push([0, Math.max(0, sum.sum + sum.error)]);
    guide.breakBefore.push(false);
    upper.push(Math.max(0, peaks.sum + peaks.error));
  }
  for (let index = 0; index < hours.length; index++) {
    if (measure !== 'total' && hours[index + 1] !== from) continue;
    const before = guide.bounds[index]; const after = guide.bounds[index + 1];
    upper[index] = before ? Math.max(before[1], after?.[1] ?? before[1]) : null;
  }
  return { flows, measure, from, hours, basis, guide, upper, work };
}

export function forEachContributionMeasureSegment(model: ContributionMeasureModel, from: number, to: number, minHours: number,
  visit: (piece: StackSegment) => void): void {
  forEachCompactStackSegment(model.guide, from, to, minHours, visit,
    hour => stackEdges(model.flows.map((flow, index) => Math.max(0, flowValue(flow, model.measure, model.from, hour, model.basis[index])))));
}

export function contributionMeasureExtent(model: ContributionMeasureModel, from: number, to: number): [number, number] | null {
  if (to < from) return null;
  let start = 0;
  for (let low = 0, high = model.hours.length - 1; low <= high;) {
    const middle = (low + high) >> 1;
    if (model.hours[middle] <= from) { start = middle; low = middle + 1; } else high = middle - 1;
  }
  let highest: number | null = null;
  for (let index = start; index < model.hours.length && model.hours[index] <= to; index++) {
    const upper = model.upper[index];
    if (upper != null) highest = Math.max(highest ?? 0, upper);
  }
  return highest == null ? null : [0, highest];
}

export function pinContributions(source: WidgetSpec, track: Pick<BoundTrack, 'id' | 'branch' | 'source'>, selection: TradeSelection,
  time: GatedTime, extent: readonly [number, number], countryTag: string | null = null): PinnedSpec {
  const country = selection.kind === 'country' && countryTag;
  const pinned = pinnedBreakdown(country ? { ...source, countries: { kind: 'tags', tags: [countryTag] } } : source,
    selection.kind === 'country' ? COUNTRY_BREAKDOWN : selection.kind === 'market' ? { id: 'contributions', label: 'Market & lend-lease' } : TRADE_BREAKDOWN, track.id, time);
  return { ...pinned,
    options: { ...pinned.options, tradeSelection: selectionToken(selection, tradeContext(track)),
      tradeExtent: setToken(JSON.stringify(extent.map(at => at - track.branch.firstHour))) } };
}

