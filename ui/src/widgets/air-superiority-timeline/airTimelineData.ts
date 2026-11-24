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

function markCellSides(cell: AirDayCell | null, hour: number, lastUsed: number[]): void {
  if (!cell || cell.kind === 'reason') return;
  const mark = (side: number) => {
    if (side >= 0 && side < lastUsed.length && hour > lastUsed[side]) lastUsed[side] = hour;
  };
  if (cell.kind === 'view') {
    if (cell.side !== null) mark(cell.side);
    return;
  }
  for (const part of cell.parts) mark(part.side);
  for (const [side] of cell.viewpoints) mark(side);
  for (const side of cell.unknown) mark(side);
  for (const side of cell.noAirObject) mark(side);
}

export function mergeTimelineSides(replies: readonly (AirTimelineReply | undefined)[], moment?: AirSuperiorityReply): AirSide[] {
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
    const lastUsed = new Array<number>(reply.sides.length).fill(-Infinity);
    for (const row of reply.rows) {
      let day = 0;
      for (const [repeat, cell] of row.cells) {
        let hour = -Infinity;
        for (let d = day + repeat - 1; d >= day; d--) {
          const read = reply.days[d];
          if (read != null) { hour = read; break; }
        }
        if (hour > -Infinity) markCellSides(cell, hour, lastUsed);
        day += repeat;
      }
    }
    reply.sides.forEach((side, sideIndex) => record(side, lastUsed[sideIndex]));
  }
  for (const side of moment?.sides ?? []) record(side, Infinity);
  return held;
}

export function validateAirRanking(value: unknown): AirRankingReply {
  if (!object(value) || !Array.isArray(value.rows) || !value.rows.every(row => object(row) && natural(row.region) && optionalText(row.name) && optionalText(row.nameMissing))
    || !Array.isArray(value.warnings) || !value.warnings.every(text)) throw new Error('Invalid air-superiority-ranking reply');
  return value as AirRankingReply;
}
