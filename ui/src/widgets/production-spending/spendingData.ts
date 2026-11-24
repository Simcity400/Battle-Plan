import type { SunburstItem, SunburstTotal } from '../../charts/sunburst/layout';
import { runAt, sameStretch } from '../../charts/series.ts';
import { formatIc } from '../../dashboard/icDecimals.ts';
import { DOMAINS, UNRESOLVED_ARM, domainName, unresolvedTypeName, variantNameAt } from '../../dashboard/equipment.ts';
import { readAt } from '../../dashboard/reads.ts';
import { windowOf, windowsOf } from '../../dashboard/heldWindows.ts';
import type { FactoryKind } from './definition.ts';
import type { Domain, ProductionSpendingLeaf, ProductionSpendingReply, ProductionSpendingRun, ProductionSpendingUnknown, RepairRatePart, StockVariantName } from '../../generated/dashboard';

export const LEVELS = ['domainLevel', 'typeLevel', 'variantLevel'] as const;
export const UNIT = 'IC';
const ATTRIBUTION_NOTE = 'Equipment type unknown.';

export const UNKNOWN_LABELS: Readonly<Record<ProductionSpendingUnknown, string>> = {
  notRecorded: 'Spending not recorded.',
  spendingUnavailable: 'Spending unavailable at this time.',
  lostOutput: 'Final total missing.',
  missedPublication: 'Spending unavailable at this time.',
  recordingGap: 'Hours were not recorded',
  observationRestarted: 'Campaign total unavailable.',
  observationMismatch: 'Production totals start on different dates.',
  countryChanged: 'Country changed during this period.',
  rocketTableUnavailable: 'Rocket spending not recorded.',
  rocketInputInexact: 'Rocket spending total unknown.',
  rocketOwnerUnreadable: 'Rocket producer unknown.',
  rocketDesignUnreadable: 'Rocket design unknown.',
};

type UnknownReason = ProductionSpendingUnknown | 'outsideRead' | 'observationChanged' | 'unattributed';
export const unknownLabel = (reason: string) => UNKNOWN_LABELS[reason as ProductionSpendingUnknown]
  ?? (reason === 'observationChanged' ? 'Campaign total unavailable.' : reason === 'unattributed' ? ATTRIBUTION_NOTE : 'Spending unavailable at this time.');

const unattributed = (design: { rocket: boolean; domain: Domain | null }) => !design.rocket && design.domain == null;
export const inFactoryKind = (design: { rocket: boolean; domain: Domain | null }, kind: FactoryKind) =>
  unattributed(design) || (kind === 'dockyards' ? !design.rocket && design.domain === 'naval' : design.rocket || design.domain !== 'naval');

const RAW = /^-?\d{1,39}$/;
const PURPOSES: readonly unknown[] = ['new', 'refit', 'converted', 'unknown'];
const int = (value: unknown): value is number => Number.isSafeInteger(value);
const text = (value: unknown) => value === null || typeof value === 'string';
const hours = (value: unknown): value is number[] => Array.isArray(value) && value.every((hour, index) => int(hour)
  && (index === 0 || value[index - 1] < hour));
export const validRuns = (value: unknown): value is ProductionSpendingRun[] => Array.isArray(value) && value.every((run, index) => typeof run === 'object' && run != null
  && int(run.firstHour) && int(run.lastHour) && run.firstHour <= run.lastHour && (index === 0 || value[index - 1].lastHour < run.firstHour)
  && (run.breakBefore === undefined || run.breakBefore === true)
  && (run.totalRaw === null ? Object.hasOwn(UNKNOWN_LABELS, run.reason) : typeof run.totalRaw === 'string' && RAW.test(run.totalRaw) && run.reason === null));
export const validNames = (names: unknown): names is StockVariantName[] => Array.isArray(names) && names.every((name, index) => typeof name === 'object' && name != null
  && int(name.firstHour) && text(name.name) && text(name.design) && typeof name.known === 'boolean' && (index === 0 || names[index - 1].firstHour < name.firstHour));

