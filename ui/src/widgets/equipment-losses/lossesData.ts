import type { Coverage, LossCause, LossDesign, LossLeaf, LossName, LossPoint, LossSource, LossesReply, UnknownSpan } from '../../generated/dashboard';
import type { SunburstItem, SunburstNode, SunburstTotal } from '../../charts/sunburst/layout';
import type { LegendItem } from '../../charts/Legend';
import type { ChartTokens } from '../../charts/tokens';
import { mix } from '../../shell/appearance/color.ts';
import type { TooltipRow } from '../../charts/tooltip';
import { formatAmount } from '../../charts/valueAxis.ts';
import { unresolvedTypeName } from '../../dashboard/equipment.ts';
import { entryAt } from '../../dashboard/reads.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { int, object } from '../../dashboard/validation.ts';

export const LEVELS = ['causeLevel', 'typeLevel', 'designLevel'] as const;
export const UNKNOWN_LABELS = {
  unread: 'Cause not read at the moment shown',
  endedAfterLastRead: 'Exchange ended after its last read',
  destructionWithheld: 'Destruction table withheld',
  destructionGap: 'Destruction drain not recorded',
  destructionUnresolved: 'Destruction owner not resolved',
  destructionDomain: 'Destruction equipment arm not resolved',
};

const text = (value: unknown) => value === null || typeof value === 'string';
const RAW = /^-?\d{1,39}$/;
export const sourceKey = (source: LossSource) => source.kind === 'combatLog' || source.kind === 'destruction' ? `${source.kind}:${source.reason}` : source.kind;
const isSource = (source: LossSource) => object(source) && (source.kind === 'combatLog'
  ? int(source.reason) && source.reason >= 0 && source.reason <= 255
  : source.kind === 'destruction' ? typeof source.reason === 'string' && source.reason.length > 0
  : ['recovered', 'sunkShips', 'sunkConvoys', 'lendLeaseSunk', 'lendLeaseSent'].includes(source.kind));
const isName = (name: LossName) => object(name) && text(name.key) && text(name.template) && text(name.text);
const isCoverage = (coverage: Coverage) => ['logged', 'notLogged', 'unknown', 'pruneUnobserved'].includes(coverage as string)
  || (typeof coverage === 'object' && coverage !== null && 'partial' in coverage && object(coverage.partial) && int(coverage.partial.ungrouped) && coverage.partial.ungrouped > 0);
const isSpan = (span: UnknownSpan) => object(span) && int(span.firstHour) && int(span.lastHour) && span.firstHour <= span.lastHour
  && Object.hasOwn(UNKNOWN_LABELS, span.reason) && text(span.detail);
const isHours = (span: { firstHour: number; lastHour: number }) => object(span) && int(span.firstHour) && int(span.lastHour) && span.firstHour <= span.lastHour;
const isDesign = (design: LossDesign) => object(design) && int(design.variant) && design.variant >= 0
  && [design.equipment, design.archetype, design.archetypeName].every(text)
  && Array.isArray(design.names) && design.names.every((read, index) => object(read) && int(read.firstHour) && text(read.name)
    && typeof read.known === 'boolean' && (read.design === null || typeof read.design === 'string' && read.design.length > 0)
    && (index === 0 || design.names[index - 1].firstHour < read.firstHour));
const isLeaf = (leaf: LossLeaf, designs: number) => object(leaf)
  && (leaf.design === null || (int(leaf.design) && leaf.design >= 0 && leaf.design < designs))
  && (leaf.lowerBoundFrom === null || int(leaf.lowerBoundFrom)) && Array.isArray(leaf.points)
  && leaf.points.every((point, index) => object(point) && int(point.hour) && typeof point.totalRaw === 'string' && RAW.test(point.totalRaw)
    && (index === 0 || leaf.points[index - 1].hour < point.hour));

