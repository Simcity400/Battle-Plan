import { repliedCells } from '../../dashboard/follow/liveCells.ts';
import { liveKey } from '../../charts/live.ts';
import type { LaneView, Painted, PaintInput, Scene, Value } from '../lane.ts';
import { advanceTableState, cellIcons, cellText, drawTable, fitState, nextChange, rankMarkers, tablePicture, tableReading, tableState,
  type RankMarker, type TablePicture, type TableReading, type TableRowModel, type TableSurfaceModel, type TableSurfaceState, type TableValues } from '../tables/surface.ts';
import { sourceValues } from '../tables/sources.ts';
import { hitTable, preserveRevealTimes, ROW_HEIGHT, SORT_GLIDE_MS } from '../tables/logic.ts';
import { buildDetail, buildNext, buildStretch, buildTable, type BuiltTable } from '../tables/builders.ts';
import { liveValues, fieldsOf, type Structure } from './fields.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { TableBuild } from '../tables/builders';

export interface TableBinding { model: TableSurfaceModel; tooltipKeys?: Readonly<Record<string, readonly string[]>>; tooltipColumns?: readonly string[]; window?: { first: number; count: number }; audit?: boolean }

interface Held {
  canvas: OffscreenCanvas; ctx: OffscreenCanvasRenderingContext2D; picture: TablePicture; state?: TableSurfaceState; content: string;
  icons: Map<string, ImageBitmap | null>; order?: { sort: string; keys: string[] }; markers: Map<string, RankMarker>; floor: Map<string, number>;
  measures: Map<string, number>; fitted?: TableSurfaceModel; mirror?: { text: string; content: string }; tooltip?: { text: string; content: string };
  audit?: { text: string; content: string };
  wake?: () => void;
}

const answerOf = (value: Value): FollowedAnswer | undefined => value.kind === 'answer' ? value.answer : undefined;

function replyOf(scene: Scene, model: TableSurfaceModel): unknown {
  if (model.sources) return undefined;
  if (model.follow !== undefined) {
    const at = scene.subs.findIndex(sub => sub.id === model.follow);
    const head = at < 0 ? undefined : scene.pictures[at]?.parts[0]!.value;
    return head?.kind === 'panel' ? head.data : undefined;
  }
  return model.reply;
}

function valuesOf(scene: Scene, model: TableSurfaceModel): TableValues | null {
  if (model.sources) {
    const answers = new Map<string, FollowedAnswer | undefined>();
    for (const [name, id] of Object.entries(model.sources)) {
      const at = scene.subs.findIndex(sub => sub.id === id);
      answers.set(name, at < 0 ? undefined : answerOf(scene.pictures[at]!.parts[0]!.value));
    }
    return sourceValues(answers);
  }
  if (model.follow !== undefined) {
    const at = scene.subs.findIndex(sub => sub.id === model.follow);
    const picture = at < 0 ? undefined : scene.pictures[at];
    if (picture?.parts[0]!.value.kind !== 'panel') return null;
    const live = liveValues(picture);
    return live && { get: key => { const v = live.get(key); return typeof v === 'number' || typeof v === 'bigint' ? v : undefined; }, missing: key => { const v = live.get(key); return typeof v === 'string' ? v : 'Not recorded'; } };
  }
  if (model.build && model.reply !== undefined) return repliedCells(model.reply).read();
  if (!model.values) return null;
  const values = model.values, missing = model.missingValues ?? {};
  return { get: key => values[key], missing: key => missing[key] ?? 'Not recorded' };
}

const built = new WeakMap<object, WeakMap<TableBuild, { stretch: number; table: BuiltTable | null }>>();
function builtAt(build: TableBuild, reply: unknown, hour: number): BuiltTable | null {
  if (!reply || typeof reply !== 'object') return buildTable(build, reply, hour);
  let byBuild = built.get(reply);
  if (!byBuild) built.set(reply, byBuild = new WeakMap());
  const stretch = buildStretch(build, hour), held = byBuild.get(build);
  if (held && held.stretch === stretch) return held.table;
  const table = buildTable(build, reply, hour);
  byBuild.set(build, { stretch, table });
  return table;
}

function rowsAt(scene: Scene, model: TableSurfaceModel, hour: number): { rows: readonly TableRowModel[]; empty: string | undefined; reply: unknown } {
  const reply = replyOf(scene, model);
  if (!model.build || reply === undefined) return { rows: model.rows, empty: model.empty, reply };
  const table = builtAt(model.build, reply, hour);
  return { rows: table?.rows ?? [], empty: table?.empty ?? model.empty, reply };
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
    .then(blob => createImageBitmap(blob)).then(image => { held.icons.set(url, image); held.picture = tablePicture(); held.wake?.(); }, () => {});
}