export type SpendingData = Omit<ProductionSpendingReply, 'window' | 'firstHour' | 'lastHour'>;

export function validateSpendingData(data: unknown): SpendingData {
  const reply = data as SpendingData;
  const ok = typeof reply === 'object' && reply != null && typeof reply.branchId === 'string' && typeof reply.tag === 'string'
    && reply.decimals === 5 && hours(reply.reads) && hours(reply.observations)
    && [reply.total, reply.totalWithoutRefits, reply.equipmentTotal, reply.equipmentTotalWithoutRefits, reply.rocketTotal].every(validRuns)
    && hours(reply.equipmentReads) && hours(reply.rocketReads) && hours(reply.equipmentObservations) && hours(reply.rocketObservations)
    && text(reply.rocketKey) && text(reply.rocketName)
    && Array.isArray(reply.production) && reply.production.every((run, index) => run != null
      && int(run.firstHour) && int(run.lastHour) && run.firstHour <= run.lastHour
      && (run.present === null || typeof run.present === 'boolean')
      && (run.emptyCensusHour === null || int(run.emptyCensusHour) && run.emptyCensusHour <= run.firstHour
        && readAt(reply.equipmentReads, run.emptyCensusHour) === run.emptyCensusHour && run.present === false)
      && (index === 0 || reply.production[index - 1].lastHour < run.firstHour))
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string')
    && Array.isArray(reply.leaves) && reply.leaves.every(leaf => typeof leaf === 'object' && leaf != null
      && typeof leaf.variant === 'string' && leaf.variant.length > 0 && typeof leaf.typeKey === 'string' && leaf.typeKey.length > 0 && text(leaf.name)
      && (leaf.domain === null || DOMAINS.includes(leaf.domain)) && typeof leaf.rocket === 'boolean' && validRuns(leaf.runs)
      && (leaf.rocket ? leaf.purpose === null : PURPOSES.includes(leaf.purpose)) && validRuns(leaf.unproven) && validNames(leaf.names))
    && new Set(reply.leaves.map(leafKey)).size === reply.leaves.length
    && (reply.repairs === null || typeof reply.repairs === 'object' && int(reply.repairs.denominator) && reply.repairs.denominator > 0
      && validRuns(reply.repairs.total) && Array.isArray(reply.repairs.parts) && reply.repairs.parts.every(part => typeof part === 'object' && part != null
        && (part.variant === null ? part.typeKey === '' : typeof part.variant === 'string' && part.variant.length > 0 && part.typeKey.length > 0)
        && text(part.name) && (part.domain === null || DOMAINS.includes(part.domain)) && validNames(part.names) && validRuns(part.runs))
      && new Set(reply.repairs.parts.map(part => part.variant)).size === reply.repairs.parts.length);
  if (!ok) throw new Error('The production spending reply is malformed.');
  return reply;
}

export function validateProductionSpendingReply(data: unknown): ProductionSpendingReply {
  const reply = validateSpendingData(data) as ProductionSpendingReply;
  if (!int(reply.window) || reply.window < 0 || !int(reply.firstHour) || !int(reply.lastHour)) throw new Error('The production spending reply is malformed.');
  return reply;
}

export const leafKey = (leaf: ProductionSpendingLeaf) => JSON.stringify([leaf.variant, leaf.rocket, leaf.purpose]);

export interface SpendingAmount { exact: bigint | null; reason: UnknownReason | null }

export function spendingAt(values: readonly ProductionSpendingRun[], hour: number): SpendingAmount {
  const run = runAt(values, Math.floor(hour));
  return run ? { exact: run.totalRaw == null ? null : BigInt(run.totalRaw), reason: run.reason }
    : { exact: null, reason: 'outsideRead' };
}

export interface SpendingWindow { from: number | null; hour: number; end: number }

export const observationsOf = (reply: SpendingData, leaf: { rocket: boolean }) => leaf.rocket ? reply.rocketObservations : reply.equipmentObservations;

