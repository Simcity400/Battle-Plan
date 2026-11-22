import type { LaneView, Painted, PaintInput, Scene, Value } from '../lane.ts';
import { advanceTableState, cellIcons, drawTable, fitState, rankMarkers, tablePicture, tableReading, tableState,
  type RankMarker, type TablePicture, type TableReading, type TableRowModel, type TableSurfaceModel, type TableSurfaceState, type TableValues } from '../tables/surface.ts';
import { sourceValues } from '../tables/sources.ts';
import { hitTable, preserveRevealTimes, ROW_HEIGHT, SORT_GLIDE_MS } from '../tables/logic.ts';
import { buildDetail, buildStretch, buildTable } from '../tables/builders.ts';
import { liveValues, fieldsOf, type Structure } from './fields.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';

export interface TableBinding { model: TableSurfaceModel; tooltipKeys?: Readonly<Record<string, readonly string[]>>; window?: { first: number; count: number } }

interface Held {
  canvas: OffscreenCanvas; ctx: OffscreenCanvasRenderingContext2D; picture: TablePicture; state?: TableSurfaceState; content: string;
  icons: Map<string, ImageBitmap | null>; order?: { sort: string; keys: string[] }; markers: Map<string, RankMarker>; floor: Map<string, number>;
  measures: Map<string, number>; fitted?: TableSurfaceModel; built?: { reply: unknown; stretch: number; rows: TableRowModel[]; empty: string | undefined };
  painted?: string; widths?: string; rows?: string; wake?: () => void;
}

const GLIDE_MAX_SPEED = 24;
const answerOf = (value: Value): FollowedAnswer | undefined => value.kind === 'answer' ? value.answer : undefined;

function valuesOf(scene: Scene, model: TableSurfaceModel): { values: TableValues | null; reply: unknown } {
  if (model.sources) {
    const answers = new Map<string, FollowedAnswer | undefined>();
    for (const [name, id] of Object.entries(model.sources)) {
      const at = scene.subs.findIndex(sub => sub.id === id);
      answers.set(name, at < 0 ? undefined : answerOf(scene.pictures[at]!.parts[0]!.value));
    }
    return { values: sourceValues(answers), reply: undefined };
  }
  if (model.follow !== undefined) {
    const at = scene.subs.findIndex(sub => sub.id === model.follow);
    const picture = at < 0 ? undefined : scene.pictures[at];
    const head = picture?.parts[0]!.value;
    if (head?.kind === 'panel' && picture) {
      const live = liveValues(picture);
      return { values: live ? { get: key => { const v = live.get(key); return typeof v === 'number' ? v : undefined; }, missing: key => { const v = live.get(key); return typeof v === 'string' ? v : 'Not recorded'; } } : null,
        reply: head.data };
    }
  }
  if (!model.values) return { values: null, reply: model.reply };
  const values = model.values, missing = model.missingValues ?? {};
  return { values: { get: key => values[key], missing: key => missing[key] ?? 'Not recorded' }, reply: model.reply };
}

function measurer(held: Held) {
  return (text: string, font: string) => {
    const key = `${font}\u0000${text}`;
    let width = held.measures.get(key);
    if (width === undefined) {
      if (held.measures.size > 4096) held.measures.clear();
      const was = held.ctx.font; held.ctx.font = font; width = held.ctx.measureText(text).width; held.ctx.font = was;
      held.measures.set(key, width);
    }
    return width;
  };
}

function icon(held: Held, url: string) {
  if (held.icons.has(url)) return;
  held.icons.set(url, null);
  void fetch(url).then(response => { if (!response.ok) throw new Error(`icon ${response.status}`); return response.blob(); })
    .then(blob => createImageBitmap(blob)).then(image => { held.icons.set(url, image); held.painted = undefined; held.wake?.(); }, () => {});
}

