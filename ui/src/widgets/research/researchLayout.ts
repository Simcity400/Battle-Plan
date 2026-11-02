import { failureText, type Failure } from '../../dashboard/queryError.ts';
import type { ResearchCoverage, ResearchEntry, ResearchHistory, ResearchSlotHistory, ResearchSlotState } from '../../generated/dashboard';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { formatDate, formatDateTime, gameDate, monthNames } from '../../dashboard/calendar.ts';
import { researchNumber } from './researchBreakdown.ts';
import { researchHeldRuns, researchLatestProgress } from './researchData.ts';
import type { ResearchFolderLook, ResearchHeldRun } from './researchData';
import { researchAssignments, researchStartKnown } from './researchJourney.ts';

/** One row of a slot's lane: a completion, a technology held without its
 * recorded occurrence, a stretch the recording lacks, or the slot opening or
 * closing. `hour` orders the lane. */
export type ResearchItem =
  | { kind: 'completed'; key: string; hour: number; slot: ResearchSlotHistory; entry: ResearchEntry }
  | { kind: 'held'; key: string; hour: number; slot: ResearchSlotHistory; run: ResearchHeldRun }
  | { kind: 'gap'; key: string; hour: number; lastHour: number }
  | { kind: 'opened'; key: string; hour: number; known: boolean }
  | { kind: 'closed'; key: string; hour: number };

/** A slot's lane. `offset` is how many rows down it starts: only a slot that
 * opened during the recording starts lower, at the row the other slots had
 * reached by then. */
export interface ResearchLane { slot: ResearchSlotHistory; appeared: number; offset: number; items: ResearchItem[] }

const RANK: Record<ResearchItem['kind'], number> = { opened: 0, completed: 1, held: 1, gap: 2, closed: 3 };

/** Research coverage known by `hour`. */
export function coverageAt(coverage: readonly ResearchCoverage[], hour: number): ResearchCoverage[] {
  return coverage.filter(span => span.firstHour <= hour).map(span => ({ firstHour: span.firstHour, lastHour: Math.min(span.lastHour, hour) }));
}

/** The stretches between recorded research coverage, first and last hour. */
export function coverageGaps(coverage: readonly ResearchCoverage[]): { firstHour: number; lastHour: number }[] {
  return coverage.slice(1).map((span, index) => ({ firstHour: coverage[index].lastHour + 1, lastHour: span.firstHour - 1 }))
    .filter(gap => gap.lastHour >= gap.firstHour);
}

/** One lane per slot, in slot order: its completions (or, for older
 * recordings, what it held) in order, the recording's gaps that fall inside its
 * sequence, and its opening and closing. `slots` and `coverage` are clipped at
 * `hour`; a technology still held at `hour` is not yet history. */
export function researchLanes(slots: readonly ResearchSlotHistory[], coverage: readonly ResearchCoverage[], hour: number): ResearchLane[] {
  const spanOf = (at: number) => coverage.findIndex(span => span.firstHour <= at && at <= span.lastHour);
  const gaps = coverageGaps(coverage).map(gap => ({ ...gap, before: gap.firstHour - 1, after: gap.lastHour + 1 }));
  const lanes: ResearchLane[] = slots.filter(slot => slot.availability.length > 0).map(slot => {
    const items: ResearchItem[] = [];
    for (const entry of slot.entries) if (entry.outcome === 'completed' && entry.endedByHour !== null)
      items.push({ kind: 'completed', key: `c:${entry.firstObservedHour}:${entry.token}`, hour: entry.endedByHour, slot, entry });
    for (const run of researchHeldRuns(slot)) if (run.lastHour < hour)
      items.push({ kind: 'held', key: `h:${run.firstHour}:${run.token}`, hour: run.endedByHour ?? run.lastHour, slot, run });
    slot.availability.forEach((span, index) => {
      const within = spanOf(span.firstHour);
      if (within >= 0 && coverage[within].firstHour < span.firstHour) {
        // The read before, in the same coverage, did not list this slot.
        items.push({ kind: 'opened', key: `o:${span.firstHour}`, hour: span.firstHour, known: true });
      } else if (within > 0) {
        // First listed after a gap: when it opened is not recorded, unless the
        // slot simply continues from before the gap.
        const before = index > 0 ? slot.availability[index - 1] : null;
        if (before?.lastHour !== coverage[within - 1].lastHour) items.push({ kind: 'opened', key: `o:${span.firstHour}`, hour: span.firstHour, known: false });
      }
      const end = spanOf(span.lastHour);
      if (span.lastHour < hour && end >= 0 && coverage[end].lastHour > span.lastHour)
        items.push({ kind: 'closed', key: `x:${span.lastHour}`, hour: span.lastHour + 1 });
    });
    for (const gap of gaps) if (slot.availability.some(span => span.lastHour === gap.before) && slot.availability.some(span => span.firstHour === gap.after))
      items.push({ kind: 'gap', key: `g:${gap.firstHour}`, hour: gap.firstHour, lastHour: gap.lastHour });
    items.sort((a, b) => a.hour - b.hour || RANK[a.kind] - RANK[b.kind]);
    return { slot, appeared: slot.availability[0].firstHour, offset: 0, items };
  });
  const order = [...lanes].sort((a, b) => a.appeared - b.appeared || a.slot.index - b.slot.index);
  for (const lane of order) for (const other of order) {
    if (other.appeared >= lane.appeared) break;
    lane.offset = Math.max(lane.offset, other.offset + other.items.filter(item => item.hour <= lane.appeared).length);
  }
  return lanes.sort((a, b) => a.slot.index - b.slot.index);
}