export function amountIn(values: readonly ProductionSpendingRun[], from: number | null, hour: number, observations: readonly number[]): SpendingAmount {
  const amount = spendingAt(values, hour);
  if (from == null || amount.exact == null) return amount;
  const observation = readAt(observations, hour);
  if (observation != null && from < observation) return { exact: null, reason: 'observationChanged' };
  if (from === observation) return amount;
  const before = spendingAt(values, from - 1);
  if (before.exact == null) return before;
  if (!sameStretch(values, from - 1, hour)) return { exact: null, reason: 'outsideRead' };
  return { exact: amount.exact - before.exact, reason: null };
}

export function sumAmounts(amounts: Iterable<SpendingAmount>): SpendingAmount {
  let exact = 0n;
  for (const amount of amounts) {
    if (amount.exact == null) return amount;
    exact += amount.exact;
  }
  return { exact, reason: null };
}

export interface SpendingDesign {
  variant: string; typeKey: string; name: string | null; domain: Domain | null; rocket: boolean;
  names: readonly StockVariantName[]; parts: readonly ProductionSpendingLeaf[];
}

export function spendingDesigns(reply: SpendingData, refits: boolean, kind?: FactoryKind): SpendingDesign[] {
  const designs = new Map<string, SpendingDesign & { parts: ProductionSpendingLeaf[] }>();
  for (const leaf of reply.leaves) {
    if (!refits && leaf.purpose === 'refit') continue;
    if (kind && !inFactoryKind(leaf, kind)) continue;
    const key = JSON.stringify([leaf.variant, leaf.rocket]);
    const held = designs.get(key);
    if (held) held.parts.push(leaf);
    else designs.set(key, { variant: leaf.variant, typeKey: leaf.typeKey, name: leaf.name, domain: leaf.domain, rocket: leaf.rocket, names: leaf.names, parts: [leaf] });
  }
  return [...designs.values()];
}

export const designAmount = (reply: SpendingData, design: SpendingDesign, from: number | null, hour: number) =>
  sumAmounts(design.parts.map(part => amountIn(part.runs, from, hour, observationsOf(reply, design))));

function wholeOf(reply: SpendingData, kind: FactoryKind | undefined, refits: boolean): { runs: readonly ProductionSpendingRun[]; observations: readonly number[] } {
  return kind === 'dockyards'
    ? { runs: refits ? reply.equipmentTotal : reply.equipmentTotalWithoutRefits, observations: reply.equipmentObservations }
    : { runs: refits ? reply.total : reply.totalWithoutRefits, observations: reply.observations };
}

const UNRECORDED_REASONS: ReadonlySet<ProductionSpendingUnknown | null> = new Set(['notRecorded', 'rocketTableUnavailable']);

export const spendingRecorded = (reply: SpendingData): boolean =>
  [reply.total, reply.equipmentTotal, reply.rocketTotal].some(runs =>
    runs.some(run => run.totalRaw != null || !UNRECORDED_REASONS.has(run.reason)));

export type SpendingGate = 'empty' | 'error' | 'outside' | 'loading' | 'unavailable' | 'ready';

export function spendingWidgetGate(input: {
  hasTarget: boolean; hasError: boolean;
  branch: { firstHour: number; lastHour: number } | undefined;
  hour: number | undefined; end: number | undefined;
  hasData: boolean; recorded: boolean;
}): SpendingGate {
  if (!input.hasTarget || !input.branch) return 'empty';
  if (input.hasError) return 'error';
  if (input.hour == null || input.end == null || input.hour < input.branch.firstHour || input.hour > input.branch.lastHour) return 'outside';
  if (!input.hasData) return 'loading';
  if (!input.recorded) return 'unavailable';
  return 'ready';
}

