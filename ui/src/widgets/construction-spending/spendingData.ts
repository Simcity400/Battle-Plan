import type { ConstructionSpendingReply, SpendingLeaf, SpentStep } from '../../generated/dashboard';
import { isBuildingInfo, isSpendingGap, isStateName, placeName, typeName } from '../../dashboard/construction.ts';
import type { StateNameAt } from '../../dashboard/construction';
import type { SunburstItem } from '../../charts/sunburst/layout';
import { formatIc } from '../../dashboard/icDecimals.ts';
import { preparedPrefix } from '../../dashboard/preparedPrefix.ts';


/** The hierarchy's levels, innermost first, and the option that switches each off. */
export const LEVELS = ['splitLevel', 'typeLevel', 'stateLevel'] as const;

// A total adds i64 amounts in i128 (up to 39 digits).
const RAW = /^-?\d{1,39}$/;
const int = (value: unknown) => Number.isSafeInteger(value);
const hours = (list: unknown) => Array.isArray(list) && list.every(int) && list.every((hour, index) => index === 0 || (list[index - 1] as number) < (hour as number));
const isStep = (step: SpentStep) => typeof step === 'object' && step != null && int(step.knownAt) && typeof step.totalRaw === 'string' && RAW.test(step.totalRaw);
const isLeaf = (leaf: SpendingLeaf, types: number) => typeof leaf === 'object' && leaf != null && typeof leaf.repair === 'boolean'
  && (leaf.kind === 'building' || leaf.kind === 'railway') && int(leaf.building) && leaf.building >= 0 && leaf.building < types
  && [leaf.state, leaf.toState].every(value => value == null || int(value))
  && Array.isArray(leaf.spent) && leaf.spent.every(isStep) && leaf.spent.every((step, index) => index === 0 || leaf.spent[index - 1].knownAt < step.knownAt)
  && Array.isArray(leaf.unknown) && leaf.unknown.every(gap => typeof gap === 'object' && gap != null && int(gap.knownAt) && isSpendingGap(gap.reason) && typeof gap.project === 'string' && (gap.until == null || (int(gap.until) && gap.until > gap.knownAt)))
  && Array.isArray(leaf.projects) && leaf.projects.length > 0 && leaf.projects.every((project, index) => typeof project === 'object' && project != null
    && typeof project.project === 'string' && int(project.firstRead) && (project.until == null || (int(project.until) && project.until > project.firstRead))
    && (project.knownAt == null || (int(project.knownAt) && project.knownAt >= project.firstRead))
    && typeof project.unread === 'boolean' && (!project.unread || project.until != null)
    && (project.resumes == null || (project.unread && int(project.resumes) && project.until != null && project.resumes > project.until))
    && (index === 0 || leaf.projects[index - 1].firstRead <= project.firstRead));

/** Checks the reply's shape: each leaf names a listed type, its totals are exact
 * integers in hour order, including recorded repair output. */
