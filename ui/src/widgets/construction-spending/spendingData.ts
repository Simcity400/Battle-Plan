import type { ConstructionSpendingReply, SpendingGap, SpendingLeaf, SpentStep } from '../../generated/dashboard';
import { isBuildingInfo, isStateName, placeName, typeName } from '../../dashboard/construction.ts';
import type { SunburstItem } from '../../charts/sunburst/layout';
import { formatAmount } from '../../charts/valueAxis.ts';

/** Why a stretch of a project's progress is not counted, as the tooltip names it. */
export const GAP_LABELS: Readonly<Record<SpendingGap, string>> = {
  notRecorded: 'Hours not recorded',
  readFailed: 'A queue read failed',
  listedLate: 'Listed after it began',
  builtBeforeRead: 'Its building changed before its first read',
  sharedTarget: 'Shared with another project',
  levelsUnaccounted: 'Levels lost, built or dropped',
  unclaimedRise: 'Its building rose without the line showing it',
  costChanged: 'Its cost changed',
  repairing: 'Repair work on it',
  unexplained: 'Not explained by its reads',
  leftUnfinished: 'After it left unfinished',
  endUnknown: 'After it left the queue',
  finishedAfterRead: 'Finished after its last read',
};

/** Why repairs have no amount (the product's notes give the evidence). */
export const REPAIR_NOTE = 'Repairs are not measured: the game keeps a repair’s progress on the building, part of it free, and the recording does not show the factory output each day’s repair took.';

/** The hierarchy's levels, innermost first, and the option that switches each off. */
export const LEVELS = ['splitLevel', 'typeLevel', 'stateLevel'] as const;

// A total adds i64 amounts in i128 (up to 39 digits).
const RAW = /^-?\d{1,39}$/;
const int = (value: unknown) => Number.isSafeInteger(value);
const hours = (list: unknown) => Array.isArray(list) && list.every(int) && list.every((hour, index) => index === 0 || (list[index - 1] as number) < (hour as number));
const isStep = (step: SpentStep) => typeof step === 'object' && step != null && int(step.knownAt) && RAW.test(step.totalRaw);
const isLeaf = (leaf: SpendingLeaf, types: number) => typeof leaf === 'object' && leaf != null && typeof leaf.repair === 'boolean'
  && (leaf.kind === 'building' || leaf.kind === 'railway') && int(leaf.building) && leaf.building >= 0 && leaf.building < types
  && [leaf.state, leaf.toState].every(value => value == null || int(value))
  && Array.isArray(leaf.spent) && leaf.spent.every(isStep) && leaf.spent.every((step, index) => index === 0 || leaf.spent[index - 1].knownAt < step.knownAt)
  && (!leaf.repair || leaf.spent.length === 0)
  && Array.isArray(leaf.unknown) && leaf.unknown.every(gap => typeof gap === 'object' && gap != null && int(gap.knownAt) && Object.hasOwn(GAP_LABELS, gap.reason) && typeof gap.project === 'string')
  && Array.isArray(leaf.projects) && leaf.projects.length > 0 && leaf.projects.every((project, index) => typeof project === 'object' && project != null
    && typeof project.project === 'string' && int(project.firstRead) && (project.until == null || (int(project.until) && project.until > project.firstRead))
    && (project.knownAt == null || (int(project.knownAt) && project.knownAt >= project.firstRead))
    && typeof project.unread === 'boolean' && (!project.unread || project.until != null)
    && (project.resumes == null || (project.unread && int(project.resumes) && project.until != null && project.resumes > project.until))
    && (index === 0 || leaf.projects[index - 1].firstRead <= project.firstRead));

/** Checks the reply's shape: each leaf names a listed type, its totals are exact
 * integers in hour order, and a repair leaf has no amount. */