export const noProductionAt = (reply: SpendingData, hour: number, refits = true) => {
  const evidence = runAt(reply.production, Math.floor(hour));
  return evidence?.present === false && evidence.emptyCensusHour != null && spendingAt(refits ? reply.total : reply.totalWithoutRefits, hour).exact === 0n;
};

export function kindAmount(reply: SpendingData, designs: readonly SpendingDesign[], from: number | null, hour: number, kind: FactoryKind | undefined, refits: boolean): SpendingAmount {
  const whole = wholeOf(reply, kind, refits);
  const all = amountIn(whole.runs, from, hour, whole.observations);
  if (!kind || all.exact == null) return all;
  let exact = 0n;
  for (const design of designs) {
    const amount = designAmount(reply, design, from, hour);
    if (amount.exact == null) return amount;
    if (amount.exact !== 0n && unattributed(design)) return { exact: null, reason: 'unattributed' };
    exact += amount.exact;
  }
  return { exact, reason: null };
}

export function spendingEmptyLabel(reply: SpendingData, designs: readonly SpendingDesign[], hour: number, from: number | null, kind: FactoryKind | undefined, refits: boolean): string {
  const amount = kindAmount(reply, designs, from, hour, kind, refits);
  if (amount.exact == null) return unknownLabel(amount.reason ?? 'outsideRead');
  if (noProductionAt(reply, hour, refits)) return 'No military production yet.';
  const factories = kind === 'dockyards' ? 'dockyard' : kind === 'military' ? 'military factory' : 'factory';
  return runAt(reply.production, Math.floor(hour))?.present == null
    ? `No ${factories} IC spending recorded.` : `No ${factories} IC spent in this view.`;
}

export interface SpendingFinal { exact: bigint | null; geometry: number }

export function spendingFinals(reply: SpendingData, designs: readonly SpendingDesign[], from: number | null, end: number, hour = end): SpendingFinal[] {
  if (end > hour) return designs.map(() => ({ exact: null, geometry: 0 }));
  return designs.map(design => {
    const amount = designAmount(reply, design, from, end);
    if (amount.exact != null) return { exact: amount.exact, geometry: Number(amount.exact) };
    let geometry = 0n;
    for (const part of design.parts) {
      const observations = observationsOf(reply, design);
      const baseline = from == null || observations.includes(from) ? 0n : spendingAt(part.runs, from - 1).exact;
      let maximum = 0n;
      for (const run of part.runs) {
        if (run.firstHour > end) break;
        if (baseline == null || run.totalRaw == null || (from != null && run.lastHour < from)) continue;
        const candidate = BigInt(run.totalRaw) - baseline;
        if (candidate > maximum) maximum = candidate;
      }
      geometry += maximum;
    }
    return { exact: null, geometry: Number(geometry) };
  });
}