/** Validate before local playback; unknown spans may overlap for different reasons. */
export function validateLossesReply(data: unknown): LossesReply {
  const reply = data as LossesReply;
  const ok = object(reply) && typeof reply.branchId === 'string' && typeof reply.tag === 'string'
    && ['destruction', 'combatLog'].includes(reply.landSource) && text(reply.landLowerBound)
    && (reply.destructionCountedFrom === null || int(reply.destructionCountedFrom))
    && int(reply.amountScale) && reply.amountScale > 0 && Number.isInteger(Math.log10(reply.amountScale))
    && (reply.retainedMonths === null || int(reply.retainedMonths))
    && Array.isArray(reply.designs) && reply.designs.every(isDesign)
    && new Set(reply.designs.map(design => design.variant)).size === reply.designs.length
    && Array.isArray(reply.coverage) && reply.coverage.every((span, index) => object(span) && int(span.firstHour) && int(span.lastHour)
      && span.firstHour <= span.lastHour && isCoverage(span.coverage) && (index === 0 || reply.coverage[index - 1].lastHour < span.firstHour))
    && Array.isArray(reply.destructionCoverage) && reply.destructionCoverage.every((span, index) => isHours(span) && text(span.reason)
      && (index === 0 || reply.destructionCoverage[index - 1].lastHour < span.firstHour))
    && Array.isArray(reply.causes) && reply.causes.every(cause => object(cause) && isSource(cause.source) && isName(cause.name)
      && ['land', 'air', 'naval', 'other'].includes(cause.domain)
      && (cause.group === null || isName(cause.group)) && typeof cause.loss === 'boolean' && text(cause.unknown)
      && Array.isArray(cause.unknownSpans) && cause.unknownSpans.every(isSpan)
      && Array.isArray(cause.leaves) && cause.leaves.every(leaf => isLeaf(leaf, reply.designs.length))
      && new Set(cause.leaves.map(leaf => leaf.design)).size === cause.leaves.length
      && (cause.unknown === null || cause.leaves.length === 0))
    && new Set(reply.causes.map(cause => sourceKey(cause.source))).size === reply.causes.length
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string');
  if (!ok) throw new Error('The equipment losses reply is malformed.');
  return reply;
}

/** Loss totals change only on their recorded points, never between them. */
export function pointAt(leaf: LossLeaf, hour: number): LossPoint | null {
  let low = 0;
  let high = leaf.points.length - 1;
  let found: LossPoint | null = null;
  while (low <= high) {
    const middle = (low + high) >> 1;
    if (leaf.points[middle].hour <= hour) { found = leaf.points[middle]; low = middle + 1; } else high = middle - 1;
  }
  return found;
}

const peaks = new WeakMap<LossLeaf, { firstHour: number; value: number }[]>();
function weightAt(leaf: LossLeaf, hour: number, from?: number | null): number {
  if (from != null) {
    const baseline = BigInt(pointAt(leaf, from - 1)?.totalRaw ?? '0');
    return leaf.points.reduce((peak, point) => point.hour < from || point.hour > hour ? peak
      : Math.max(peak, Math.abs(Number(BigInt(point.totalRaw) - baseline))), 0);
  }
  let steps = peaks.get(leaf);
  if (!steps) {
    steps = [];
    let peak = 0;
    for (const point of leaf.points) {
      const value = Math.abs(Number(BigInt(point.totalRaw)));
      if (value > peak) { peak = value; steps.push({ firstHour: point.hour, value }); }
    }
    peaks.set(leaf, steps);
  }
  return entryAt(steps, hour)?.value ?? 0;
}

export const causeName = (cause: LossCause) => cause.name.text ?? cause.name.template ?? cause.name.key ?? `Unnamed cause (${sourceKey(cause.source)})`;
export const typeName = (design: LossDesign | undefined) => design?.archetypeName
  ?? (design?.archetype != null ? unresolvedTypeName(design.archetype) : 'Equipment type not known');

const spansAt = (cause: LossCause, hour: number) => [...new Map(cause.unknownSpans
  .filter(span => span.firstHour <= hour && hour <= span.lastHour)
  .map(span => [JSON.stringify([span.reason, span.detail]), span])).values()];
const destructionReason = (span: UnknownSpan) => span.reason.startsWith('destruction');
const spanNote = (span: UnknownSpan) => span.detail ?? UNKNOWN_LABELS[span.reason];

