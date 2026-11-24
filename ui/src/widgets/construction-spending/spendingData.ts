import { isConstructionType, isMissingSpan, isSpendingGap, isStateName, placeName, stateNamesOf, typeName } from '../../dashboard/construction.ts';
import type { BreakdownLeafItem as SunburstItem } from '../../families/breakdown/tree';
import { formatIc } from '../../dashboard/icDecimals.ts';
import type { ConstructionSpendingReply, SpendingLeaf, SpendingUnknown, SpentStep } from '../../generated/dashboard';

export const LEVELS = ['splitLevel', 'typeLevel', 'stateLevel'] as const;

const RAW = /^-?\d{1,39}$/;
const int = (value: unknown) => Number.isSafeInteger(value);
const optionalInt = (value: unknown) => value == null || int(value);
const isStep = (step: SpentStep) => typeof step === 'object' && step != null && int(step.knownAt) && (step.totalRaw === null || (typeof step.totalRaw === 'string' && RAW.test(step.totalRaw)));
const isGap = (gap: SpendingUnknown) => typeof gap === 'object' && gap != null && int(gap.knownAt) && isSpendingGap(gap.reason) && typeof gap.project === 'string'
  && (gap.until == null || (int(gap.until) && gap.until > gap.knownAt));
const isLeaf = (leaf: SpendingLeaf) => typeof leaf === 'object' && leaf != null && typeof leaf.repair === 'boolean'
  && (leaf.kind === 'building' || leaf.kind === 'railway') && [leaf.building, leaf.state, leaf.toState].every(optionalInt)
  && Array.isArray(leaf.spent) && leaf.spent.every(isStep) && leaf.spent.every((step, index) => index === 0 || leaf.spent[index - 1].knownAt < step.knownAt)
  && Array.isArray(leaf.unknown) && leaf.unknown.every(isGap)
  && Array.isArray(leaf.projects) && leaf.projects.every((project, index) => typeof project === 'object' && project != null
    && typeof project.project === 'string' && int(project.firstRead) && (project.until == null || (int(project.until) && project.until > project.firstRead))
    && (index === 0 || leaf.projects[index - 1].firstRead <= project.firstRead));

export function validateSpendingReply(data: unknown): ConstructionSpendingReply {
  const reply = data as ConstructionSpendingReply;
  const ok = typeof reply === 'object' && reply != null && typeof reply.branchId === 'string' && typeof reply.tag === 'string'
    && int(reply.costScale) && reply.costScale > 0 && typeof reply.recorded === 'boolean'
    && Array.isArray(reply.types) && reply.types.every(isConstructionType) && (reply.repairIcon == null || typeof reply.repairIcon === 'string')
    && Array.isArray(reply.states) && reply.states.every(isStateName) && int(reply.beforeFirstRead) && reply.beforeFirstRead >= 0
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string')
    && Array.isArray(reply.unattributed) && reply.unattributed.every(isGap)
    && Array.isArray(reply.missing) && reply.missing.every(isMissingSpan)
    && Array.isArray(reply.leaves) && reply.leaves.every(isLeaf);
  if (!ok) throw new Error('The construction spending reply is malformed.');
  return reply;
}

function stepAt(steps: readonly SpentStep[], hour: number): SpentStep | null {
  let low = 0;
  let high = steps.length - 1;
  let found: SpentStep | null = null;
  while (low <= high) {
    const middle = (low + high) >> 1;
    if (steps[middle].knownAt <= hour) { found = steps[middle]; low = middle + 1; } else high = middle - 1;
  }
  return found;
}

export function totalAt(leaf: SpendingLeaf, hour: number): bigint | null {
  const step = stepAt(leaf.spent, hour);
  if (!step) return 0n;
  return step.totalRaw === null ? null : BigInt(step.totalRaw);
}

function projectsIn(leaf: SpendingLeaf, from: number | null, to: number): string[] {
  return [...new Set(leaf.projects.filter(span => span.firstRead <= to && (from == null || span.until == null || span.until > from)).map(span => span.project))];
}

export interface SpendingWindow { from: number | null; hour: number; end: number }