export function productionSpendingItems(reply: SpendingData, window: SpendingWindow, designs: readonly SpendingDesign[],
  finals: readonly SpendingFinal[], kind: FactoryKind | undefined, refits: boolean): SunburstItem[] {
  const futureEnd = window.end > window.hour;
  const amounts = designs.map(design => designAmount(reply, design, window.from, window.hour));
  const unresolved = designs.some((design, index) => !design.rocket && design.domain == null && amounts[index].exact !== 0n);
  const unresolvedAtEnd = !futureEnd && designs.some((design, index) => !design.rocket && design.domain == null && finals[index].exact !== 0n);
  const unresolvedRocket = designs.some((design, index) => design.rocket && design.domain == null && amounts[index].exact !== 0n);
  const unresolvedRocketAtEnd = !futureEnd && designs.some((design, index) => design.rocket && design.domain == null && finals[index].exact !== 0n);
  const items: SunburstItem[] = designs.map((design, index) => {
    const amount = amounts[index];
    const final = finals[index];
    const observed = amount.exact == null || amount.exact !== 0n;
    const attribution = !design.rocket && unresolved && design.domain != null;
    const finalAttribution = !design.rocket && unresolvedAtEnd && design.domain != null;
    const typeAttribution = design.rocket ? unresolvedRocket && design.domain != null : attribution;
    const identity = design.rocket ? `rocket:${design.variant}` : design.variant;
    return {
      path: [
        design.rocket ? { id: 'rocket', label: reply.rocketName ?? reply.rocketKey ?? 'Rocket sites' }
          : { id: design.domain ?? UNRESOLVED_ARM, label: domainName(design.domain ?? UNRESOLVED_ARM) },
        { id: design.typeKey, label: design.name ?? unresolvedTypeName(design.typeKey) },
        { id: identity, label: variantNameAt(design.names, window.hour, design.variant) },
      ],
      value: amount.exact == null ? null : Number(amount.exact), exact: amount.exact,
      unknownValue: amount.exact == null, unknownLevels: [attribution, attribution, false],
      localUnknownLevels: [false, design.rocket && typeAttribution, false],
      note: amount.reason ? unknownLabel(amount.reason) : typeAttribution ? ATTRIBUTION_NOTE : undefined,
      final: futureEnd ? 0 : final.geometry,
      ...(futureEnd ? { weight: amount.exact == null ? 0 : Number(amount.exact) } : {}),
      finalUnknown: futureEnd || final.exact == null,
      finalUnknownLevels: [finalAttribution, finalAttribution, false],
      localFinalUnknownLevels: [false, design.rocket && unresolvedRocketAtEnd && design.domain != null, false],
      present: observed || typeAttribution, counted: observed ? [identity] : [],
    };
  });
  const total = kindAmount(reply, designs, window.from, window.hour, kind, refits);
  const finalTotal = futureEnd ? { exact: null, reason: null } : kindAmount(reply, designs, window.from, window.end, kind, refits);
  const hasParts = items.some(item => item.present);
  if (total.exact == null || !hasParts || (!futureEnd && finalTotal.exact == null)) items.push({
    path: [], value: total.exact == null ? null : hasParts ? 0 : Number(total.exact), exact: total.exact == null ? null : hasParts ? 0n : total.exact,
    unknownValue: total.exact == null, note: total.reason ? unknownLabel(total.reason) : undefined,
    final: 0, finalUnknown: finalTotal.exact == null,
  });
  return items;
}

export function spendingTotal(reply: SpendingData, designs: readonly SpendingDesign[], window: SpendingWindow, kind: FactoryKind | undefined, refits: boolean): SunburstTotal {
  const amount = kindAmount(reply, designs, window.from, window.hour, kind, refits);
  const final = window.end > window.hour ? { exact: null, reason: null } : kindAmount(reply, designs, window.from, window.end, kind, refits);
  return {
    value: amount.exact == null ? null : Number(amount.exact), exact: amount.exact,
    unknownValue: amount.exact == null, note: amount.reason ? unknownLabel(amount.reason) : undefined,
    final: final.exact == null ? 0 : Number(final.exact), finalUnknown: final.exact == null,
  };
}

export const formatAppliedIc = (value: number, exact: bigint | null, decimals: number, digits: number) =>
  formatIc(value, exact, decimals, digits);


const sameRun = (a: ProductionSpendingRun, b: ProductionSpendingRun) => a.totalRaw === b.totalRaw && a.reason === b.reason;

function joinRuns(runs: ProductionSpendingRun[], next: readonly ProductionSpendingRun[]): void {
  for (const run of next) {
    const open = runs.at(-1);
    if (open && !run.breakBefore && open.lastHour + 1 === run.firstHour && sameRun(open, run)) runs[runs.length - 1] = { ...open, lastHour: run.lastHour };
    else runs.push({ ...run });
  }
}

function joinNames(names: StockVariantName[], next: readonly StockVariantName[]): void {
  for (const name of next) if (!names.some(held => held.firstHour === name.firstHour && held.name === name.name && held.known === name.known)) names.push(name);
  names.sort((a, b) => a.firstHour - b.firstHour);
}