interface IndexedLane { source: ResearchLane; reveals: { hour: number; position: number; item: ResearchItem }[]; count: number; shown: ResearchLane }
const laneIndexes = new WeakMap<ResearchHistory, { lanes: IndexedLane[]; shown: ResearchLane[] }>();
function prefixAt<T>(rows: readonly T[], hour: number, at: (row: T) => number): number {
  let low = 0, high = rows.length;
  while (low < high) { const mid = (low + high) >>> 1; if (at(rows[mid]) <= hour) low = mid + 1; else high = mid; }
  return low;
}
function revealedAt(item: ResearchItem): number {
  return item.kind === 'completed' ? Math.max(item.hour, item.entry.firstObservedHour, item.entry.outcomeObservedHour ?? item.hour)
    : item.kind === 'held' ? item.run.lastHour + 1 : item.kind === 'gap' ? item.lastHour + 1 : item.hour;
}

/** Index the immutable history once. Playback selects only newly revealed rows;
 * unchanged lanes retain their rows and native keyboard identities. Slot reads
 * remain in the source history and are looked up at the displayed hour. */
export function researchLanesAt(history: ResearchHistory, hour: number): ResearchLane[] {
  if (Number.isNaN(hour)) return [];
  let index = laneIndexes.get(history);
  if (!index) {
    index = { lanes: researchLanes(history.slots, history.coverage, Infinity).map(source => ({ source, count: -1,
      shown: { ...source, offset: 0, items: [] }, reveals: source.items.map((item, position) => ({ item, position, hour: revealedAt(item) }))
        .sort((a, b) => a.hour - b.hour || a.position - b.position) })), shown: [] };
    laneIndexes.set(history, index);
  }
  const shown: ResearchLane[] = [];
  for (const lane of index.lanes) {
    if (lane.source.appeared > hour) continue;
    const count = prefixAt(lane.reveals, hour, row => row.hour);
    if (count !== lane.count) {
      lane.count = count;
      lane.shown = { ...lane.source, offset: 0,
        items: lane.reveals.slice(0, count).sort((a, b) => a.position - b.position).map(row => row.item) };
    }
    shown.push(lane.shown);
  }
  const order = [...shown].sort((a, b) => a.appeared - b.appeared || a.slot.index - b.slot.index);
  for (const lane of order) {
    let offset = 0;
    for (const other of order) {
      if (other.appeared >= lane.appeared) break;
      offset = Math.max(offset, other.offset + prefixAt(other.items, lane.appeared, item => item.hour));
    }
    if (lane.offset !== offset) {
      const next = { ...lane, offset }, at = shown.indexOf(lane);
      shown[at] = next; order[order.indexOf(lane)] = next;
      index.lanes.find(owner => owner.source.slot.index === lane.slot.index)!.shown = next;
    }
  }
  if (shown.length !== index.shown.length || shown.some((lane, at) => lane !== index!.shown[at])) index.shown = shown;
  return index.shown;
}

/** One List row: a completion, something held (older recordings), or a gap. */
export type ResearchRow =
  | { kind: 'completed'; key: string; hour: number; slot: ResearchSlotHistory; entry: ResearchEntry; token: string;
    started: { hour: number; known: boolean }; startSlot: number | null; tookHours: number | null }
  | { kind: 'held'; key: string; hour: number; slot: ResearchSlotHistory; run: ResearchHeldRun; token: string;
    startKnown: boolean; heldHours: number | null }
  | { kind: 'gap'; key: string; hour: number; lastHour: number };

