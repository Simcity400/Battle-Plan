import type { HeatCell, HeatPeriod, HeatRow, HeatTail } from '../../charts/heat/layout';
import { liveKey, valuesOf } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import { cellView, momentViews, regionLabel, sideOrder } from '../../dashboard/airSuperiority.ts';
import type { AirLooks, AirView } from '../../dashboard/airSuperiority';
import { int, object } from '../../dashboard/validation.ts';
import type { AirCell, AirDayCell, AirRankingReply, AirSide, AirSuperiorityReply, AirTimelineReply, AirTimelineRow } from '../../generated/dashboard';

export const STEP = 256 * 24;
export const tileOf = (hour: number) => Math.floor(hour / STEP);

export const followedEnd = (hour: number, playing: boolean) => playing ? Math.floor(hour / STEP) * STEP + STEP - 1 : Math.floor(hour);

const NOT_RECORDED = valuesOf<number>(new Map(), 'Scores were not recorded at this hour.');

export function asMoment(cell: AirDayCell): { cell: AirCell; values: LiveValues<number> } {
  if (cell.kind === 'reason') return { cell, values: NOT_RECORDED };
  const held = new Map<string, number>();
  if (cell.kind === 'view') {
    held.set(liveKey('view', 'friendly'), Number(cell.friendly));
    held.set(liveKey('view', 'enemy'), Number(cell.enemy));
    return { cell: { kind: 'view', side: cell.side, leader: cell.leader, live: 'view' }, values: valuesOf(held, 'Not recorded') };
  }
  const parts = cell.parts.map((part, at) => {
    held.set(liveKey(String(at), 'friendly'), Number(part.friendly));
    return { side: part.side, tag: part.tag, live: String(at) };
  });
  return { cell: { kind: 'sides', parts, viewpoints: cell.viewpoints, unknown: cell.unknown, noAirObject: cell.noAirObject, filtered: cell.filtered, excluded: 0 },
    values: valuesOf(held, 'Not recorded') };
}

export function dayView(cell: AirDayCell | null, reply: AirTimelineReply, looks: AirLooks, order: ReadonlyMap<string, number>, day: number): AirView {
  if (cell === null || reply.days[day] == null) return cellView(null, reply.sides, NOT_RECORDED, looks, order, null, false);
  const { cell: moment, values } = asMoment(cell);
  const view = cellView(moment, reply.sides, values, looks, order, null);
  const excluded = reply.excluded[day] ?? 0;
  return excluded ? { ...view, excluded: view.excluded + excluded } : view;
}

export function heatCell(view: AirView): HeatCell {
  return { parts: view.parts, unknown: view.state === 'unknown', filtered: view.reason === 'outside-theatres' };
}

export function airPeriods(reply: AirTimelineReply): HeatPeriod[] {
  return reply.days.map((readHour, index) => ({ firstHour: reply.firstHour + index * reply.dayHours,
    lastHour: Math.min(reply.lastHour, reply.firstHour + (index + 1) * reply.dayHours - 1), readHour }));
}

export function cellAt(row: AirTimelineRow, day: number): AirDayCell | null | undefined {
  let start = 0;
  for (const [repeat, cell] of row.cells) {
    if (day < start + repeat) return cell;
    start += repeat;
  }
  return undefined;
}

export function timelineRows(reply: AirTimelineReply, looks: AirLooks): HeatRow[] {
  const order = sideOrder(reply.sides);
  return reply.rows.map(row => {
    const cells: HeatCell[] = [];
    let day = 0;
    for (const [repeat, cell] of row.cells) {
      const shared: (HeatCell | undefined)[] = [];
      for (let at = 0; at < repeat; at++, day++) {
        const read = reply.days[day] == null ? 0 : 1;
        cells.push(shared[read] ??= heatCell(dayView(cell, reply, looks, order, day)));
      }
    }
    return { id: String(row.region), label: regionLabel({ id: row.region, name: row.name }), cells };
  });
}

export function dayOf(reply: AirTimelineReply, hour: number): number | null {
  const day = Math.floor((hour - reply.firstHour) / reply.dayHours);
  return day >= 0 && day < reply.days.length ? day : null;
}