const union = (lists: readonly (readonly number[])[]) => [...new Set(lists.flat())].sort((a, b) => a - b);
const absentRuns = (basis: readonly ProductionSpendingRun[]): ProductionSpendingRun[] =>
  basis.map(run => run.totalRaw == null ? { ...run } : { ...run, totalRaw: '0', reason: null });

function absentRepairBasis(repairs: NonNullable<ProductionSpendingReply['repairs']>): ProductionSpendingRun[] {
  const unattributed = repairs.parts.find(part => part.variant === null);
  if (unattributed) return absentRuns(unattributed.runs);
  return repairs.total.map(run => run.totalRaw == null ? { ...run }
    : BigInt(run.totalRaw) === 0n ? { ...run, totalRaw: '0', reason: null }
    : { ...run, totalRaw: null, reason: 'spendingUnavailable' });
}

export function displayedSpendingWindows(branch: { firstHour: number; lastHour: number }, at: number, from: number | null,
  whole: boolean, repairs: boolean): number[] {
  const clampedAt = Math.min(branch.lastHour, Math.floor(at));
  if (clampedAt < branch.firstHour) return [];
  const start = from == null ? branch.firstHour : Math.max(branch.firstHour, from - 1);
  if (whole || repairs || from != null) return windowsOf(start, clampedAt, false);
  return [windowOf(clampedAt)];
}

export function prefetchSpendingWindows(branch: { firstHour: number; lastHour: number }, at: number, from: number | null,
  whole: boolean, repairs: boolean, ahead: boolean): number[] {
  const needed = displayedSpendingWindows(branch, at, from, whole, repairs);
  if (!ahead || !needed.length) return needed;
  const next = needed[needed.length - 1] + 1;
  return next <= windowOf(branch.lastHour) ? [...needed, next] : needed;
}

export type SpendingWindowBuilder = (branch: { firstHour: number; lastHour: number }, at: number, from: number | null,
  whole: boolean, repairs: boolean, ahead: boolean) => number[];

export function createWindowTracker(build: SpendingWindowBuilder = prefetchSpendingWindows) {
  let cached: {
    firstHour: number; lastHour: number; atWindow: number; from: number | null;
    whole: boolean; repairs: boolean; ahead: boolean; windows: number[];
  } | null = null;
  let recomputations = 0;
  return {
    get recomputations() { return recomputations; },
    get(branch: { firstHour: number; lastHour: number }, at: number, from: number | null,
      whole: boolean, repairs: boolean, ahead: boolean): number[] {
      const clamped = Math.min(branch.lastHour, Math.floor(at));
      const atWindow = clamped < branch.firstHour ? -1 : windowOf(clamped);
      if (cached && cached.firstHour === branch.firstHour && cached.lastHour === branch.lastHour
        && cached.atWindow === atWindow && cached.from === from && cached.whole === whole
        && cached.repairs === repairs && cached.ahead === ahead) return cached.windows;
      recomputations++;
      const windows = build(branch, at, from, whole, repairs, ahead);
      cached = { firstHour: branch.firstHour, lastHour: branch.lastHour, atWindow, from, whole, repairs, ahead, windows };
      return windows;
    },
  };
}

export function selectSpendingState<E>(needed: readonly number[], data: ReadonlyMap<number, ProductionSpendingReply>,
  errors: ReadonlyMap<number, E>, at?: number): { data: SpendingData | undefined; error: E | undefined } {
  const error = needed.map(index => errors.get(index)).find(failure => failure !== undefined);
  if (error !== undefined) return { data: undefined, error };
  if (!needed.length || !needed.every(index => data.has(index))) return { data: undefined, error: undefined };
  const selected = new Map<number, ProductionSpendingReply>();
  for (const index of needed) selected.set(index, data.get(index)!);
  return { data: mergeSpending(selected, at) ?? undefined, error: undefined };
}