/** A completion's start and time taken, as its explanation joins them: started
 * with its first assignment, and taken from then to the read at which it had
 * left the slot. Both are known only when its explanation knows the start;
 * otherwise it started by its first assignment and its time is unknown. */
export function completedRow(slots: readonly ResearchSlotHistory[], slot: ResearchSlotHistory, entry: ResearchEntry, hour: number): Extract<ResearchRow, { kind: 'completed' }> {
  const first = researchAssignments(slots, slot, entry)[0];
  const known = researchStartKnown(first.slot, first.entry);
  return { kind: 'completed', key: `c:${slot.index}:${entry.firstObservedHour}:${entry.token}`, hour, slot, entry, token: entry.token,
    started: { hour: first.entry.firstObservedHour, known }, startSlot: first.slot.index !== slot.index ? first.slot.index : null,
    tookHours: known ? hour - first.entry.firstObservedHour : null };
}

/** What a slot held without its recorded occurrence: its start is known when the
 * read before, in the same coverage, showed something else. */
export function heldRow(coverage: readonly ResearchCoverage[], slot: ResearchSlotHistory, run: ResearchHeldRun, hour: number): Extract<ResearchRow, { kind: 'held' }> {
  const startKnown = coverage.some(span => span.firstHour < run.firstHour && run.firstHour <= span.lastHour);
  return { kind: 'held', key: `h:${slot.index}:${run.firstHour}:${run.token}`, hour, slot, run, token: run.token, startKnown,
    heldHours: startKnown && run.endedByHour !== null ? run.endedByHour - run.firstHour : null };
}

/** Every lane's completions and held technologies with the recording's gaps,
 * in time order (the List sorts them itself). */
export function researchRows(lanes: readonly ResearchLane[], slots: readonly ResearchSlotHistory[], coverage: readonly ResearchCoverage[]): ResearchRow[] {
  const rows: ResearchRow[] = [];
  for (const lane of lanes) for (const item of lane.items) {
    if (item.kind === 'completed') rows.push(completedRow(slots, item.slot, item.entry, item.hour));
    else if (item.kind === 'held') rows.push(heldRow(coverage, item.slot, item.run, item.hour));
  }
  for (const gap of coverageGaps(coverage)) rows.push({ kind: 'gap', key: `g:${gap.firstHour}`, hour: gap.firstHour, lastHour: gap.lastHour });
  return rows.sort((a, b) => a.hour - b.hour || (a.kind === 'gap' ? 1 : 0) - (b.kind === 'gap' ? 1 : 0) || (a.key < b.key ? -1 : a.key > b.key ? 1 : 0));
}

/** A date in the app's calendar; an hour outside it reads "Unavailable date". */
export function researchDate(hour: number): string {
  const date = gameDate(hour);
  return date ? formatDate(date) : 'Unavailable date';
}

/** "1–3 Apr 1944": a stretch of days, written as briefly as its ends allow. */
export function researchDateRange(firstHour: number, lastHour: number): string {
  const first = gameDate(firstHour), last = gameDate(lastHour);
  if (!first || !last) return 'Unavailable dates';
  if (first.year === last.year && first.month === last.month && first.day === last.day) return formatDate(last);
  if (first.year === last.year && first.month === last.month) return `${first.day}–${formatDate(last)}`;
  if (first.year === last.year) return `${first.day} ${monthNames[first.month - 1].slice(0, 3)} – ${formatDate(last)}`;
  return `${formatDate(first)} – ${formatDate(last)}`;
}

/** Elapsed hours as days, written as the explanation writes them. */
export function researchDays(hours: number): string {
  const text = researchNumber(BigInt(hours) * 100_000n / 24n);
  return `${text} ${text === '1' ? 'day' : 'days'}`;
}

/** The plain date under a lane cell's name, and the List's Date column. A held
 * technology's date is when it had left the slot, never a completion. */
export function researchItemDate(item: Exclude<ResearchItem, { kind: 'gap' }> | Exclude<ResearchRow, { kind: 'gap' }>): string {
  switch (item.kind) {
    case 'completed': return researchDate(item.hour);
    case 'held': return item.run.endedByHour !== null ? `until ${researchDate(item.run.endedByHour)}` : `last seen ${researchDate(item.run.lastHour)}`;
    case 'opened': return item.known ? `Opened by ${researchDate(item.hour)}` : `Recorded from ${researchDate(item.hour)}`;
    case 'closed': return `Closed by ${researchDate(item.hour)}`;
  }
}

export interface ResearchText {
  /** Repeated technology definitions have distinct template indices. */
  name: (token: string, databaseIndex?: number | null) => string;
  icon: (token: string) => string | undefined;
  artworkFailure?: (token: string) => Failure | undefined;
  /** Each recorded definition can belong to different folders. */
  folder: (token: string, databaseIndex?: number | null) => ResearchFolderLook | undefined;
}

