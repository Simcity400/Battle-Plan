import { failureText, type Failure } from '../../dashboard/queryError.ts';
import type { ResearchHistory, ResearchSlotHistory } from './types';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { formatDate, formatDateTime, gameDate, monthNames } from '../../dashboard/calendar.ts';
import { researchNumber } from './researchBreakdown.ts';
import type { ResearchFolderLook } from './researchData';
import { researchAt } from './researchData.ts';
import { researchAssignments, researchStartKnown } from './researchJourney.ts';
import type { ResearchCoverage, ResearchEntry } from '../../generated/dashboard';

export type ResearchItem =
  | { kind: 'completed'; key: string; hour: number; slot: ResearchSlotHistory; entry: ResearchEntry }
  | { kind: 'gap'; key: string; hour: number; lastHour: number }
  | { kind: 'opened'; key: string; hour: number; known: boolean }
  | { kind: 'closed'; key: string; hour: number };

export interface ResearchLane { slot: ResearchSlotHistory; appeared: number; offset: number; items: ResearchItem[] }

const RANK: Record<ResearchItem['kind'], number> = { opened: 0, completed: 1, gap: 2, closed: 3 };

export function coverageAt(coverage: readonly ResearchCoverage[], hour: number): ResearchCoverage[] {
  return coverage.filter(span => span.firstHour <= hour).map(span => ({ firstHour: span.firstHour, lastHour: Math.min(span.lastHour, hour) }));
}

export function coverageGaps(coverage: readonly ResearchCoverage[]): { firstHour: number; lastHour: number }[] {
  return coverage.slice(1).map((span, index) => ({ firstHour: coverage[index].lastHour + 1, lastHour: span.firstHour - 1 }))
    .filter(gap => gap.lastHour >= gap.firstHour);
}

export function researchLanes(slots: readonly ResearchSlotHistory[], coverage: readonly ResearchCoverage[], hour: number): ResearchLane[] {
  const spanOf = (at: number) => coverage.findIndex(span => span.firstHour <= at && at <= span.lastHour);
  const gaps = coverageGaps(coverage).map(gap => ({ ...gap, before: gap.firstHour - 1, after: gap.lastHour + 1 }));
  const lanes: ResearchLane[] = slots.filter(slot => slot.availability.length > 0).map(slot => {
    const items: ResearchItem[] = [];
    for (const entry of slot.entries) if (entry.outcome === 'completed' && entry.endedByHour !== null)
      items.push({ kind: 'completed', key: `c:${entry.firstObservedHour}:${entry.token}`, hour: entry.endedByHour, slot, entry });
    slot.availability.forEach((span, index) => {
      const within = spanOf(span.firstHour);
      if (within >= 0 && coverage[within].firstHour < span.firstHour) {
        items.push({ kind: 'opened', key: `o:${span.firstHour}`, hour: span.firstHour, known: true });
      } else if (within > 0) {
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

export function recordedResearchLanes(history: ResearchHistory, hour: number) {
  return researchLanes(researchAt(history, hour), coverageAt(history.coverage, hour), hour);
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
    : item.kind === 'gap' ? item.lastHour + 1 : item.hour;
}

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

export function researchRevealKey(history: ResearchHistory, hour: number): string {
  const lanes = researchLanesAt(history, hour).map(lane => `${lane.slot.index}:${lane.items.length}:${lane.offset}`).join(',');
  return `${lanes};${history.coverage.filter(span => span.firstHour <= hour).length}`;
}

export type ResearchRow =
  | { kind: 'completed'; key: string; hour: number; slot: ResearchSlotHistory; entry: ResearchEntry; token: string;
    started: { hour: number; known: boolean }; startSlot: number | null; tookHours: number | null }
  | { kind: 'gap'; key: string; hour: number; lastHour: number };

export function completedRow(slots: readonly ResearchSlotHistory[], slot: ResearchSlotHistory, entry: ResearchEntry, hour: number): Extract<ResearchRow, { kind: 'completed' }> {
  const first = researchAssignments(slots, slot, entry)[0];
  const known = researchStartKnown(first.slot, first.entry);
  return { kind: 'completed', key: `c:${slot.index}:${entry.firstObservedHour}:${entry.token}`, hour, slot, entry, token: entry.token,
    started: { hour: first.entry.firstObservedHour, known }, startSlot: first.slot.index !== slot.index ? first.slot.index : null,
    tookHours: known ? hour - first.entry.firstObservedHour : null };
}

export function researchRows(lanes: readonly ResearchLane[], slots: readonly ResearchSlotHistory[], coverage: readonly ResearchCoverage[]): ResearchRow[] {
  const rows: ResearchRow[] = [];
  for (const lane of lanes) for (const item of lane.items) {
    if (item.kind === 'completed') rows.push(completedRow(slots, item.slot, item.entry, item.hour));
  }
  for (const gap of coverageGaps(coverage)) rows.push({ kind: 'gap', key: `g:${gap.firstHour}`, hour: gap.firstHour, lastHour: gap.lastHour });
  return rows.sort((a, b) => a.hour - b.hour || (a.kind === 'gap' ? 1 : 0) - (b.kind === 'gap' ? 1 : 0) || (a.key < b.key ? -1 : a.key > b.key ? 1 : 0));
}

export function researchDate(hour: number): string {
  const date = gameDate(hour);
  return date ? formatDate(date) : 'Unavailable date';
}

export function researchDateRange(firstHour: number, lastHour: number): string {
  const first = gameDate(firstHour), last = gameDate(lastHour);
  if (!first || !last) return 'Unavailable dates';
  if (first.year === last.year && first.month === last.month && first.day === last.day) return formatDate(last);
  if (first.year === last.year && first.month === last.month) return `${first.day}–${formatDate(last)}`;
  if (first.year === last.year) return `${first.day} ${monthNames[first.month - 1].slice(0, 3)} – ${formatDate(last)}`;
  return `${formatDate(first)} – ${formatDate(last)}`;
}

export function researchDays(hours: number): string {
  const text = researchNumber(BigInt(hours) * 100_000n / 24n);
  return `${text} ${text === '1' ? 'day' : 'days'}`;
}

export function researchItemDate(item: Exclude<ResearchItem, { kind: 'gap' }> | Exclude<ResearchRow, { kind: 'gap' }>): string {
  switch (item.kind) {
    case 'completed': return researchDate(item.hour);
    case 'opened': return item.known ? `Opened by ${researchDate(item.hour)}` : `Recorded from ${researchDate(item.hour)}`;
    case 'closed': return `Closed by ${researchDate(item.hour)}`;
  }
}

export interface ResearchText {
  name: (token: string, databaseIndex?: number | null) => string;
  icon: (token: string) => string | undefined;
  artworkFailure?: (token: string) => Failure | undefined;
  folder: (token: string, databaseIndex?: number | null) => ResearchFolderLook | undefined;
}

function artworkFailureRows(text: ResearchText, token: string): TooltipRow[] {
  const failure = text.artworkFailure?.(token);
  return failure ? [{ label: failureText(failure, 'Couldn’t load artwork.'), detail: failure.message }] : [];
}

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

export function gapTooltip(gap: { hour: number; lastHour: number }): TooltipContent {
  return { title: 'Not recorded', rows: [{ label: 'From', value: formatDateTime(gap.hour) }, { label: 'To', value: formatDateTime(gap.lastHour) }],
    note: 'Nothing is dated through this stretch.' };
}

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