export function spendingItems(reply: ConstructionSpendingReply, window: SpendingWindow): SunburstItem[] {
  const items: SunburstItem[] = [];
  const before = window.from == null ? null : window.from - 1;
  const types = new Map(reply.types.map(type => [type.id, type]));
  const stateAt = stateNamesOf(reply.states);
  for (const leaf of reply.leaves) {
    const unknown: Record<string, number> = {};
    for (const gap of leaf.unknown) {
      if (gap.knownAt > window.hour || (gap.until != null && gap.until <= window.hour)) continue;
      unknown[gap.reason] = (unknown[gap.reason] ?? 0) + 1;
    }
    const projects = projectsIn(leaf, window.from, window.hour);
    const type = leaf.building == null ? undefined : types.get(leaf.building);
    const railway = leaf.kind === 'railway';
    const place = placeName({ state: leaf.state, toState: leaf.toState, railway }, stateAt);
    const path = [
      leaf.repair ? { id: 'repair', label: 'Repairs' } : { id: 'new', label: 'New buildings' },
      { id: `type:${type?.id ?? leaf.building ?? 'none'}`, label: typeName(type), icon: type?.icon ?? null },
      { id: `place:${leaf.state ?? ''}>${railway && leaf.toState !== leaf.state ? leaf.toState ?? '' : ''}`, label: place.label },
    ];
    const since = (hour: number) => {
      const now = totalAt(leaf, hour);
      if (before == null || now == null) return now;
      const start = totalAt(leaf, before);
      return start == null ? null : now - start;
    };
    const exact = since(window.hour);
    const final = since(window.end);
    if (exact == null) unknown.notRecorded = (unknown.notRecorded ?? 0) + 1;
    const finalUnknown = final == null || leaf.unknown.some(gap => gap.knownAt <= window.end && (gap.until == null || gap.until > window.end));
    items.push({
      path,
      value: exact == null ? null : Number(exact),
      exact,
      unknownValue: exact == null,
      final: final == null ? null : Number(final),
      finalUnknown,
      present: projects.length > 0 || (exact != null && exact !== 0n) || Object.keys(unknown).length > 0,
      unknown,
      counted: projects,
      note: place.missing,
    });
  }
  const applies = (gap: SpendingUnknown, hour: number) =>
    gap.knownAt <= hour && (window.from == null || gap.until == null || gap.until > window.from);
  const unowned = reply.unattributed.filter(gap => applies(gap, window.hour));
  if (reply.unattributed.length) {
    const unknown: Record<string, number> = {};
    for (const gap of unowned) unknown[gap.reason] = (unknown[gap.reason] ?? 0) + 1;
    items.push({
      path: [], value: 0, exact: 0n, final: 0, finalUnknown: reply.unattributed.some(gap => applies(gap, window.end)),
      present: unowned.length > 0, unknown,
    });
  }
  return items;
}

export const hasConstruction = (items: readonly SunburstItem[]) => items.some(item => item.path.length > 0 && item.present !== false);

export const presentKey = (items: readonly SunburstItem[]) => items.map(item => item.present === false ? '0' : '1').join('');

export const scaleDigits = (scale: number) => Math.round(Math.log10(scale));

export const formatSpent = (value: number, exact: bigint | null, scale: number, digits: number) =>
  formatIc(value, exact, scaleDigits(scale), digits);

const plural = (count: number, one: string) => `${count} ${one}${count === 1 ? '' : 's'}`;

export function footnote(reply: ConstructionSpendingReply, items: readonly SunburstItem[], window: SpendingWindow): string | undefined {
  const notes: string[] = [];
  if (reply.beforeFirstRead) notes.push(`Earlier spending on ${plural(reply.beforeFirstRead, 'project')} isn’t counted.`);
  const shown = items.filter(item => item.present !== false);
  if (shown.some(item => item.path.length > 0 && Object.keys(item.unknown ?? {}).length > 0))
    notes.push('Partial amounts leave out output that wasn’t recorded exactly.');
  let ownerUnknown = 0;
  for (const item of shown) if (item.path.length === 0) ownerUnknown += item.unknown?.ownerUnknown ?? 0;
  if (ownerUnknown) notes.push(`Output of ${plural(ownerUnknown, 'project')} not traced to a country isn’t counted.`);
  if (reply.missing.some(span => span.firstHour <= window.hour && (window.from == null || span.lastHour >= window.from)))
    notes.push('Output in hours that weren’t recorded isn’t counted.');
  return notes.length ? notes.join(' ') : undefined;
}