const kept = (held: { text: string; content: string } | undefined, text: string, content: () => string) => held && held.text === text ? held : { text, content: content() };

export function auditRows(rows: readonly TableRowModel[], columns: readonly string[], values: TableValues | null, hour: number, empty: string | undefined) {
  if (!rows.length) return empty === undefined ? [] : [{ row: 'empty', key: 'empty:notice', value: empty }];
  return rows.flatMap(row => columns.map(column => ({ row: row.key, key: `${row.key}:${column}`, value: cellText(row.cells[column] ?? { missing: 'Not recorded' }, values, hour) })));
}

export function tableView(structureOf: (product: string | undefined) => Structure | undefined, wake: (widget: number) => void): LaneView {
  return {
    next(scene, hour) {
      const bound = scene.model as TableBinding | undefined;
      if (!bound) return Infinity;
      const model = bound.model, { rows } = rowsAt(scene, model, hour);
      let next = nextChange(rows, hour) ?? Infinity;
      if (model.build && replyOf(scene, model) !== undefined) next = Math.min(next, buildNext(model.build, hour));
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
      if (held.canvas.width !== width || held.canvas.height !== height) { held.canvas.width = width; held.canvas.height = height; held.picture = tablePicture(); }
      held.ctx.setTransform(surface.dpr, 0, 0, surface.dpr, 0, 0);
      const model = bound.model, values = valuesOf(input, model), { rows, empty, reply } = rowsAt(input, model, input.hour);
      for (const row of rows) for (const key in row.cells) for (const url of cellIcons(row.cells[key]!)) icon(held, url);
      const prior = held.state;
      const made: TableSurfaceModel = { ...model, rows: prior && !model.build ? preserveRevealTimes(rows as TableRowModel[], prior.model.rows) : rows as TableRowModel[], empty };
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
      if (drawTable(held.ctx, next, now, model.scrollLeft, held.icons, held.picture, surface.dpr)) held.content = content();
      const moving = next.glide.size > 0 || next.highlight.size > 0 || [...held.markers.values()].some(marker => marker.until > now);
      const widths = Object.fromEntries(next.layout.columns.map(column => [column.key, column.width]));
      const rowsInfo = { count: next.rows.length, ...next.rows.length || !next.model.empty ? {} : { empty: next.model.empty } };
      fields.widths = { value: widths, content: `widths:${JSON.stringify(widths)}` };
      fields.rows = { value: rowsInfo, content: `rows:${JSON.stringify(rowsInfo)}` };
      if (bound.window) {
        const mirror = { first: bound.window.first, rows: tableReading(next, [], bound.window).rows ?? [] };
        held.mirror = kept(held.mirror, JSON.stringify(mirror), content);
        fields.mirror = { value: mirror, content: `mirror:${held.mirror.content}` };
      }
      if (bound.audit) {
        const rows = auditRows(next.rows, next.layout.columns.map(column => column.key), next.values, next.hour, next.model.empty);
        held.audit = kept(held.audit, JSON.stringify(rows), content);
        fields.audit = { value: rows, content: `audit:${held.audit.content}` };
      }
      if (input.inspect?.startsWith('row:')) {
        const row = input.inspect.split(':')[1]!;
        const reading: TableReading = { ...tableReading(next, bound.tooltipKeys?.[row] ?? bound.tooltipColumns?.map(column => liveKey(row, column)) ?? [], undefined),
          ...model.build && reply !== undefined ? { detail: buildDetail(model.build, reply, row) } : {} };
        const text = JSON.stringify(reading, (_key, value: unknown) => typeof value === 'bigint' ? `${value}n` : value);
        held.tooltip = kept(held.tooltip, `${input.inspect}\u0000${text}`, content);
        fields.tooltip = { value: { key: input.inspect, text: reading }, content: `tooltip:${held.tooltip.content}` };
      }
      const layout = next.layout, shown = next.rows, rowHeight = ROW_HEIGHT[model.settings.density];
      const hit = (point: readonly [number, number]) => {
        const at = hitTable(layout, shown, { x: point[0], y: point[1] }, rowHeight, model.headerHeight ?? 0, model.scrollLeft, model.rankShown, model.rankWidth, model.scrollTop ?? 0);
        return at.rowKey ? `${at.kind}:${at.rowKey}${at.column ? `:${at.column}` : ''}` : at.column ? `${at.kind}:${at.column}` : null;
      };
      return { surfaces: [{ source: held.canvas, content: held.content }], fields, motion: moving, hit };
    },
  };
}

export type { TableRowModel };
