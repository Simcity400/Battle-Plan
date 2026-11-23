import { gameDate } from '../../dashboard/calendar.ts';
import type { MissingSpan } from '../../generated/dashboard';
import type { FlatLogItem, LogColumnTrack, LogDensity, LogModel } from './model.ts';

export const LOG_ROW_HEIGHT: Record<LogDensity, number> = {
  comfortable: 28,
  compact: 22,
};

export const LOG_TWO_LINE_HEIGHT: Record<LogDensity, number> = {
  comfortable: 44,
  compact: 36,
};

export const LOG_MONTH_HEIGHT: Record<LogDensity, number> = {
  comfortable: 24,
  compact: 20,
};

export const LOG_GAP_HEIGHT: Record<LogDensity, number> = {
  comfortable: 28,
  compact: 22,
};

export const LOG_MIN_TAIL_LIMIT = 64;
const LOG_OVERSCAN_ROWS = 24;
const LOG_LIMIT_STEP = 16;
const MAX_PHYSICAL_SCROLL_HEIGHT = 1_000_000;

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'] as const;

export function nominalRowHeight(density: LogDensity, twoLine: boolean): number {
  return twoLine ? LOG_TWO_LINE_HEIGHT[density] : LOG_ROW_HEIGHT[density];
}

export function tailLimitForViewport(viewportHeight: number, density: LogDensity, twoLine: boolean): number {
  const rowH = nominalRowHeight(density, twoLine);
  const visible = Math.ceil(Math.max(0, viewportHeight) / rowH);
  const needed = visible + LOG_OVERSCAN_ROWS;
  const stepped = Math.ceil(needed / LOG_LIMIT_STEP) * LOG_LIMIT_STEP;
  return Math.max(LOG_MIN_TAIL_LIMIT, stepped);
}

export function monthKeyOf(hour: number): string {
  const d = gameDate(hour);
  if (!d) return `h-${Math.floor(hour / (24 * 30))}`;
  return `${d.year}-${d.month}`;
}

export function monthLabelOf(hour: number): string {
  const d = gameDate(hour);
  if (!d) return `Hour ${Math.floor(hour)}`;
  return `${MONTHS[d.month - 1] ?? 'Jan'} ${d.year}`;
}

export function shortDateParts(hour: number): { readonly day: string; readonly year: string } {
  const d = gameDate(hour);
  if (!d) return { day: `H${Math.floor(hour)}`, year: '' };
  return {
    day: `${d.day} ${MONTHS[d.month - 1] ?? 'Jan'}`,
    year: String(d.year),
  };
}

function gapLabelOf(span: MissingSpan): string {
  const first = gameDate(span.firstHour);
  const last = gameDate(span.lastHour);
  if (!first || !last) {
    return span.firstHour === span.lastHour
      ? `Not recorded · Hour ${span.firstHour}`
      : `Not recorded · Hour ${span.firstHour} – ${span.lastHour}`;
  }
  const a = `${first.day} ${MONTHS[first.month - 1] ?? 'Jan'} ${first.year}`;
  const b = `${last.day} ${MONTHS[last.month - 1] ?? 'Jan'} ${last.year}`;
  return a === b ? `Not recorded · ${a}` : `Not recorded · ${a} – ${b}`;
}

export interface LogLayout {
  readonly items: readonly FlatLogItem[];
  readonly localHeight: number;
  readonly logicalHeight: number;
  readonly physicalHeight: number;
  readonly compressed: boolean;
  readonly rowHeight: number;
  readonly offsetTop: number;
  readonly monthStarts: readonly { readonly top: number; readonly label: string }[];
}

export function buildLogLayout(model: LogModel, density: LogDensity, showMonths: boolean): LogLayout {
  const twoLine = model.twoLine === true;
  const rowHeight = nominalRowHeight(density, twoLine);
  const monthHeight = LOG_MONTH_HEIGHT[density];
  const gapHeight = LOG_GAP_HEIGHT[density];
  const offsetTop = model.offset * rowHeight;

  const items: FlatLogItem[] = [];
  const monthStarts: { top: number; label: string }[] = [];

  const relevantGaps: MissingSpan[] = [];
  if (model.gaps.length > 0 && model.offset === 0) {
    const hasMoreOlder = model.offset + model.rows.length < model.total;
    const oldestShown = hasMoreOlder && model.rows.length > 0 ? model.rows[model.rows.length - 1]!.hour : -Infinity;
    for (let i = model.gaps.length - 1; i >= 0; i--) {
      const span = model.gaps[i]!;
      if (span.firstHour <= model.hour && span.lastHour >= oldestShown) {
        relevantGaps.push({
          firstHour: span.firstHour,
          lastHour: Math.min(span.lastHour, model.hour),
          reason: span.reason,
        });
      }
    }
  }

  let gapIdx = 0;
  let cursorY = offsetTop;
  let currentMonth = '';

  const pushMonth = (hour: number) => {
    if (!showMonths) return;
    const key = monthKeyOf(hour);
    if (key === currentMonth) return;
    currentMonth = key;
    const label = monthLabelOf(hour);
    monthStarts.push({ top: cursorY, label });
    items.push({ kind: 'month', key: `m:${key}:${items.length}`, label, top: cursorY, height: monthHeight });
    cursorY += monthHeight;
  };

  const pushGap = (span: MissingSpan) => {
    pushMonth(span.lastHour);
    const label = gapLabelOf(span);
    items.push({
      kind: 'gap',
      key: `gap:${span.firstHour}:${span.lastHour}:${span.reason}`,
      span,
      label,
      top: cursorY,
      height: gapHeight,
    });
    cursorY += gapHeight;
  };

  for (let i = 0; i < model.rows.length; i++) {
    const row = model.rows[i]!;
    while (gapIdx < relevantGaps.length && relevantGaps[gapIdx]!.lastHour >= row.hour) {
      pushGap(relevantGaps[gapIdx]!);
      gapIdx++;
    }
    pushMonth(row.hour);
    items.push({
      kind: 'row',
      key: row.key,
      row,
      index: model.offset + i,
      top: cursorY,
      height: rowHeight,
    });
    cursorY += rowHeight;
  }

  while (gapIdx < relevantGaps.length) {
    pushGap(relevantGaps[gapIdx]!);
    gapIdx++;
  }

  const localHeight = cursorY - offsetTop;
  const remainingRows = Math.max(0, model.total - (model.offset + model.rows.length));
  const logicalHeight = cursorY + remainingRows * rowHeight;
  const compressed = logicalHeight > MAX_PHYSICAL_SCROLL_HEIGHT;
  const physicalHeight = compressed ? MAX_PHYSICAL_SCROLL_HEIGHT : logicalHeight;

  return {
    items,
    localHeight,
    logicalHeight,
    physicalHeight,
    compressed,
    rowHeight,
    offsetTop,
    monthStarts,
  };
}