export function headTail(moment: AirSuperiorityReply, values: LiveValues<number>, looks: AirLooks, regions: readonly { id: number; name: string }[],
  hour: number, dayHours: number): { tail: HeatTail; views: Map<number, AirView> } {
  const views = new Map(momentViews(moment, values, looks, regions).map(row => [row.region.id, row.view]));
  const firstHour = Math.floor(hour / dayHours) * dayHours;
  const cells = regions.map(region => {
    const view = views.get(region.id);
    return view ? heatCell(view) : { parts: [] };
  });
  return { tail: { period: { firstHour, lastHour: hour, readHour: hour }, cells }, views };
}

const text = (item: unknown): item is string => typeof item === 'string';
const optionalText = (item: unknown) => item === null || text(item);
const natural = (item: unknown): item is number => int(item) && item >= 0;
const score = (item: unknown) => text(item) && /^-?[0-9]+$/.test(item);

function validDayCell(cell: unknown, sides: number): boolean {
  if (cell === null) return true;
  const index = (item: unknown) => natural(item) && item < sides;
  if (!object(cell)) return false;
  if (cell.kind === 'reason') return text(cell.reason);
  if (cell.kind === 'view') return (cell.side === null || index(cell.side)) && optionalText(cell.leader) && score(cell.friendly) && score(cell.enemy);
  return cell.kind === 'sides' && Array.isArray(cell.parts) && cell.parts.every(part => object(part) && index(part.side) && text(part.tag) && score(part.friendly))
    && Array.isArray(cell.viewpoints) && cell.viewpoints.every(entry => Array.isArray(entry) && entry.length === 2 && index(entry[0]) && text(entry[1]))
    && Array.isArray(cell.unknown) && cell.unknown.every(index) && Array.isArray(cell.noAirObject) && cell.noAirObject.every(index) && typeof cell.filtered === 'boolean';
}

export function validateAirTimeline(value: unknown): AirTimelineReply {
  const invalid = (): never => { throw new Error('Invalid air-superiority-timeline reply'); };
  if (!object(value) || !int(value.firstHour) || !int(value.lastHour) || value.firstHour > value.lastHour || !natural(value.dayHours) || value.dayHours === 0
    || !Array.isArray(value.days) || !Array.isArray(value.excluded) || value.excluded.length !== value.days.length || !value.excluded.every(natural)
    || !Array.isArray(value.sides) || !value.sides.every(side => object(side) && text(side.key)) || !Array.isArray(value.rows) || !Array.isArray(value.warnings)
    || !value.warnings.every(text) || !natural(value.scale) || value.scale === 0 || !natural(value.badBelow) || !natural(value.goodAbove)
    || value.badBelow > value.goodAbove || value.goodAbove > value.scale || !(value.viewpoint === null || natural(value.viewpoint) && value.viewpoint < value.sides.length)) return invalid();
  const first = value.firstHour, last = value.lastHour, dayHours = value.dayHours, sides = value.sides.length;
  if (value.days.length !== Math.floor((last - first) / dayHours) + 1) return invalid();
  value.days.forEach((read, day) => {
    const start = first + day * dayHours;
    if (read !== null && (!int(read) || read < start || read > Math.min(last, start + dayHours - 1))) invalid();
  });
  for (const row of value.rows) {
    if (!object(row) || !natural(row.region) || !optionalText(row.name) || !optionalText(row.nameMissing) || !Array.isArray(row.cells)) return invalid();
    let count = 0;
    for (const run of row.cells) {
      if (!Array.isArray(run) || run.length !== 2 || !natural(run[0]) || !run[0] || !validDayCell(run[1], sides)) return invalid();
      count += run[0];
    }
    if (count !== value.days.length) return invalid();
  }
  return value as AirTimelineReply;
}

export function viewTiles(firstHour: number, lastHour: number): { whole: number[]; cut: number | null } {
  const first = tileOf(firstHour), last = tileOf(lastHour);
  const ends = (lastHour + 1) % STEP === 0;
  const whole: number[] = [];
  for (let tile = first; tile <= (ends ? last : last - 1); tile++) whole.push(tile);
  return { whole, cut: ends ? null : last };
}