/** Reasons stay visible when the cause ring or slice legend is switched off. */
export const unknownCauses = (reply: LossesReply, hour: number) => reply.causes.filter(cause => cause.loss).flatMap(cause => {
  if (cause.unknown !== null) return [{ id: sourceKey(cause.source), label: `${causeName(cause)}: Not known — ${cause.unknown}` }];
  const spans = spansAt(cause, hour);
  const known = cause.leaves.some(leaf => pointAt(leaf, hour));
  return spans.map(span => ({ id: `${sourceKey(cause.source)}:${JSON.stringify([span.reason, span.detail])}`, label: span.reason === 'unread'
    ? `${causeName(cause)}: Not known at this hour — cause not read; recorded amounts may lag.`
    : destructionReason(span) ? `${causeName(cause)}: ${known ? 'recorded subtotal; total Not known' : 'Not known'} — ${spanNote(span)}`
      : `${causeName(cause)}: recorded subtotal; total Not known — an exchange ended after its last read.` }));
});

type LandState = 'logged' | 'partial' | 'notLogged' | 'unknown';
const stateOf = (coverage: Coverage): LandState => typeof coverage === 'object' ? 'partial' : coverage === 'pruneUnobserved' ? 'unknown' : coverage;
export function coverageAt(reply: LossesReply, hour: number): Coverage {
  const span = entryAt(reply.coverage, hour);
  return span && span.lastHour >= hour ? span.coverage : 'unknown';
}

/** Inclusive hourly counts; an omitted hour is unknown, never logged. */
export function coverageHours(reply: LossesReply, first: number, last: number): Record<LandState, number> {
  const counts = { logged: 0, partial: 0, notLogged: 0, unknown: 0 };
  let covered = 0;
  for (const span of reply.coverage) {
    const hours = Math.max(0, Math.min(last, span.lastHour) - Math.max(first, span.firstHour) + 1);
    counts[stateOf(span.coverage)] += hours;
    covered += hours;
  }
  counts.unknown += Math.max(0, last - first + 1 - covered);
  return counts;
}

export interface LossWindow { first: number; hour: number; end: number; campaign: boolean; from?: number | null }

/** Follow's trailing frame window is irrelevant to cumulative totals. Ranges are inclusive. */
export function lossWindow(window: { firstHour: number; lastHour: number } | null,
  branch: { firstHour: number; lastHour: number }, mode: string): LossWindow | null {
  if (!window || !int(window.lastHour) || window.lastHour < branch.firstHour || window.lastHour > branch.lastHour
    || mode !== 'follow' && (!int(window.firstHour) || window.firstHour < branch.firstHour || window.firstHour > window.lastHour)) return null;
  return { first: mode === 'range' ? window.firstHour : branch.firstHour, hour: window.lastHour,
    end: mode === 'range' ? window.lastHour : branch.lastHour, campaign: mode === 'campaign' || mode === 'range',
    from: mode === 'range' ? window.firstHour : null };
}

/** Range amounts are changes at supplying reads, including a selected first retained read. */
export function lossAmount(leaf: LossLeaf, hour: number, from?: number | null): bigint | null {
  const point = pointAt(leaf, hour);
  return point ? BigInt(point.totalRaw) - (from == null ? 0n : BigInt(pointAt(leaf, from - 1)?.totalRaw ?? '0')) : null;
}

export const lossKey = (cause: LossCause, design: LossDesign | undefined) => JSON.stringify([sourceKey(cause.source), design?.variant ?? null]);
export const lossDesignName = (design: LossDesign | undefined, hour: number) => {
  if (!design) return 'Design not recorded';
  const read = entryAt(design.names, hour);
  return (read?.known ? read.name : null) ?? `Design ${design.variant} (name not known)`;
};

export interface LossItem extends SunburstItem {
  /** Independent ordering weight; consumed by the reviewed optional shared weight API. */
  weight?: number;
  /** A measured subtotal remains exact, with its missing-total reason alongside it. */
  incomplete?: boolean;
}
export interface LossTotal extends SunburstTotal { exact: bigint | null; incomplete: boolean }


const NO_DESTRUCTION_READ = 'No destruction drain was recorded at this hour.';
export function destructionCoverageAt(reply: LossesReply, hour: number): string | null {
  const span = entryAt(reply.destructionCoverage, hour);
  return span && span.lastHour >= hour ? span.reason : NO_DESTRUCTION_READ;
}