function artworkFailureRows(text: ResearchText, token: string): TooltipRow[] {
  const failure = text.artworkFailure?.(token);
  return failure ? [{ label: failureText(failure, 'Couldn’t load artwork.'), detail: failure.message }] : [];
}

/** A completion's tooltip: the plain date's "by" bound, its start, the time it
 * took and its slot. */
export function completedTooltip(row: Extract<ResearchRow, { kind: 'completed' }>, text: ResearchText): TooltipContent {
  const { entry, started } = row;
  const rows: TooltipRow[] = [
    { label: 'Completed', value: `by ${formatDateTime(row.hour)}` },
    { label: 'Still researching on', value: formatDateTime(entry.lastObservedHour) },
    { label: 'Started', value: started.known ? researchDate(started.hour) : `by ${researchDate(started.hour)}` },
    ...row.tookHours !== null ? [{ label: 'Took', value: researchDays(row.tookHours) }] : [],
    { label: 'Slot', value: `${row.slot.index + 1}${row.startSlot !== null ? ` · started in slot ${row.startSlot + 1}` : ''}` },
  ];
  rows.push(...artworkFailureRows(text, row.token));
  return { title: text.name(row.token, entry.databaseIndex), icon: text.icon(row.token), subtitle: text.folder(row.token, entry.databaseIndex)?.name, rows };
}

/** What a slot held without its recorded occurrence: when it was first and last
 * seen, its last read progress, and that its outcome is not recorded. */
export function heldTooltip(row: Extract<ResearchRow, { kind: 'held' }>, text: ResearchText): TooltipContent {
  const { run, slot } = row;
  const state: ResearchSlotState | undefined = slot.states.find(s => s.token === run.token && s.firstHour <= run.lastHour && run.lastHour <= s.lastHour);
  const progress = state ? researchLatestProgress(slot, state, run.lastHour) : null;
  const rows: TooltipRow[] = [
    { label: 'First held', value: row.startKnown ? researchDate(run.firstHour) : `by ${researchDate(run.firstHour)}` },
    run.endedByHour !== null ? { label: 'Gone', value: `by ${formatDateTime(run.endedByHour)}` } : { label: 'Last seen', value: formatDateTime(run.lastHour) },
    ...progress ? [{ label: 'Last read', value: `${Math.round(progress.fraction * 100)}% on ${formatDateTime(progress.hour)}` }] : [],
    { label: 'Slot', value: `${slot.index + 1}` },
  ];
  rows.push(...artworkFailureRows(text, row.token));
  return { title: text.name(row.token), icon: text.icon(row.token), subtitle: text.folder(row.token)?.name, rows,
    note: 'This recording does not say whether it finished.' };
}

export function gapTooltip(gap: { hour: number; lastHour: number }): TooltipContent {
  return { title: 'Not recorded', rows: [{ label: 'From', value: formatDateTime(gap.hour) }, { label: 'To', value: formatDateTime(gap.lastHour) }],
    note: 'Nothing is dated through this stretch.' };
}

/** What a slot is researching at an hour: its latest read progress dated by
 * that read, the game's days-left estimate, the year it is ahead of, and
 * whether its history can be opened. */
export function currentTooltip(token: string, slotIndex: number, progress: { fraction: number; hour: number } | null, left: number | null,
  joined: boolean, text: ResearchText, aheadOf: number | null = null, databaseIndex: number | null = null): TooltipContent {
  const rows: TooltipRow[] = [
    { label: 'Progress', value: progress ? `${Math.round(progress.fraction * 100)}% on ${formatDateTime(progress.hour)}` : 'Not recorded' },
    ...left !== null ? [{ label: 'Days left', value: `${left}` }] : [],
    ...aheadOf !== null ? [{ label: 'Ahead of time', value: `dated ${aheadOf}`, tone: 'neutral' as const }] : [],
    { label: 'Slot', value: `${slotIndex + 1}` },
  ];
  const notes = [left !== null ? 'Days left is the game’s estimate at this hour’s research speed.' : null,
    aheadOf !== null ? 'Research ahead of its year is slower until then.' : null,
    joined ? null : 'This recording does not identify the assignment, so its history is unknown.'];
  rows.push(...artworkFailureRows(text, token));
  return { title: text.name(token, databaseIndex), icon: text.icon(token), subtitle: text.folder(token, databaseIndex)?.name, rows, note: notes.filter(Boolean).join(' ') || undefined };
}