export function validateSpendingReply(data: unknown): ConstructionSpendingReply {
  const reply = data as ConstructionSpendingReply;
  const ok = typeof reply === 'object' && reply != null && typeof reply.branchId === 'string' && typeof reply.tag === 'string'
    && int(reply.costScale) && reply.costScale > 0 && hours(reply.reads) && hours(reply.railwayReads)
    && Array.isArray(reply.types) && reply.types.every(isBuildingInfo) && (reply.repairIcon == null || typeof reply.repairIcon === 'string')
    && Array.isArray(reply.states) && reply.states.every(isStateName) && int(reply.beforeFirstRead) && reply.beforeFirstRead >= 0
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string')
    && Array.isArray(reply.unattributed) && reply.unattributed.every(gap => int(gap.knownAt) && isSpendingGap(gap.reason) && typeof gap.project === 'string' && (gap.until == null || (int(gap.until) && gap.until > gap.knownAt)))
    && Array.isArray(reply.leaves) && reply.leaves.every(leaf => isLeaf(leaf, reply.types.length));
  if (!ok) throw new Error('The construction spending reply is malformed.');
  preparedPrefix(reply);
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

/** A leaf's running total established by `hour` (fixed5), exact. */
export const totalAt = (leaf: SpendingLeaf, hour: number): bigint => BigInt(stepAt(leaf.spent, hour)?.totalRaw ?? '0');

/** Projects whose recorded work overlaps the requested window. Output-only
 * work has its own supplying-read interval, so natural repairs count too. */
function projectsIn(leaf: SpendingLeaf, from: number | null, to: number): string[] {
  return [...new Set(leaf.projects.filter(span => Math.max(span.firstRead, span.knownAt ?? span.firstRead) <= to
    && (from == null || span.until == null || span.until > from
      || (span.unread && (span.resumes == null || span.resumes > from)))).map(span => span.project))];
}

/** The hours a sunburst adds up: from `from` (null: from the branch's start) to
 * `hour`, with `end` the last hour of what is shown (Fill growth sweeps toward it). */
export interface SpendingWindow { from: number | null; hour: number; end: number }

/** The sunburst's items, one per leaf: its path new or repair → building type (from
 * content) → state; its amount by the hour shown, exactly (`exact`, fixed5) and as a
 * number for geometry; its whole amount at the end of what is shown; and what is not
 * counted by reason. Output not counted leaves the counted amount shown and partial,
 * never hidden. A leaf none of whose projects was in the queue during what is
 * shown (established by its end, and not gone before its start) is not present: it adds only its whole amount, so Fill's whole, the order and the colours
 * are those of the end from the start. Repairs use their own recorded factory channel.
 * Output no country can be given (no census owner, or output not read) makes only the
 * whole partial: it is an item without a path, so it has no slice or legend row. */
export function spendingItems(reply: ConstructionSpendingReply, window: SpendingWindow, stateAt: StateNameAt): SunburstItem[] {
  const items: SunburstItem[] = [];
  const before = window.from == null ? null : window.from - 1;
  for (const leaf of reply.leaves) {
    const unknown: Record<string, number> = {};
    for (const gap of leaf.unknown) {
      if (gap.knownAt > window.hour || (gap.until != null && gap.until <= window.hour)) continue;
      unknown[gap.reason] = (unknown[gap.reason] ?? 0) + 1;
    }
    const projects = projectsIn(leaf, window.from, window.hour);
    const started = projects.length;
    const type = reply.types[leaf.building];
    const railway = leaf.kind === 'railway';
    const place = placeName({ state: leaf.state, toState: leaf.toState, railway }, window.hour, stateAt);
    const path = [
      leaf.repair ? { id: 'repair', label: 'Repairs' } : { id: 'new', label: 'New buildings' },
      { id: `type:${type?.id ?? leaf.building}`, label: typeName(type), icon: type?.icon ?? null },
      // A railway within one state is that state's, as a building there is.
      { id: `place:${leaf.state ?? ''}>${railway && leaf.toState !== leaf.state ? leaf.toState ?? '' : ''}`, label: place.label },
    ];
    const since = (hour: number) => before == null ? totalAt(leaf, hour) : totalAt(leaf, hour) - totalAt(leaf, before);
    const exact = since(window.hour);
    const finalUnknown = leaf.unknown.some(gap => gap.knownAt <= window.end && (gap.until == null || gap.until > window.end));
    items.push({
      path,
      value: Number(exact),
      exact,
      final: Number(since(window.end)),
      finalUnknown,
      // Present when a project worked in what is shown, or the reads in it
      // established an amount or a stretch not counted.
      present: started > 0 || (exact != null && exact !== 0n) || Object.keys(unknown).length > 0,
      unknown,
      counted: projects,
      note: place.missing,
    });
  }
  const applies = (gap: ConstructionSpendingReply['unattributed'][number], hour: number) =>
    gap.knownAt <= hour && (gap.reason === 'ownerUnknown'
      ? window.from == null || gap.until == null || gap.until > window.from
      : gap.until == null || gap.until > hour);
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

/** Decimal places of a cost scale (fixed5: 5). */
export const scaleDigits = (scale: number) => Math.round(Math.log10(scale));

/** An amount of output (fixed5 at `scale`) written to `digits` places. */
export const formatSpent = (value: number, exact: bigint | null, scale: number, digits: number) =>
  formatIc(value, exact, scaleDigits(scale), digits);

const plural = (count: number, one: string) => `${count} ${one}${count === 1 ? '' : 's'}`;

/** The note under the legend, saying plainly what the amounts leave out: earlier
 * spending, and in what is shown, output not recorded exactly (partial amounts) and
 * output no country can be given. `prepared`, for a reply answered from a prepared
 * prefix before the hour shown, says through when it is spent; it replaces the
 * partial amounts' note about output still being prepared. */
export function footnote(reply: ConstructionSpendingReply, items: readonly SunburstItem[], prepared?: string): string | undefined {
  const notes: string[] = [];
  if (reply.beforeFirstRead) notes.push(`Earlier spending on ${plural(reply.beforeFirstRead, 'project')} isn’t counted.`);
  if (prepared) notes.push(prepared);
  const shown = items.filter(item => item.present !== false);
  const partial = new Set(shown.flatMap(item => item.path.length > 0 ? Object.keys(item.unknown ?? {}) : []));
  if ([...partial].some(reason => reason !== 'preparing'))
    notes.push('Partial amounts leave out output that wasn’t recorded exactly.');
  if (partial.has('preparing') && !prepared) notes.push('Partial amounts leave out output still being prepared.');
  const unowned: Record<string, number> = {};
  for (const item of shown) if (item.path.length === 0) for (const [reason, count] of Object.entries(item.unknown ?? {})) unowned[reason] = (unowned[reason] ?? 0) + count;
  const { ownerUnknown = 0, preparing = 0, ...unread } = unowned;
  if (ownerUnknown) notes.push(`Output of ${plural(ownerUnknown, 'project')} not traced to a country isn’t counted.`);
  if (preparing) notes.push(`Output of ${plural(preparing, 'project')} is still being prepared.`);
  if (Object.keys(unread).length) notes.push('Output in hours that weren’t recorded isn’t counted.');
  return notes.length ? notes.join(' ') : undefined;
}