export function joinTiles(tiles: readonly (AirTimelineReply | undefined)[], spans: readonly { firstHour: number; lastHour: number }[], looks: AirLooks,
  regions: readonly { id: number; name: string }[]): { rows: HeatRow[]; periods: HeatPeriod[]; loading: { firstHour: number; lastHour: number }[] } {
  const rows: HeatRow[] = regions.map(region => ({ id: String(region.id), label: region.name, cells: [] }));
  const periods: HeatPeriod[] = [], loading: { firstHour: number; lastHour: number }[] = [];
  tiles.forEach((tile, at) => {
    if (!tile) { loading.push(spans[at]); return; }
    periods.push(...airPeriods(tile));
    const made = new Map(timelineRows(tile, looks).map(row => [row.id, row.cells]));
    for (const row of rows) (row.cells as HeatCell[]).push(...(made.get(row.id) ?? tile.days.map(() => ({ parts: [] }))));
  });
  return { rows, periods, loading };
}

interface TileSideIndex {
  readHours: readonly number[];
  byRegion: ReadonlyMap<number, ReadonlyMap<number, readonly [number, number][]>>;
}

const tileSideCache = new WeakMap<AirTimelineReply, TileSideIndex>();

function visitDayCellSides(cell: AirDayCell | null, mark: (side: number) => void): void {
  if (!cell || cell.kind === 'reason') return;
  if (cell.kind === 'view') {
    if (cell.side !== null) mark(cell.side);
    return;
  }
  for (const part of cell.parts) {
    if (Number(part.friendly) > 0) mark(part.side);
  }
  for (const [side] of cell.viewpoints) mark(side);
  for (const side of cell.unknown) mark(side);
  for (const side of cell.noAirObject) mark(side);
}

function visitLiveCellSides(cell: AirCell | null, mark: (side: number) => void): void {
  if (!cell || cell.kind === 'reason') return;
  if (cell.kind === 'view') {
    if (cell.side !== null) mark(cell.side);
    return;
  }
  for (const part of cell.parts) mark(part.side);
  for (const [side] of cell.viewpoints) mark(side);
  for (const side of cell.unknown) mark(side);
  for (const side of cell.noAirObject) mark(side);
}

function indexTile(reply: AirTimelineReply): TileSideIndex {
  const cached = tileSideCache.get(reply);
  if (cached) return cached;
  const readHours: number[] = [];
  const beforeCount = new Int32Array(reply.days.length);
  const throughCount = new Int32Array(reply.days.length);
  for (let d = 0; d < reply.days.length; d++) {
    beforeCount[d] = readHours.length;
    const read = reply.days[d];
    if (read != null) readHours.push(read);
    throughCount[d] = readHours.length;
  }
  const byRegion = new Map<number, Map<number, [number, number][]>>();
  if (readHours.length > 0) {
    const sideCount = reply.sides.length;
    for (const row of reply.rows) {
      const regionSides = new Map<number, [number, number][]>();
      let day = 0;
      for (const [repeat, cell] of row.cells) {
        const startDay = day;
        const endDay = Math.min(day + repeat - 1, reply.days.length - 1);
        day += repeat;
        if (startDay > endDay || !cell || cell.kind === 'reason') continue;
        const lo = beforeCount[startDay];
        const hi = throughCount[endDay] - 1;
        if (lo > hi) continue;
        const seen = new Set<number>();
        visitDayCellSides(cell, side => {
          if (side >= 0 && side < sideCount) seen.add(side);
        });
        for (const side of seen) {
          let spans = regionSides.get(side);
          if (!spans) {
            spans = [[lo, hi]];
            regionSides.set(side, spans);
          } else {
            const prev = spans[spans.length - 1];
            if (lo <= prev[1] + 1) prev[1] = Math.max(prev[1], hi);
            else spans.push([lo, hi]);
          }
        }
      }
      if (regionSides.size > 0) byRegion.set(row.region, regionSides);
    }
  }
  const built: TileSideIndex = { readHours, byRegion };
  tileSideCache.set(reply, built);
  return built;
}

