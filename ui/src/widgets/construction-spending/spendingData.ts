import { isConstructionType, isMissingSpan, isSpendingGap, isStateName, placeName, stateNamesOf, typeName } from '../../dashboard/construction.ts';
import type { SunburstItem } from '../../charts/sunburst/layout';
import { formatIc } from '../../dashboard/icDecimals.ts';
import type { ConstructionSpendingReply, SpendingLeaf, SpendingUnknown, SpentStep } from '../../generated/dashboard';

/** The hierarchy's levels, innermost first, and the option that switches each off. */
export const LEVELS = ['splitLevel', 'typeLevel', 'stateLevel'] as const;

// A total adds i64 amounts in i128 (up to 39 digits).
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

/** Checks the reply's shape: its totals exact integers in hour order, each gap a reason the recording gives. */
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

/** The last step at or before `hour`, by binary search; null before the first. */
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

/** A leaf's running total established by `hour` (fixed5), exact: zero before its first output, null where the
 * recording does not hold the total. */
export function totalAt(leaf: SpendingLeaf, hour: number): bigint | null {
  const step = stepAt(leaf.spent, hour);
  if (!step) return 0n;
  return step.totalRaw === null ? null : BigInt(step.totalRaw);
}

/** Lines whose recorded life overlaps the requested window. */
function projectsIn(leaf: SpendingLeaf, from: number | null, to: number): string[] {
  return [...new Set(leaf.projects.filter(span => span.firstRead <= to && (from == null || span.until == null || span.until > from)).map(span => span.project))];
}

/** The hours a sunburst adds up: from `from` (null: from the branch's start) to `hour`, with `end` the last hour of
 * what is shown (Fill growth sweeps toward it). */
export interface SpendingWindow { from: number | null; hour: number; end: number }

/** The sunburst's items, one per leaf: its path new or repair → building type (from content) → state; its amount by
 * the hour shown, exactly (`exact`, fixed5) and as a number for geometry; its whole amount at the end of what is
 * shown; and what is not counted by reason. Output not counted leaves the counted amount shown and partial, never
 * hidden; a total the recording does not hold is not measured, never zero. A leaf none of whose lines was in the
 * queue during what is shown is not present: it adds only its whole amount, so Fill's whole, the order and the
 * colours are those of the end from the start. Output no country can be given makes only the whole partial: it is
 * an item without a path, so it has no slice or legend row. */
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
      // A railway within one state is that state's, as a building there is.
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
      // Present when a line worked in what is shown, or the reads in it established an amount or a stretch not counted.
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

/** Whether a country's own construction shows: a leaf present in what is shown. */
export const hasConstruction = (items: readonly SunburstItem[]) => items.some(item => item.path.length > 0 && item.present !== false);

/** Which leaves are present: what a render shows, as opposed to the amounts painted at each frame. */
export const presentKey = (items: readonly SunburstItem[]) => items.map(item => item.present === false ? '0' : '1').join('');

/** Decimal places of a cost scale (fixed5: 5). */
export const scaleDigits = (scale: number) => Math.round(Math.log10(scale));

/** An amount of output (fixed5 at `scale`) written to `digits` places. */
export const formatSpent = (value: number, exact: bigint | null, scale: number, digits: number) =>
  formatIc(value, exact, scaleDigits(scale), digits);

const plural = (count: number, one: string) => `${count} ${one}${count === 1 ? '' : 's'}`;

/** The note under the legend, saying plainly what the amounts leave out: earlier spending, output not recorded
 * exactly (partial amounts), output no country can be given, and hours the recording does not hold. */
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