/** The legend describes the drain intervals, independently of cumulative unknown spans. */
export function destructionCoverageHours(reply: LossesReply, first: number, last: number) {
  first = Math.max(first, reply.destructionCountedFrom ?? reply.destructionCoverage[0]?.firstHour ?? last + 1);
  let recorded = 0;
  let unknown = 0;
  const reasons = new Set<string>();
  for (const span of reply.destructionCoverage) {
    const hours = Math.max(0, Math.min(last, span.lastHour) - Math.max(first, span.firstHour) + 1);
    if (!hours) continue;
    if (span.reason === null) recorded += hours;
    else { unknown += hours; reasons.add(span.reason); }
  }
  const missing = Math.max(0, last - first + 1 - recorded - unknown);
  if (missing) { unknown += missing; reasons.add(NO_DESTRUCTION_READ); }
  return { recorded, unknown, reasons: [...reasons] };
}

/** Moment coverage with Partial's exact N, or hours per state for Live off. */
export function destructionLegend(reply: LossesReply, window: LossWindow, tokens: Pick<ChartTokens, 'positive' | 'surface' | 'text'>): LegendItem[] {
  const neutral = mix(tokens.surface, tokens.text, 0.1);
  const first = reply.destructionCountedFrom;
  // The uncounted opening interval is explained by the footnote, not a gap state.
  if ((first === null && !reply.destructionCoverage.length) || (first !== null && window.hour < first)) return [];
  if (!window.campaign) {
    const reason = destructionCoverageAt(reply, window.hour);
    return [{ id: reason === null ? 'recorded' : 'unknown', color: reason === null ? tokens.positive : neutral,
      label: reason === null ? 'Recorded' : `Unknown: ${reason}` }];
  }
  const counts = destructionCoverageHours(reply, window.first, window.hour);
  const hours = (count: number) => `${count.toLocaleString('en-US')} ${count === 1 ? 'hour' : 'hours'}`;
  return [counts.recorded ? { id: 'recorded', color: tokens.positive, label: `Recorded: ${hours(counts.recorded)}` } : null,
    counts.unknown ? { id: 'unknown', color: neutral, label: `Unknown: ${hours(counts.unknown)} — ${counts.reasons.join(' ')}` } : null]
    .filter((item): item is NonNullable<typeof item> => item !== null);
}

export function coverageLegend(reply: LossesReply, window: LossWindow, tokens: Pick<ChartTokens, 'positive' | 'warning' | 'error' | 'surface' | 'text'>): LegendItem[] {
  if (reply.landSource === 'destruction') return destructionLegend(reply, window, tokens);
  const neutral = mix(tokens.surface, tokens.text, 0.1);
  const colours = { logged: tokens.positive, partial: tokens.warning, notLogged: tokens.error, unknown: neutral };
  const labels = { logged: 'Logged', partial: 'Partial', notLogged: 'Not logged', unknown: 'Unknown' };
  if (window.campaign) return Object.entries(coverageHours(reply, window.first, window.hour)).filter(([, hours]) => hours > 0)
    .map(([state, hours]) => ({ id: state, color: colours[state as LandState], label: `${labels[state as LandState]}: ${hours.toLocaleString('en-US')} ${hours === 1 ? 'hour' : 'hours'}` }));
  const coverage = coverageAt(reply, window.hour);
  const state = stateOf(coverage);
  const label = typeof coverage === 'object' ? `Partial: ${coverage.partial.ungrouped.toLocaleString('en-US')} ${coverage.partial.ungrouped === 1 ? 'division' : 'divisions'} outside armies`
    : coverage === 'pruneUnobserved' ? 'Unknown: retention was not observed' : labels[state];
  return [{ id: state, color: colours[state], label }];
}

/** Unlogged land hours cannot become a complete campaign total after rejoining an army. */
export function landNote(reply: LossesReply, first: number, hour: number): string | undefined {
  if (reply.landSource === 'destruction') return reply.landLowerBound ?? undefined;
  const counts = coverageHours(reply, first, hour);
  const notes: string[] = [];
  if (counts.partial || counts.notLogged) notes.push('Land losses are incomplete: some hours were partial or not logged.');
  if (counts.unknown) notes.push('Land losses are unknown for some hours; recorded amounts may be incomplete.');
  if (reply.coverage.some(span => span.coverage === 'pruneUnobserved' && span.firstHour <= hour && span.lastHour >= first)) notes.push('Land-loss retention was not observed at some hours.');
  return notes.join(' ') || undefined;
}