export function validateSpendingReply(data: unknown): ConstructionSpendingReply {
  const reply = data as ConstructionSpendingReply;
  const ok = typeof reply === 'object' && reply != null && typeof reply.branchId === 'string' && typeof reply.tag === 'string'
    && int(reply.costScale) && reply.costScale > 0 && hours(reply.reads) && hours(reply.railwayReads)
    && Array.isArray(reply.types) && reply.types.every(isBuildingInfo) && (reply.repairIcon == null || typeof reply.repairIcon === 'string')
    && Array.isArray(reply.states) && reply.states.every(isStateName) && int(reply.beforeFirstRead) && reply.beforeFirstRead >= 0
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string')
    && Array.isArray(reply.leaves) && reply.leaves.every(leaf => isLeaf(leaf, reply.types.length));
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

/** A leaf's running total established by `hour` (fixed5), exact. */
export const totalAt = (leaf: SpendingLeaf, hour: number): bigint => BigInt(stepAt(leaf.spent, hour)?.totalRaw ?? '0');

/** The projects of a leaf that did its kind of work at some hour of [from, to]
 * (an interval established by `to` and not ended by `from`), by id, and how many of its
 * intervals broke off for hours not recorded inside [from, to] (to where its reads
 * resume, or open-ended when they never do): what a project did then is not known,
 * never nothing. A break closed by an uncounted stretch already counted
 * (`counted`: project and read) is that stretch, not another. */
function projectsIn(leaf: SpendingLeaf, from: number | null, to: number, counted: ReadonlySet<string>): { ids: string[]; unread: number } {
  const ids = new Set<string>();
  let unread = 0;
  for (const span of leaf.projects) {
    if (Math.max(span.firstRead, span.knownAt ?? span.firstRead) > to) continue;
    if (from == null || span.until == null || span.until > from) ids.add(span.project);
    if (!span.unread || span.until == null || span.until > to) continue;
    if (span.resumes != null && from != null && span.resumes <= from) continue;
    ids.add(span.project);
    if (span.resumes == null || !counted.has(`${span.project}@${span.resumes}`)) unread++;
  }
  return { ids: [...ids], unread };
}

/** The hours a sunburst adds up: from `from` (null: from the branch's start) to
 * `hour`, with `end` the last hour of what is shown (Fill growth sweeps toward it). */
export interface SpendingWindow { from: number | null; hour: number; end: number }

/** The sunburst's items, one per leaf: its path new or repair → building type (from
 * content) → state; its amount by the hour shown, exactly (`exact`, fixed5) and as a
 * number for geometry; its whole amount at the end of what is shown; and what is not
 * counted by reason. A leaf none of whose projects was in the queue during what is
 * shown (established by its end, and not gone before its start) is not present: it adds only its whole amount, so Fill's whole, the order and the colours
 * are those of the end from the start. A repair has no amount, and says why. */
export function spendingItems(reply: ConstructionSpendingReply, window: SpendingWindow): SunburstItem[] {
  const items: SunburstItem[] = [];
  const before = window.from == null ? null : window.from - 1;
  for (const leaf of reply.leaves) {
    const unknown: Record<string, number> = {};
    // The unread stretches this range counts, by project and closing read.
    const counted = new Set<string>();
    for (const gap of leaf.unknown) {
      if (gap.knownAt > window.hour || (window.from != null && gap.knownAt < window.from)) continue;
      unknown[gap.reason] = (unknown[gap.reason] ?? 0) + 1;
      if (gap.reason === 'notRecorded' || gap.reason === 'readFailed') counted.add(`${gap.project}@${gap.knownAt}`);
    }
    const { ids: projects, unread } = projectsIn(leaf, window.from, window.hour, counted);
    const started = projects.length;
    const type = reply.types[leaf.building];
    const railway = leaf.kind === 'railway';
    const place = placeName(reply.states, { state: leaf.state, toState: leaf.toState, railway });
    const path = [
      leaf.repair ? { id: 'repair', label: 'Repairs' } : { id: 'new', label: 'New buildings' },
      { id: `type:${type?.token ?? type?.id ?? leaf.building}`, label: typeName(type), icon: type?.icon ?? null },
      // A railway within one state is that state's, as a building there is.
      { id: `place:${leaf.state ?? ''}>${railway && leaf.toState !== leaf.state ? leaf.toState ?? '' : ''}`, label: place },
    ];
    const since = (hour: number) => before == null ? totalAt(leaf, hour) : totalAt(leaf, hour) - totalAt(leaf, before);
    if (unread) unknown.notRecorded = (unknown.notRecorded ?? 0) + unread;
    const exact = leaf.repair ? null : since(window.hour);
    items.push({
      path,
      value: exact == null ? null : Number(exact),
      exact,
      final: leaf.repair ? null : Number(since(window.end)),
      // Present when a project worked in what is shown, or the reads in it
      // established an amount or a stretch not counted.
      present: started > 0 || (exact != null && exact !== 0n) || Object.keys(unknown).length > 0,
      unknown,
      note: leaf.repair ? REPAIR_NOTE : undefined,
      counted: projects,
    });
  }
  return items;
}

/** An amount of output as written, from fixed5, two decimals at most: from the exact
 * integer when there is one (a number loses digits past 2^53), else the number. */
export const formatSpent = (value: number, exact: bigint | null, scale: number) =>
  formatAmount(exact ?? BigInt(Math.round(value)), Math.round(Math.log10(scale)));

/** The note under the legend: what the totals leave out before the queue's first read. */
export function footnote(reply: ConstructionSpendingReply): string | undefined {
  const count = reply.beforeFirstRead;
  if (!count) return undefined;
  return `${count} ${count === 1 ? 'project was' : 'projects were'} already under way at the queue’s first read; progress before it is not counted.`;
}