export function tableView(structureOf: (product: string | undefined) => Structure | undefined, wake: (widget: number) => void): LaneView {
  return {
    next(scene, hour) {
      const bound = scene.model as TableBinding | undefined;
      const rows = bound?.model.rows ?? [];
      let next = Infinity;
      for (const row of rows) if (row.revealAt !== undefined && row.revealAt > hour && row.revealAt < next) next = row.revealAt;
      if (bound?.model.build) {
        const stretch = buildStretch(bound.model.build, hour);
        for (let at = hour + 1; at <= hour + 24 * 366 && at < next; at += 24) if (buildStretch(bound.model.build, at) !== stretch) { next = at; break; }
      }
      return next;
    },
    paint(input: PaintInput, state: Map<string, unknown>, content: () => string): Painted {
      const bound = input.model as TableBinding | undefined;
      const fields = { ...fieldsOf(input, input.hour, structureOf, state, content) };
      const surface = input.binding.surfaces[0];
      if (!bound || !surface) return { surfaces: [], fields, motion: false };
      let held = state.get('table') as Held | undefined;
      const width = Math.max(1, Math.round(bound.model.width * surface.dpr)), height = Math.max(1, Math.round(bound.model.height * surface.dpr));
      if (!held) {
        const canvas = new OffscreenCanvas(width, height), ctx = canvas.getContext('2d')!;
        state.set('table', held = { canvas, ctx, picture: tablePicture(), content: content(), icons: new Map(), markers: new Map(), floor: new Map(), measures: new Map() });
      }
      held.wake = () => wake(input.widget);
      if (held.canvas.width !== width || held.canvas.height !== height) { held.canvas.width = width; held.canvas.height = height; held.picture = tablePicture(); held.painted = undefined; }
      held.ctx.setTransform(surface.dpr, 0, 0, surface.dpr, 0, 0);
      const model = bound.model, { values, reply } = valuesOf(input, model);
      let rows = model.rows, empty = model.empty;
      if (model.build && reply !== undefined) {
        const stretch = buildStretch(model.build, input.hour);
        if (!held.built || held.built.reply !== reply || held.built.stretch !== stretch) {
          const built = buildTable(model.build, reply, input.hour);
          held.built = { reply, stretch, rows: built?.rows ?? [], empty: built?.empty ?? undefined };
          for (const row of held.built.rows) for (const key in row.cells) for (const url of cellIcons(row.cells[key]!)) icon(held, url);
        }
        rows = held.built.rows; empty = held.built.empty ?? empty;
      } else for (const row of rows) for (const key in row.cells) for (const url of cellIcons(row.cells[key]!)) icon(held, url);
      const prior = held.state;
      const made: TableSurfaceModel = { ...model, rows: prior && !model.build ? preserveRevealTimes(rows, prior.model.rows) : rows, empty };
      const next = tableState(made, values, input.hour, input.pictures[0]?.h ?? input.hour);
      fitState(next, model !== held.fitted ? measurer(held) : null, held.floor);
      held.fitted = model;
      next.hoverKey = typeof input.inspect === 'string' && input.inspect.startsWith('row:') ? input.inspect.split(':')[1] : undefined;
      const now = input.now;
      if (prior) {
        const old = new Map(prior.rows.map((row, index) => [row.key, index]));
        next.glide = new Map();
        for (const [index, row] of next.rows.entries()) {
          const before = old.get(row.key);
          if (before !== undefined && before !== index) next.glide.set(row.key, { from: (before - index) * ROW_HEIGHT[model.settings.density], to: 0, started: now });
          if (row.revealAt !== undefined && row.revealAt > prior.hour && row.revealAt <= next.hour) next.highlight.set(row.key, now + 300);
        }
        if (prior.hour === next.hour && prior.rows.length === next.rows.length) { next.glide = prior.glide; next.highlight = prior.highlight; }
      }
      if (model.rankMarkers) {
        const keys = next.rows.map(row => row.key), sort = `${model.settings.sort.column}\u0000${model.settings.sort.direction}`;
        held.markers = held.order?.sort === sort ? rankMarkers(held.order.keys, keys, now, held.markers) : new Map();
        next.markers = held.markers;
        held.order = { sort, keys };
      }
      for (const [key, glide] of next.glide) if (now - glide.started >= SORT_GLIDE_MS) next.glide.delete(key);
      for (const [key, until] of next.highlight) if (until <= now) next.highlight.delete(key);
      if (next.nextChange !== undefined && next.nextChange <= input.hour) advanceTableState(next, input.hour, now);
      held.state = next;
      drawTable(held.ctx, next, now, model.scrollLeft, held.icons, held.picture, surface.dpr);
      const moving = next.glide.size > 0 || next.highlight.size > 0 || [...held.markers.values()].some(marker => marker.until > now);
      const frame = held.picture.frame ?? '';
      if (frame !== held.painted) { held.painted = frame; held.content = content(); }
      const widths = Object.fromEntries(next.layout.columns.map(column => [column.key, column.width]));
      const widthsText = JSON.stringify(widths);
      const rowsInfo = { count: next.rows.length, ...next.rows.length || !next.model.empty ? {} : { empty: next.model.empty } };
      fields.widths = { value: widths, content: `widths:${widthsText}` };
      fields.rows = { value: rowsInfo, content: `rows:${JSON.stringify(rowsInfo)}` };
      if (bound.window) {
        const reading = tableReading(next, [], bound.window);
        fields.mirror = { value: { first: bound.window.first, rows: reading.rows ?? [] }, content: `mirror:${held.content}:${bound.window.first}:${bound.window.count}` };
      }
      if (input.inspect?.startsWith('row:')) {
        const row = input.inspect.split(':')[1]!;
        const reading: TableReading = { ...tableReading(next, bound.tooltipKeys?.[row] ?? [], undefined),
          ...model.build && reply !== undefined ? { detail: buildDetail(model.build, reply, row) } : {} };
        fields.tooltip = { value: { key: input.inspect, text: reading }, content: `tooltip:${input.inspect}:${held.content}` };
      }
      const point = state.get('pointer') as { point: [number, number] } | null | undefined;
      let hit: string | null | undefined;
      if (point !== undefined) {
        state.delete('pointer');
        if (point) {
          const at = hitTable(next.layout, next.rows, { x: point.point[0], y: point.point[1] }, ROW_HEIGHT[model.settings.density], model.headerHeight ?? 0,
            model.scrollLeft, model.rankShown, model.rankWidth, model.scrollTop ?? 0);
          hit = at.rowKey ? `${at.kind}:${at.rowKey}${at.column ? `:${at.column}` : ''}` : at.column ? `${at.kind}:${at.column}` : null;
        } else hit = null;
      }
      return { surfaces: [{ source: held.canvas, content: held.content }], fields, motion: moving && input.speed <= GLIDE_MAX_SPEED, ...hit !== undefined ? { hit } : {} };
    },
  };
}

export type { TableRowModel };