/** Native cause, content archetype and recorded design; unknown causes invent no leaves. */
export function lossItems(reply: LossesReply, window: LossWindow): LossItem[] {
  const items: LossItem[] = [];
  const land = landNote(reply, window.first, window.hour);
  for (const cause of reply.causes) {
    if (!cause.loss) continue;
    const causePath = [{ id: `cause:${sourceKey(cause.source)}`, label: causeName(cause) }];
    if (cause.unknown !== null) {
      items.push({ path: causePath, value: null, final: null, finalUnknown: true, unknownValue: true, incomplete: true, note: cause.unknown });
      continue;
    }
    const spans = spansAt(cause, window.hour), finalSpans = spansAt(cause, window.end);
    const destructionNotes = [...new Set(spans.filter(destructionReason).map(spanNote))];
    const unread = spans.some(span => span.reason === 'unread');
    const ended = spans.some(span => span.reason === 'endedAfterLastRead');
    const started = cause.source.kind !== 'destruction' || reply.destructionCountedFrom !== null && window.hour >= reply.destructionCountedFrom;
    const incomplete = unread || ended || destructionNotes.length > 0 || !!(cause.domain === 'land' && started && land);
    const finalUnknown = finalSpans.length > 0 || !!(cause.domain === 'land' && landNote(reply, window.first, window.end));
    const noAmount = !cause.leaves.some(leaf => pointAt(leaf, window.hour));
    const note = [incomplete && !noAmount ? 'Recorded subtotal; total Not known.' : undefined,
      cause.domain === 'land' ? land : undefined, ...destructionNotes,
      unread ? 'This cause was not read at the moment shown; its recorded amount may lag.' : undefined,
      ended ? 'An exchange ended after its last read; losses since that read are not known.' : undefined].filter(Boolean).join(' ');
    const noKnownAmount = noAmount && incomplete;
    items.push({ path: causePath, value: noKnownAmount ? null : 0, exact: 0n, final: 0,
      unknownValue: noKnownAmount, incomplete, finalUnknown, note: note || undefined, present: started || spans.length > 0 });
    for (const leaf of cause.leaves) {
      const point = pointAt(leaf, window.hour);
      const design = leaf.design === null ? undefined : reply.designs[leaf.design];
      const exact = lossAmount(leaf, window.hour, window.from), final = lossAmount(leaf, window.end, window.from);
      const path = [...causePath, { id: `type:${design?.archetype ?? '?unknown'}`, label: typeName(design) },
        { id: `design:${design?.variant ?? 'none'}`, label: lossDesignName(design, window.hour), keepZero: true }];
      items.push({ path, value: exact === null ? null : Number(exact), exact, final: final === null ? null : Number(final),
        weight: weightAt(leaf, window.end, window.from), present: point !== null,
        incomplete: unread || cause.source.kind === 'destruction' && incomplete || !!(cause.domain === 'land' && land)
          || leaf.lowerBoundFrom !== null && leaf.lowerBoundFrom <= window.hour,
        finalUnknown: finalSpans.some(span => span.reason === 'unread') || cause.source.kind === 'destruction' && finalUnknown
          || !!(cause.domain === 'land' && landNote(reply, window.first, window.end))
          || leaf.lowerBoundFrom !== null && leaf.lowerBoundFrom <= window.end,
        note: !ended || unread || cause.source.kind === 'destruction' || !!land
          || leaf.lowerBoundFrom !== null && leaf.lowerBoundFrom <= window.hour ? note || undefined : undefined,
        counted: [lossKey(cause, design)] });
    }
  }
  return items;
}

const leafCatalogues = new WeakMap<LossesReply, Map<string, { cause: LossCause; leaf: LossLeaf }>>();
function leafCatalogue(reply: LossesReply) {
  let catalogue = leafCatalogues.get(reply);
  if (!catalogue) {
    catalogue = new Map();
    for (const cause of reply.causes) if (cause.loss) for (const leaf of cause.leaves)
      catalogue.set(lossKey(cause, leaf.design === null ? undefined : reply.designs[leaf.design]), { cause, leaf });
    leafCatalogues.set(reply, catalogue);
  }
  return catalogue;
}