export function stickyMonthAt(layout: LogLayout, scrollTop: number): string | null {
  const starts = layout.monthStarts;
  if (!starts.length) return null;
  let low = 0;
  let high = starts.length;
  while (low < high) {
    const mid = (low + high) >> 1;
    if (starts[mid]!.top <= scrollTop) low = mid + 1;
    else high = mid;
  }
  return low > 0 ? starts[low - 1]!.label : null;
}

export function logicalScrollFromPhysical(layout: LogLayout, viewportHeight: number, physicalTop: number): number {
  if (!layout.compressed) return Math.max(0, physicalTop);
  const physRange = Math.max(1, layout.physicalHeight - viewportHeight);
  const logRange = Math.max(0, layout.logicalHeight - viewportHeight);
  return Math.max(0, Math.min(logRange, (physicalTop / physRange) * logRange));
}

export function physicalScrollFromLogical(layout: LogLayout, viewportHeight: number, logicalTop: number): number {
  if (!layout.compressed) return Math.max(0, logicalTop);
  const physRange = Math.max(1, layout.physicalHeight - viewportHeight);
  const logRange = Math.max(1, layout.logicalHeight - viewportHeight);
  return Math.max(0, Math.min(physRange, (logicalTop / logRange) * physRange));
}

export function visibleItemRange(items: readonly FlatLogItem[], scrollTop: number, viewportHeight: number): readonly [number, number] {
  if (!items.length) return [0, 0];
  let low = 0;
  let high = items.length;
  while (low < high) {
    const mid = (low + high) >> 1;
    const item = items[mid]!;
    if (item.top + item.height <= scrollTop) low = mid + 1;
    else high = mid;
  }
  const start = low;
  const bottom = scrollTop + Math.max(1, viewportHeight);
  let end = start;
  while (end < items.length && items[end]!.top < bottom) end++;
  return [start, end];
}

export interface ResolvedLogColumn {
  readonly id: string;
  readonly left: number;
  readonly width: number;
  readonly align: 'start' | 'end';
}

export function resolveLogColumns(tracks: readonly LogColumnTrack[], totalWidth: number, padX = 10, gap = 8): ResolvedLogColumn[] {
  const inner = Math.max(0, totalWidth - padX * 2);
  let active = [...tracks];
  const minNeeded = (list: readonly LogColumnTrack[]) => {
    let sum = Math.max(0, list.length - 1) * gap;
    for (const track of list) sum += track.width ?? track.minWidth ?? 64;
    return sum;
  };
  while (active.length > 2 && minNeeded(active) > inner) {
    let dropAt = -1;
    let lowestPriority = Infinity;
    for (let i = 0; i < active.length; i++) {
      const prio = active[i]!.priority ?? 0;
      if (prio > 0 && prio < lowestPriority) {
        lowestPriority = prio;
        dropAt = i;
      }
    }
    if (dropAt < 0) break;
    active.splice(dropAt, 1);
  }
  let fixed = Math.max(0, active.length - 1) * gap;
  let flexSum = 0;
  for (const track of active) {
    if (track.flex && track.flex > 0) flexSum += track.flex;
    else fixed += track.width ?? track.minWidth ?? 64;
  }
  const flexPool = Math.max(0, inner - fixed);
  const out: ResolvedLogColumn[] = [];
  let x = padX;
  for (const track of active) {
    const width = track.flex && track.flex > 0
      ? Math.max(track.minWidth ?? 48, (flexPool * track.flex) / flexSum)
      : (track.width ?? track.minWidth ?? 64);
    out.push({ id: track.id, left: x, width, align: track.align ?? 'start' });
    x += width + gap;
  }
  return out;
}