export function mergeSpending(windows: ReadonlyMap<number, ProductionSpendingReply>, atHour?: number): SpendingData | null {
  const cutoff = atHour == null ? Infinity : Math.floor(atHour);
  const held = [...windows.values()].filter(reply => reply.firstHour <= cutoff).sort((a, b) => a.window - b.window);
  if (!held.length) return null;
  const leaves = new Map<string, ProductionSpendingLeaf>();
  const hasUnproven = new Set<string>();
  const totals = { total: [] as ProductionSpendingRun[], totalWithoutRefits: [] as ProductionSpendingRun[], equipmentTotal: [] as ProductionSpendingRun[],
    equipmentTotalWithoutRefits: [] as ProductionSpendingRun[], rocketTotal: [] as ProductionSpendingRun[] };
  const production: SpendingData['production'] = [];
  const parts = new Map<string | null, RepairRatePart>();
  const repairTotal: ProductionSpendingRun[] = [];
  const warnings = new Set<string>();
  for (const reply of held) {
    for (const leaf of reply.leaves) {
      const key = leafKey(leaf);
      if (!leaves.has(key)) leaves.set(key, { ...leaf, runs: [], unproven: [], names: [] });
      if (leaf.unproven.length) hasUnproven.add(key);
    }
    for (const part of reply.repairs?.parts ?? []) {
      if (!parts.has(part.variant)) parts.set(part.variant, { ...part, runs: [], names: [] });
    }
  }
  for (const reply of held) {
    const windowLeaves = new Map(reply.leaves.map(leaf => [leafKey(leaf), leaf]));
    for (const [key, into] of leaves) {
      const leaf = windowLeaves.get(key);
      const basis = into.rocket ? reply.rocketTotal : reply.equipmentTotal;
      if (leaf) {
        joinRuns(into.runs, leaf.runs);
        if (hasUnproven.has(key)) joinRuns(into.unproven, leaf.unproven.length ? leaf.unproven : absentRuns(basis));
        joinNames(into.names, leaf.names);
      } else {
        joinRuns(into.runs, absentRuns(basis));
        if (hasUnproven.has(key)) joinRuns(into.unproven, absentRuns(basis));
      }
    }
    for (const field of Object.keys(totals) as (keyof typeof totals)[]) joinRuns(totals[field], reply[field]);
    for (const run of reply.production) {
      const open = production.at(-1);
      if (open && open.lastHour + 1 === run.firstHour && open.present === run.present) production[production.length - 1] = { ...open, lastHour: run.lastHour };
      else production.push({ ...run });
    }
    if (reply.repairs) {
      const windowParts = new Map(reply.repairs.parts.map(part => [part.variant, part]));
      const absentRepair = absentRepairBasis(reply.repairs);
      for (const [variant, into] of parts) {
        const part = windowParts.get(variant);
        if (part) {
          joinRuns(into.runs, part.runs);
          joinNames(into.names, part.names);
        } else joinRuns(into.runs, variant === null ? absentRuns(reply.repairs.total) : absentRepair);
      }
      joinRuns(repairTotal, reply.repairs.total);
    }
    for (const warning of reply.warnings) warnings.add(warning);
  }
  const { window: _window, firstHour: _firstHour, lastHour: _lastHour, ...first } = held[0];
  const named = [...held].reverse().find(reply => reply.rocketKey !== null) ?? first;
  const repairs = held.find(reply => reply.repairs !== null)?.repairs ?? null;
  return { ...first, ...totals, rocketKey: named.rocketKey, rocketName: named.rocketName,
    reads: union(held.map(reply => reply.reads)), observations: union(held.map(reply => reply.observations)),
    equipmentObservations: union(held.map(reply => reply.equipmentObservations)), rocketObservations: union(held.map(reply => reply.rocketObservations)),
    equipmentReads: union(held.map(reply => reply.equipmentReads)), rocketReads: union(held.map(reply => reply.rocketReads)),
    leaves: [...leaves.values()], production, repairs: repairs && { denominator: repairs.denominator, total: repairTotal, parts: [...parts.values()] },
    warnings: [...warnings] };
}