function maxReadIndex(readHours: readonly number[], cutoff: number): number {
  const length = readHours.length;
  if (length === 0 || readHours[0] > cutoff) return -1;
  if (readHours[length - 1] <= cutoff) return length - 1;
  let lo = 0, hi = length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >>> 1;
    if (readHours[mid] <= cutoff) lo = mid;
    else hi = mid - 1;
  }
  return lo;
}

function lastSpanIndex(spans: readonly [number, number][], maxIdx: number): number {
  const last = spans[spans.length - 1];
  if (last[0] <= maxIdx) return Math.min(last[1], maxIdx);
  if (spans[0][0] > maxIdx) return -1;
  let lo = 0, hi = spans.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >>> 1;
    if (spans[mid][0] <= maxIdx) lo = mid;
    else hi = mid - 1;
  }
  return Math.min(spans[lo][1], maxIdx);
}

export interface TimelineSideOptions {
  regions?: readonly number[];
  throughHour?: number;
  moment?: AirSuperiorityReply;
  momentHour?: number | null;
}

export function tileLegendChangeHours(
  replies: readonly (AirTimelineReply | undefined)[],
  regions: readonly number[],
): number[] {
  const hours = new Set<number>();
  for (const reply of replies) {
    if (!reply) continue;
    if (reply.viewpoint != null) hours.add(reply.lastHour);
    const indexed = indexTile(reply);
    if (!indexed.readHours.length) continue;
    const byKey = new Map<string, [number, number][]>();
    const endsByKey = new Map<string, Set<number>>();
    for (const region of regions) {
      const regionSides = indexed.byRegion.get(region);
      if (!regionSides) continue;
      for (const [sideIndex, spans] of regionSides) {
        const key = reply.sides[sideIndex].key;
        let allSpans = byKey.get(key);
        let ends = endsByKey.get(key);
        if (!allSpans) {
          allSpans = [];
          ends = new Set();
          byKey.set(key, allSpans);
          endsByKey.set(key, ends);
        }
        for (const span of spans) {
          hours.add(indexed.readHours[span[0]]);
          allSpans.push(span);
          ends!.add(span[1]);
        }
      }
    }
    for (const [key, allSpans] of byKey) {
      const ends = endsByKey.get(key)!;
      if (ends.size <= 1) continue;
      for (const endIdx of ends) {
        const nextIdx = endIdx + 1;
        if (nextIdx >= indexed.readHours.length) continue;
        if (allSpans.some(([lo, hi]) => lo <= endIdx && nextIdx <= hi)) {
          hours.add(indexed.readHours[nextIdx]);
        }
      }
    }
  }
  return [...hours].sort((a, b) => a - b);
}

export function activeLegendCutoff(now: number, changeHours: readonly number[], momentHour: number | null): number {
  let tileAt = -Infinity;
  if (changeHours.length > 0 && changeHours[0] <= now) {
    if (changeHours[changeHours.length - 1] <= now) {
      tileAt = changeHours[changeHours.length - 1];
    } else {
      let lo = 0, hi = changeHours.length - 1;
      while (lo < hi) {
        const mid = (lo + hi + 1) >>> 1;
        if (changeHours[mid] <= now) lo = mid;
        else hi = mid - 1;
      }
      tileAt = changeHours[lo];
    }
  }
  return momentHour !== null && momentHour <= now ? Math.max(tileAt, momentHour) : tileAt;
}