/** Supplying dates and item-only label/exchange notes do not spread to ancestors. */
export function lossDetails(node: SunburstNode, reply: LossesReply, hour: number): TooltipRow[] {
  let first: number | null = null, last: number | null = null;
  const rows: TooltipRow[] = [];
  const cause = node.key.startsWith('cause:') ? reply.causes.find(cause => node.key === `cause:${sourceKey(cause.source)}`) : undefined;
  if (cause) {
    const template = cause.name.template;
    if (cause.name.text === null) rows.push({ label: 'Cause label', value: 'The content supplies no fully resolved cause label; its template or identifier is shown verbatim.', tone: 'muted' });
    else if (template && template !== cause.name.text && !template.includes('$')) rows.push({ label: 'Game text', value: template, tone: 'muted' });
  }
  const catalogue = leafCatalogue(reply);
  for (const key of node.counted) {
    const entry = catalogue.get(key);
    if (!entry) continue;
    const { cause, leaf } = entry;
    const point = pointAt(leaf, hour);
    if (!point) continue;
    first = first === null ? point.hour : Math.min(first, point.hour);
    last = last === null ? point.hour : Math.max(last, point.hour);
    if (node.key.startsWith('design:') && cause.source.kind !== 'destruction' && leaf.lowerBoundFrom !== null && leaf.lowerBoundFrom <= hour)
      rows.push({ label: 'Incomplete from', value: `${formatDateTime(leaf.lowerBoundFrom)}: an exchange ended after its last read.`, tone: 'muted' });
  }
  if (first !== null && last !== null) rows.unshift({ label: first === last ? 'Last recorded change' : 'Last changes of included amounts',
    value: first === last ? formatDateTime(last) : `${formatDateTime(first)} - ${formatDateTime(last)}`, tone: 'muted' });
  return rows;
}

/** Preserve every recorded fixed-point digit, including amounts beyond 2^53. */
export function formatLosses(value: number, exact: bigint | null, scale: number): string {
  const raw = exact ?? BigInt(Math.round(value));
  const amount = formatAmount(raw, Math.round(Math.log10(scale)), Math.round(Math.log10(scale)));
  return raw < 0n ? `Correction: ${amount} (the game returned equipment)` : amount;
}

export function lossesFootnote(reply: LossesReply, window: LossWindow): string {
  return [reply.landSource === 'combatLog' ? landNote(reply, window.first, window.hour) : undefined, 'Recorded signed net amounts only; incomplete amounts are subtotals and their full totals are Not known. Negative corrections mean the game returned equipment and have no arc.',
    destructionCountingNote(reply, window.hour),
    window.from != null ? 'The range counts signed changes at their supplying reads; the first retained Combat Log read can include losses before its read date.' : undefined,
    reply.landSource === 'destruction' ? undefined
      : reply.retainedMonths === null ? 'Land retention is not known.' : reply.retainedMonths > 0
      ? `The first land read can include up to ${reply.retainedMonths} months retained by the game.`
      : 'Land retention prevents a known total; see the cause reasons.'].filter(Boolean).join(' ');
}

/** Independently counted subtotal: missing cause amounts leave the full signed total unknown. */
export function lossesTotal(items: readonly LossItem[]): LossTotal {
  let value = 0;
  let exact = 0n;
  let final = 0;
  let incomplete = false;
  let finalUnknown = false;
  let measured = false;
  for (const item of items) {
    final += item.final ?? 0;
    finalUnknown ||= item.finalUnknown === true || item.final === null;
    if (item.present === false) continue;
    measured ||= item.value !== null;
    value += item.value ?? 0;
    exact += item.exact ?? 0n;
    incomplete ||= item.incomplete === true || item.value === null;
  }
  return { value: measured ? value : null, exact: measured ? exact : null, final, finalUnknown, incomplete, unknownValue: !measured,
    note: incomplete ? 'Recorded subtotal; the full signed total is Not known.' : undefined };
}

/** Counting-start wording lives here so product wording changes need one edit. */
export function destructionCountingNote(reply: LossesReply, hour: number): string {
  if (reply.destructionCountedFrom === null) {
    const reasons = [...new Set(reply.destructionCoverage.flatMap(span => span.reason === null ? [] : [span.reason]))];
    if (reasons.length) return `Destruction was not recorded on this branch: ${reasons.join(' ')}`;
  }
  return reply.destructionCountedFrom === null || hour < reply.destructionCountedFrom
    ? 'Destruction counting has not started at this moment.'
    : `Destruction counted from ${formatDateTime(reply.destructionCountedFrom)}.`;
}