export function mergeTimelineSides(
  replies: readonly (AirTimelineReply | undefined)[],
  options: TimelineSideOptions = {},
): AirSide[] {
  const cutoff = options.throughHour ?? Infinity;
  const regionFilter = options.regions;
  const held: AirSide[] = [];
  const bestHour: number[] = [];
  const index = new Map<string, number>();
  const record = (side: AirSide, usedHour: number) => {
    const at = index.get(side.key);
    if (at === undefined) {
      index.set(side.key, held.length);
      held.push(side);
      bestHour.push(usedHour);
    } else if (usedHour >= bestHour[at]) {
      held[at] = side;
      bestHour[at] = usedHour;
    }
  };
  for (const reply of replies) {
    if (!reply) continue;
    const indexed = indexTile(reply);
    const maxIdx = maxReadIndex(indexed.readHours, cutoff);
    if (maxIdx < 0) continue;
    const lastUsed = new Array<number>(reply.sides.length).fill(-Infinity);
    const scanRegion = (regionSides: ReadonlyMap<number, readonly [number, number][]>) => {
      for (const [sideIndex, spans] of regionSides) {
        const idx = lastSpanIndex(spans, maxIdx);
        if (idx >= 0) {
          const hour = indexed.readHours[idx];
          if (hour > lastUsed[sideIndex]) lastUsed[sideIndex] = hour;
        }
      }
    };
    if (regionFilter !== undefined) {
      for (const region of regionFilter) {
        const regionSides = indexed.byRegion.get(region);
        if (regionSides) scanRegion(regionSides);
      }
    } else {
      for (const regionSides of indexed.byRegion.values()) scanRegion(regionSides);
    }
    for (let sideIndex = 0; sideIndex < reply.sides.length; sideIndex++) {
      if (lastUsed[sideIndex] > -Infinity) record(reply.sides[sideIndex], lastUsed[sideIndex]);
    }
  }
  const moment = options.moment;
  const momentHour = options.momentHour;
  if (moment && momentHour != null && momentHour <= cutoff) {
    const used = new Array<boolean>(moment.sides.length).fill(false);
    const mark = (side: number) => {
      if (side >= 0 && side < used.length) used[side] = true;
    };
    if (regionFilter !== undefined) {
      const byId = new Map(moment.regions.map(region => [region.id, region.cell]));
      let fallbackUsed = false;
      for (const region of regionFilter) {
        const cell = byId.get(region);
        if (cell) visitLiveCellSides(cell, mark);
        else if (!fallbackUsed) {
          fallbackUsed = true;
          visitLiveCellSides(moment.fallback, mark);
        }
      }
    } else {
      for (const region of moment.regions) visitLiveCellSides(region.cell, mark);
    }
    for (let sideIndex = 0; sideIndex < moment.sides.length; sideIndex++) {
      if (used[sideIndex]) record(moment.sides[sideIndex], momentHour);
    }
  }
  return held;
}

export function timelineViewpointSide(
  replies: readonly (AirTimelineReply | undefined)[],
  options: TimelineSideOptions = {},
): AirSide | null {
  const cutoff = options.throughHour ?? Infinity;
  const moment = options.moment;
  const momentHour = options.momentHour;
  if (moment && moment.viewpoint != null && momentHour != null && momentHour <= cutoff) {
    return moment.sides[moment.viewpoint] ?? null;
  }
  const regionFilter = options.regions;
  for (let i = replies.length - 1; i >= 0; i--) {
    const reply = replies[i];
    if (!reply || reply.firstHour > cutoff) continue;
    if (reply.lastHour <= cutoff) {
      if (reply.viewpoint != null) return reply.sides[reply.viewpoint] ?? null;
      continue;
    }
    const indexed = indexTile(reply);
    const maxIdx = maxReadIndex(indexed.readHours, cutoff);
    if (maxIdx < 0) continue;
    let bestHour = -Infinity;
    let bestSide = -1;
    const scanRegion = (regionSides: ReadonlyMap<number, readonly [number, number][]>) => {
      for (const [sideIndex, spans] of regionSides) {
        const idx = lastSpanIndex(spans, maxIdx);
        if (idx >= 0) {
          const hour = indexed.readHours[idx];
          if (hour > bestHour || (hour === bestHour && sideIndex > bestSide)) {
            bestHour = hour;
            bestSide = sideIndex;
          }
        }
      }
    };
    if (regionFilter !== undefined) {
      for (const region of regionFilter) {
        const regionSides = indexed.byRegion.get(region);
        if (regionSides) scanRegion(regionSides);
      }
    } else {
      for (const regionSides of indexed.byRegion.values()) scanRegion(regionSides);
    }
    if (bestSide >= 0) return reply.sides[bestSide] ?? null;
  }
  return null;
}

export function validateAirRanking(value: unknown): AirRankingReply {
  if (!object(value) || !Array.isArray(value.rows) || !value.rows.every(row => object(row) && natural(row.region) && optionalText(row.name) && optionalText(row.nameMissing))
    || !Array.isArray(value.warnings) || !value.warnings.every(text)) throw new Error('Invalid air-superiority-ranking reply');
  return value as AirRankingReply;
}
