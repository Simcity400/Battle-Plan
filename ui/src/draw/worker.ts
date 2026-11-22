import { PresentationRate, DisplayRate, shownAt } from './frameRate.ts';
import { anchorHour } from './kit.ts';
import type { ClockAnchor, FromWorker, ToWorker } from './kit';
import { Followed, type Answered } from './followed.ts';
import type { FollowItem } from '../dashboard/follow/message';
import { stretch } from './weakCpu.ts';
import { StructureOutbox } from './structureOutbox.ts';
import { PictureDate, pictureHour } from './pictureDate.ts';
import { HeldPointer } from './pointer.ts';
import { PanelProgram } from './retained/panel.ts';
import { panelViews, raceViews } from './panels/views.ts';
import type { PanelModel } from './panels/view';
import type { DrawSample, Present } from './notifications';
import { advanceTableState, cellIcons, drawTable, fitState, rankMarkers, tablePicture, tableReading, tableState, type RankMarker, type TablePicture, type TableRowModel, type TableSurfaceModel, type TableSurfaceState, type TableValues } from './tables/surface.ts';
import { sourceValues } from './tables/sources.ts';
import { hitTable, preserveRevealTimes, SORT_GLIDE_MS } from './tables/logic.ts';
import { drawDateFace, type DateFaceModel } from './dateFace.ts';
import { buildDetail, buildStretch, buildTable, type BuiltTable } from './tables/builders.ts';
import { repliedCells } from '../dashboard/follow/liveCells.ts';

const post = (message: FromWorker) => self.postMessage(message);
const outbox = new StructureOutbox(() => {
  const updates = [...followed.structures(anchor.playing)];
  if (updates.length) post({ type: 'structure', updates });
  return updates.length > 0;
});
const wall = () => performance.timeOrigin + performance.now();
interface Panel {
  kind: 'panel'; id: number; owner: number; model: PanelModel; canvas: OffscreenCanvas; ctx: OffscreenCanvasRenderingContext2D;
  date: PictureDate; program: PanelProgram; pointer: HeldPointer; shown?: boolean; reply?: unknown; through?: number; nextText?: number;
}
interface Race {
  kind: 'race'; id: number; owner: number; model: Extract<ToWorker, { type: 'race' }>['model']; picture: InstanceType<(typeof raceViews)[keyof typeof raceViews]>; shown?: boolean; through?: number;
  reply?: unknown; nextText?: number; confirmed?: number; since?: number;
}
interface Table {
  kind: 'table'; id: number; owner: number; model: TableSurfaceModel; canvas: OffscreenCanvas; ctx: OffscreenCanvasRenderingContext2D;
  state?: TableSurfaceState; shown?: boolean; through?: number; reply?: unknown; scrollLeft: number; scrollTop: number;
  icons: Map<string, ImageBitmap | null>; hoverKey?: string;
  iconsFor?: readonly TableRowModel[];
  built?: { reply: unknown; build: unknown; stretch: number; table: BuiltTable | null };
  rowCount?: number;
  emptyTold?: string;
  floor: Map<string, number>;
  floorSeek?: number;
  measures: Map<string, number>;
  fittedAt?: number;
  widthsTold?: string;
  fitModel?: TableSurfaceModel;
  fitRows?: readonly TableRowModel[];
  picture: TablePicture; dpr?: number;
  split?: boolean;
  waitFrom?: number;
  markEnds?: number;
  painted?: boolean;
  held: Map<string, Answered>;
  order?: { sort: string; keys: string[] }; markers: Map<string, RankMarker>;
}
type Surface = Panel | Race | Table;
const cost = new Map<Surface, number>();
const spent = (s: Surface, since: number) => { if (audit) cost.set(s, (cost.get(s) ?? 0) + performance.now() - since); };
const panels = new Map<number, Panel>(), races = new Map<number, Race>();
interface Face { ctx: OffscreenCanvasRenderingContext2D; model: DateFaceModel; width: number; height: number; dpr: number; drawn?: number }
const faces = new Map<number, Face>();
const iconWaiting = new Map<string, Set<Table>>();
const GLIDE_MAX_SPEED = 24;
const tables = new Map<number, Table>();
const tableIcons = new Map<string, ImageBitmap | null>();
const subscriptions = new Map<number, Set<Surface>>(), dirty = new Set<Surface>();
const scheduledChanges: { at: number; surface: Surface }[] = [];
const tableReads: Extract<ToWorker, { type: 'tableRead' }>[] = [];
let unreported = false;
let anchor: ClockAnchor = { position: 0, at: 0, speed: 1, playing: false, lastHour: 0, seek: 0 };
let rate = 1, cap = 60, frameNumber = 0;
const PAINT_SHARE = 0.6;
const deferred = new Set<Surface>();
let responding = false;
let audit: DrawSample[] | undefined;
let auditOffset = 0;
const cadence = new PresentationRate(), display = new DisplayRate();
const position = () => anchorHour(anchor, wall());
let scheduled = false;
let markWake: ReturnType<typeof setTimeout> | undefined;
let lane = 0;
let shownWall = 0;
function wake() { if (!scheduled) { scheduled = true; requestAnimationFrame(frame); } }
let dataPort: MessagePort | undefined;
const followed = new Followed(what => dataPort?.postMessage(what), id => {
  for (const s of subscriptions.get(id) ?? []) dirty.add(s);
});

function requestTableIcon(table: Table, url: string) {
  if (tableIcons.has(url)) { table.icons.set(url, tableIcons.get(url) ?? null); return; }
  const waiting = iconWaiting.get(url);
  if (waiting) { waiting.add(table); return; }
  iconWaiting.set(url, new Set([table]));
  void fetch(url).then(response => { if (!response.ok) throw new Error(`icon ${response.status}`); return response.blob(); })
    .then(blob => createImageBitmap(blob)).then(image => landIcon(url, image), () => landIcon(url, null));
}
function landIcon(url: string, image: ImageBitmap | null) {
  tableIcons.set(url, image);
  for (const table of iconWaiting.get(url) ?? []) if (tables.get(table.id) === table) { table.icons.set(url, image); dirty.add(table); }
  iconWaiting.delete(url);
  wake();
}

const followIds = (s: Surface): number[] => [...s.model.follow === undefined ? [] : [s.model.follow],
  ...s.kind === 'table' && s.model.sources ? Object.values(s.model.sources) : []];
function unbind(s: Surface) {
  for (const id of followIds(s)) {
    const dependents = subscriptions.get(id);
    dependents?.delete(s);
    if (dependents?.size === 0) subscriptions.delete(id);
  }
  const due = scheduledChanges.findIndex(q => q.surface === s);
  if (due >= 0) scheduledChanges.splice(due, 1);
  dirty.delete(s); deferred.delete(s);
}
function bind(s: Surface) {
  for (const id of followIds(s)) {
    let dependents = subscriptions.get(id);
    if (!dependents) subscriptions.set(id, dependents = new Set());
    dependents.add(s);
  }
  dirty.add(s);
}
function invalidate(s: Surface) {
  if (s.kind === 'panel') {
    s.program.clear(); s.pointer.clear(); s.date.drawn = undefined; s.reply = undefined;
  } else if (s.kind === 'race') s.picture.clear();
  else {
    s.state = undefined; s.held.clear(); s.order = undefined; s.markers.clear(); s.picture = tablePicture(); s.ctx.clearRect(0, 0, s.model.width, s.model.height);
    s.painted = false; s.split = false; s.waitFrom = undefined; s.markEnds = undefined;
  }
  s.through = s.shown = undefined; dirty.add(s);
}
function confirmedThrough(s: Surface) {
  if (s.kind === 'race') return s.confirmed;
  if (s.kind === 'table') {
    const date = tableDate(s);
    return date?.through === undefined ? undefined : date.through - s.model.offset;
  }
  const { model } = s;
  const date = model.follow === undefined ? model.date : followed.read(model.follow)?.date;
  return date?.through === undefined ? undefined : date.through - model.offset;
}
function scheduleChange(surface: Surface, at: number) {
  const current = scheduledChanges.findIndex(q => q.surface === surface);
  if (current >= 0) {
    if (scheduledChanges[current]!.at === at) return;
    scheduledChanges.splice(current, 1);
  }
  if (!Number.isFinite(at) || surface.model.fixed !== undefined) return;
  let lo = 0, hi = scheduledChanges.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (scheduledChanges[mid]!.at > at) lo = mid + 1; else hi = mid;
  }
  scheduledChanges.splice(lo, 0, { surface, at });
}
function paint(s: Surface, head: number) {
  const { model } = s;
  const fixed = model.fixed ?? (anchor.playing ? undefined : Math.floor(head));
  const entry = model.follow === undefined ? undefined : followed.read(model.follow);
  if (s.kind === 'race') {
    const m = s.model;
    const through = s.picture.draw(m, m.follow === undefined ? m.reply : entry?.reply as typeof m.reply,
      m.follow === undefined ? m.date : entry?.date, head, fixed);
    s.through = through ?? undefined;
    s.reply = m.follow === undefined ? m.reply : entry?.reply;
    if (through !== null) {
      s.nextText = s.picture.nextChange(m) - m.offset;
      s.since = (m.follow === undefined ? m.date : entry?.date)!.since - m.offset;
      s.confirmed = (m.follow === undefined ? m.date : entry?.date)!.through - m.offset;
      scheduleChange(s, s.nextText);
    }
    return s.picture.changed;
  }
  if (s.kind === 'table') {
    const m = s.model, input = tableInput(s), date = input?.date;
    const through = date?.through === undefined ? undefined : Math.floor(Math.min((m.fixed ?? head) + m.offset, date.through) - m.offset);
    if (!input || through === undefined || (fixed !== undefined && through < fixed)) return blankTable(s);
    if (date!.since > Math.floor(m.fixed ?? head) + m.offset) { const blanked = blankTable(s); s.waitFrom = date!.since; return blanked; }
    s.waitFrom = undefined;
    s.split = date!.since > date!.through;
    if (s.split) { if (!s.state) { const cleared = blankTable(s); s.split = true; return cleared; } return false; }
    const made = drawnModel(s, input.reply, through + m.offset);
    if (!made) return blankTable(s);
    const prior = s.state;
    const rows = prior && !m.build ? preserveRevealTimes(made.rows, prior.model.rows) : made.rows;
    const current = rows === made.rows ? made : { ...made, rows };
    if (s.iconsFor !== m.rows) { s.iconsFor = m.rows; for (const row of rows) for (const key in row.cells) for (const icon of cellIcons(row.cells[key]!)) requestTableIcon(s, icon); }
    s.state = tableState(current, input.values, through + m.offset, date!.since);
    const fresh = m !== s.fitModel || current.rows !== s.fitRows;
    s.fitModel = m; s.fitRows = current.rows;
    fitTable(s, s.state, performance.now(), fresh);
    const empty = s.state.rows.length ? undefined : s.state.model.empty;
    if (s.rowCount !== s.state.rows.length || s.emptyTold !== empty) {
      s.rowCount = s.state.rows.length; s.emptyTold = empty;
      post({ type: 'tableRows', id: s.id, owner: s.owner, count: s.rowCount, ...(empty !== undefined && { empty }) });
    }
    s.state.hoverKey = s.hoverKey;
    const watchable = !anchor.playing || anchor.speed <= GLIDE_MAX_SPEED;
    const revealed = prior !== undefined && prior.nextChange !== undefined && prior.nextChange <= s.state.hour;
    if (prior && (watchable || revealed)) {
      const old = watchable ? new Map(prior.rows.map((row, index) => [row.key, index])) : undefined;
      const glide = new Map<string, { from: number; to: number; started: number }>();
      for (const [index, row] of s.state.rows.entries()) {
        const before = old?.get(row.key);
        if (before !== undefined && before !== index) glide.set(row.key, { from: (before - index) * (m.settings.density === 'compact' ? 22 : 28), to: 0, started: performance.now() });
        if (row.revealAt !== undefined && row.revealAt > prior.hour && row.revealAt <= s.state.hour) s.state.highlight.set(row.key, performance.now() + 300);
      }
      s.state.glide = glide;
    }
    const now = performance.now();
    markRanks(s, s.state, now);
    drawTable(s.ctx, s.state, now, s.scrollLeft, s.icons, s.picture, s.dpr);
    s.markEnds = markEnd(s.state, now); s.painted = true;
    s.reply = input.reply; s.through = through;
    return true;
  }
  const m = s.model, at = Math.floor(m.fixed ?? head) + m.offset;
  const raw = m.follow === undefined ? m.reply : entry?.reply;
  let date = m.follow === undefined ? m.date : entry?.date;
  const view = panelViews[m.view];
  if (m.clocked && raw && view?.nextChange) {
    const next = view.nextChange(raw, at);
    date = { since: at, through: Math.min(next - 1, anchor.lastHour + m.offset) };
    scheduleChange(s, next - m.offset);
  }
  s.date.set(date ? date.through - m.offset : null, fixed, date ? date.since - m.offset : null);
  if (!raw || !s.date.canPaintAt(head)) {
    if (s.date.drawn) s.program.clear();
    s.date.drawn = undefined; s.through = undefined; s.pointer.clear();
    return false;
  }
  const through = Math.floor(m.fixed ?? Math.min(head, date!.through - m.offset));
  const same = s.date.drawn && raw === s.reply && !m.clocked && !entry?.hasFollowedCells && (!view?.nextChange || s.through === through);
  let changed = false;
  if (!same) {
    let values;
    try { values = view?.values(raw, entry?.live ?? null, through + m.offset, m.structure); }
    catch { values = null; }
    if (!values) {
      if (anchor.playing && m.fixed === undefined && s.date.presentableAt(head)) return false;
      s.program.clear(); s.pointer.clear(); s.date.drawn = undefined; s.through = undefined; return false;
    }
    changed = s.program.write(m.cells, values);
    s.reply = raw;
  }
  s.date.paint(); s.through = through;
  if (view?.nextChange) {
    s.nextText = view.nextChange(raw, through + m.offset) - m.offset;
    scheduleChange(s, s.nextText);
  }
  if (changed) s.pointer.present((x, y) => s.program.hit(x, y));
  return changed;
}
function tableSources(t: Table): Map<string, Answered> | null {
  const reads = new Map<string, Answered>();
  for (const [name, id] of Object.entries(t.model.sources ?? {})) {
    const now = followed.answered(id);
    if (now) t.held.set(name, now);
    const read = now ?? t.held.get(name);
    if (!read) return null;
    reads.set(name, read);
  }
  return reads;
}
const joined = (reads: ReadonlyMap<string, Answered>) => {
  let since = -Infinity, through = Infinity;
  for (const { date } of reads.values()) { since = Math.max(since, date.since); through = Math.min(through, date.through); }
  return { since, through };
};
function tableDate(t: Table) {
  const m = t.model;
  if (m.sources) { const reads = tableSources(t); return reads && joined(reads); }
  return m.follow === undefined ? m.date : followed.read(m.follow)?.date;
}
function tableInput(t: Table): { date?: { since: number; through: number }; values: TableValues | null; reply?: unknown } | undefined {
  const m = t.model;
  if (m.sources) {
    const reads = tableSources(t);
    return reads ? { date: joined(reads), values: sourceValues(new Map([...reads].map(([name, read]) => [name, read.answer]))) } : undefined;
  }
  if (m.follow === undefined) return { date: m.date, values: m.reply !== undefined ? repliedCells(m.reply).read() : modelValues(m), reply: m.reply };
  const entry = followed.read(m.follow);
  return entry?.reply ? { date: entry.date, values: entry.live ?? followed.sectionLive(m.follow) ?? null, reply: entry.reply } : undefined;
}
function drawnModel(t: Table, reply: unknown, hour: number): TableSurfaceModel | null {
  const m = t.model;
  if (!m.build || reply === undefined) return m;
  const held = t.built, stretch = buildStretch(m.build, hour);
  let built = held && held.reply === reply && held.build === m.build && held.stretch === stretch ? held : undefined;
  if (!built) {
    built = t.built = { reply, build: m.build, stretch, table: buildTable(m.build, reply, hour) };
    for (const row of built.table?.rows ?? []) for (const key in row.cells) for (const icon of cellIcons(row.cells[key]!)) requestTableIcon(t, icon);
  }
  const table = built.table;
  return table && { ...m, rows: table.rows, empty: table.empty ?? undefined };
}
function modelValues(m: TableSurfaceModel): TableValues | null {
  if (!m.values) return null;
  const values = m.values, missing = m.missingValues ?? {};
  return { get: key => values[key], missing: key => missing[key] ?? 'Not recorded' };
}
function blankTable(s: Table) {
  const drawn = s.painted === true;
  s.state = undefined; s.through = undefined; s.split = false; s.waitFrom = undefined; s.markEnds = undefined;
  if (!drawn) return false;
  s.picture = tablePicture(); s.ctx.clearRect(0, 0, s.model.width, s.model.height); s.painted = false;
  return true;
}
const FIT_MS = 500;
function fitTable(s: Table, state: TableSurfaceState, now: number, fresh: boolean) {
  if (s.floorSeek !== anchor.seek) { s.floorSeek = anchor.seek; fresh = true; }
  const measure = fresh || now - (s.fittedAt ?? -Infinity) >= FIT_MS ? (text: string, font: string) => {
    const key = `${font}\u0000${text}`;
    let width = s.measures.get(key);
    if (width === undefined) {
      if (s.measures.size > 4096) s.measures.clear();
      const held = s.ctx.font; s.ctx.font = font; width = s.ctx.measureText(text).width; s.ctx.font = held;
      s.measures.set(key, width);
    }
    return width;
  } : null;
  if (measure) s.fittedAt = now;
  fitState(state, measure, s.floor);
  const widths = Object.fromEntries(state.layout.columns.map(column => [column.key, column.width]));
  const told = JSON.stringify(widths);
  if (told !== s.widthsTold) { s.widthsTold = told; post({ type: 'tableColumns', id: s.id, owner: s.owner, widths }); }
}
function markEnd(state: TableSurfaceState, now: number): number | undefined {
  let end: number | undefined;
  for (const marker of state.markers?.values() ?? []) if (marker.until > now && (end === undefined || marker.until < end)) end = marker.until;
  return end;
}
function markRanks(s: Table, state: TableSurfaceState, now: number) {
  const m = s.model;
  if (!m.rankMarkers) { s.order = undefined; s.markers.clear(); return; }
  const keys = state.rows.map(row => row.key), sort = `${m.settings.sort.column}\u0000${m.settings.sort.direction}`;
  s.markers = s.order?.sort === sort ? rankMarkers(s.order.keys, keys, now, s.markers) : new Map();
  state.markers = s.markers;
  s.order = { sort, keys };
}
function advanceTable(table: Table, head: number) {
  if (table.waitFrom !== undefined) {
    if (Math.floor(table.model.fixed ?? head) + table.model.offset >= table.waitFrom) { table.waitFrom = undefined; deferred.add(table); }
    return;
  }
  const date = tableDate(table);
  if (table.split) {
    if (date && date.since <= date.through) deferred.add(table);
    else if (table.markEnds !== undefined && table.state) {
      const now = performance.now();
      if (now >= table.markEnds) { drawTable(table.ctx, table.state, now, table.scrollLeft, table.icons, table.picture, table.dpr); table.markEnds = markEnd(table.state, now); }
    }
    return;
  }
  if (!table.state) return;
  if (date?.through === undefined) return;
  const through = Math.floor(Math.min((table.model.fixed ?? head) + table.model.offset, date.through) - table.model.offset), hour = through + table.model.offset;
  if (hour < table.state.hour || hour < date.since) return;
  const build = table.model.build;
  if (build && hour !== table.state.hour && buildStretch(build, hour) !== buildStretch(build, table.state.hour)) { deferred.add(table); return; }
  const now = performance.now(), state = table.state;
  let moving = false, ended = false;
  for (const [key, glide] of state.glide) if (now - glide.started < SORT_GLIDE_MS) moving = true; else { state.glide.delete(key); ended = true; }
  for (const [key, until] of state.highlight) if (until > now) moving = true; else { state.highlight.delete(key); ended = true; }
  const revealing = state.nextChange !== undefined && hour >= state.nextChange;
  if (!moving && !ended && !revealing && (table.markEnds === undefined || now < table.markEnds)) { state.hour = hour; table.through = through; return; }
  if (revealing) { advanceTableState(state, hour, now); fitTable(table, state, now, false); markRanks(table, state, now); } else state.hour = hour;
  table.through = through;
  drawTable(table.ctx, state, now, table.scrollLeft, table.icons, table.picture, table.dpr);
  table.markEnds = markEnd(state, now); table.painted = true;
}
function report(s: Surface, head: number, notices: Present[]) {
  const shown = s.through !== undefined;
  if (s.shown === shown) return;
  s.shown = shown;
  if (s.kind === 'race') {
    post({ type: 'racePresent', id: s.id, owner: s.owner, seek: anchor.seek, generation: anchor.generation ?? 0,
      head: s.model.fixed ?? head, hour: s.through ?? null, at: wall(), shown: shownWall });
  } else if (s.kind === 'panel') notices.push({ id: s.id, owner: s.owner, seek: anchor.seek, generation: anchor.generation ?? 0,
    head: s.model.fixed ?? head, date: s.date.drawn, shown: shownWall });
  else notices.push({ id: s.id, owner: s.owner, seek: anchor.seek, generation: anchor.generation ?? 0,
    head: s.model.fixed ?? head, readHour: s.through, at: wall(), shown: shownWall });
}

function tableRows(s: Table) {
  if (!s.state) return undefined;
  if (!s.state.rows.length && s.state.model.empty) return [{ row: 'empty:notice', key: 'empty:notice', value: 'empty:notice' }];
  return s.state.rows.map(row => ({ row: row.key, key: row.key, value: row.key }));
}
function record(s: Surface, at: number, head: number, sample = false, detached = false) {
  if (!audit || !s.model.widget) return;
  const target = Math.floor(s.model.fixed ?? head);
  const entry = s.model.follow === undefined ? undefined : followed.read(s.model.follow);
  const input = s.kind === 'table' ? tableDate(s) ?? undefined : s.model.follow === undefined ? s.model.date : entry?.date;
  const eligible = s.kind === 'panel' && s.model.clocked && s.model.reply ? target + s.model.offset :
    pictureHour(input, target + s.model.offset, s.model.fixed === undefined ? undefined : target + s.model.offset);
  let confirmed = s.kind === 'panel' ? s.date.drawn?.through : s.kind === 'race' ? s.confirmed : s.through;
  if (s.reply && s.reply === entry?.reply && !entry.hasFollowedCells)
    confirmed = entry.date.through - s.model.offset;
  const nextText = s.kind === 'panel' || s.kind === 'race' ? s.nextText : undefined;
  const hour = s.through === undefined ? undefined : Math.max(s.through,
    Math.min(target, confirmed ?? s.through, (nextText ?? Infinity) - 1));
  audit.push({ t: 'picture', widget: s.model.widget, surface: s.kind === 'table' ? 'table' : s.model.view,
    placement: s.id, owner: s.owner, at, shownAt: detached ? at : Math.max(at, shownWall), seek: anchor.seek, head: target, hour: detached ? null : hour ?? null, renderer: 'worker', drawMs: cost.get(s) ?? 0,
    since: s.kind === 'panel' ? s.date.drawn?.since : s.kind === 'race' ? s.since : s.model.date?.since,
    through: confirmed === undefined ? undefined : Math.min(confirmed, (nextText ?? Infinity) - 1),
    eligibleHour: eligible === null ? null : eligible - s.model.offset, inputThrough: entry && entry.date.through - s.model.offset,
    receivedThrough: s.model.follow === undefined ? undefined : followed.receivedThrough(s.model.follow),
    offset: s.model.offset, fixedHour: s.model.fixed, detached,
    ...(sample && hour !== undefined && { rows: s.kind === 'race' ? s.picture.rows() : s.kind === 'panel' ? s.program.snapshot() : tableRows(s) }) });
}
function frame(at: number) {
  scheduled = false;
  if (anchor.playing) {
    const hz = display.sample(at);
    if (hz !== undefined && lane === 0) post({ type: 'displayRate', hz });
  }
  const began = performance.now(), shown = shownAt(at, began, display.interval);
  shownWall = performance.timeOrigin + shown;
  const head = anchorHour(anchor, shownWall);
  if (anchor.playing) {
    const step = anchor.speed / cap, through = followed.stepHour;
    const start = through !== undefined && Number.isInteger(step) && step > 0 && head < anchor.lastHour
      ? through + Math.floor((head - through) / step) * step : undefined;
    const boundary = start === undefined ? undefined : anchor.at - performance.timeOrigin + (start - anchor.position) * 1000 / anchor.speed;
    if (!cadence.due(shown, cap, boundary)) { wake(); return; }
  }
  followed.drain(head);
  if (lane === 0) {
    outbox.flush();
  }
  while (scheduledChanges.length && scheduledChanges.at(-1)!.at <= head) dirty.add(scheduledChanges.pop()!.surface);
  const changed = audit && new Set<string>();
  const notices: Present[] = [];
  const applied = audit ? performance.now() : began;
  const budget = responding ? Infinity : PAINT_SHARE * 1000 / (cap || 60) / Math.max(1, rate);
  const order = deferred.size ? [...deferred, ...[...dirty].filter(s => !deferred.has(s))] : [...dirty];
  deferred.clear();
  let paintedAny = false;
  for (const s of order) {
    if (paintedAny && performance.now() - began > budget) { deferred.add(s); continue; }
    paintedAny = true;
    const since = audit ? performance.now() : 0, painted = paint(s, head);
    spent(s, since);
    if (painted && s.model.widget) changed?.add(s.model.widget);
    report(s, head, notices);
  }
  if (unreported) {
    unreported = false;
    for (const s of [...panels.values(), ...races.values(), ...tables.values()]) if (s.shown === undefined) report(s, head, notices);
  }
  for (const face of faces.values()) {
    const hour = Math.floor(head) + face.model.offset;
    if (face.drawn !== hour) { drawDateFace(face.ctx, face.model, hour, face.width, face.height, face.dpr); face.drawn = hour; }
  }
  for (const table of tables.values()) if (!dirty.has(table) && !deferred.has(table)) { const since = audit ? performance.now() : 0; advanceTable(table, head); spent(table, since); }
  if (notices.length) post({ type: 'present', surfaces: notices });
  if (tableReads.length) for (const read of tableReads.splice(0)) {
    const table = tables.get(read.id), state = table?.owner === read.owner ? table.state : undefined;
    const detail = state && read.detail !== undefined && table!.model.build ? buildDetail(table!.model.build, table!.reply, read.detail) : undefined;
    post({ type: 'tableRead', id: read.id, owner: read.owner, request: read.request,
      reading: state ? { ...tableReading(state, read.keys ?? [], read.rows), ...(detail != null && { detail }) } : null });
  }
  const end = performance.now(), workMs = end - began;
  stretch(workMs, rate, end);
  if (audit) {
    const presented = wall();
    const sample = frameNumber++ % 60 === 0;
    for (const s of panels.values()) record(s, presented, head, sample);
    for (const s of races.values()) record(s, presented, head, sample);
    for (const s of tables.values()) record(s, presented, head, sample);
    audit.push({ t: 'workerFrame', at: presented, frameAt: performance.timeOrigin + at, shownAt: shownWall, head, seek: anchor.seek, playing: anchor.playing, workMs, lane,
      applyMs: applied - began, paintMs: end - applied, rate, changed: changed!.size });
  }
  dirty.clear(); cost.clear();
  if (!deferred.size) responding = false;
  if (deferred.size || anchor.playing && (panels.size || races.size || tables.size || faces.size || followed.size)) wake();
  else {
    let next = Infinity;
    for (const table of tables.values()) if (table.markEnds !== undefined && table.markEnds < next) next = table.markEnds;
    if (next < Infinity && markWake === undefined) markWake = setTimeout(() => { markWake = undefined; wake(); }, Math.max(0, next - performance.now()));
  }
}

self.onmessage = ({ data: m }: MessageEvent<ToWorker>) => {
  switch (m.type) {
    case 'audit': {
      if (m.start) {
        audit = []; auditOffset = 0; frameNumber = 0;
        post({ type: 'audit', records: [], done: true });
        break;
      }
      const records = audit ?? [], limit = Math.max(1, Math.floor(m.limit ?? records.length));
      const chunk = records.slice(auditOffset, auditOffset + limit);
      auditOffset += chunk.length;
      const done = auditOffset >= records.length;
      if (done) { audit = undefined; auditOffset = 0; }
      post({ type: 'audit', records: chunk, done }); break;
    }
    case 'port':
      dataPort = m.port;
      m.port.onmessage = ({ data }: MessageEvent<FollowItem[]>) => {
        if (lane === 0) for (const item of data) if (item.kind === 'failure') post({ type: 'followFailure', reason: item.reason });
        followed.land(data);
        if (lane === 0) outbox.flush();
        wake();
      };
      return;
    case 'structureTaken': outbox.taken(); return;
    case 'add': followed.add(m.id, m.branch, m.product); return;
    case 'remove': followed.remove(m.id); break;
    case 'seek': followed.seek(m.branch, m.seek, m.retainThrough); break;
    case 'bind': followed.bind(m.branch, m.route, m.seek); break;
    case 'options':
      rate = m.rate; lane = m.lane;
      if (cap !== m.cap) { cap = m.cap; cadence.reset(); }
      return;
    case 'race': {
      let race = races.get(m.id);
      if (race) {
        unbind(race);
        race.model = m.model; race.owner = m.owner; race.picture.resize(); race.shown = undefined;
      } else if (m.canvas) {
        const picture = new raceViews[m.model.view](m.canvas, () => { if (race) { dirty.add(race); wake(); } },
          item => { if (race) post({ type: 'raceHover', id: race.id, owner: race.owner, seek: anchor.seek, item }); });
        races.set(m.id, race = { kind: 'race', id: m.id, picture, owner: m.owner, model: m.model });
      }
      if (race) bind(race);
      break;
    }
    case 'table': {
      let table = tables.get(m.id);
      if (!table) {
        const ctx = m.canvas?.getContext('2d');
        if (!ctx || !m.canvas) { post({ type: 'failed', page: m.id, reason: 'Table drawing surface unavailable' }); break; }
        tables.set(m.id, table = { kind: 'table', id: m.id, owner: m.owner, model: m.model, canvas: m.canvas, ctx, scrollLeft: m.model.scrollLeft, scrollTop: m.model.scrollTop ?? 0,
          icons: new Map(), held: new Map(), markers: new Map(), picture: tablePicture(), floor: new Map(), measures: new Map() });
      } else {
        const before = table.model;
        unbind(table); table.owner = m.owner; table.model = m.model; table.scrollLeft = m.model.scrollLeft; table.scrollTop = m.model.scrollTop ?? 0;
        table.state = undefined; table.hoverKey = undefined; table.reply = undefined; table.through = undefined; table.shown = undefined;
        if (before.rankMarkers !== m.model.rankMarkers) { table.order = undefined; table.markers.clear(); }
        table.rowCount = undefined;
        if (before.width !== m.width || before.height !== m.height || table.dpr !== m.dpr || before.settings.density !== m.model.settings.density
          || JSON.stringify(before.theme) !== JSON.stringify(m.model.theme)) table.picture = tablePicture();
      }
      table.dpr = m.dpr;
      const w = Math.max(1, Math.round(m.width * m.dpr)), h = Math.max(1, Math.round(m.height * m.dpr));
      table.model.width = m.width; table.model.height = m.height;
      if (table.canvas.width !== w) { table.canvas.width = w; table.painted = false; }
      if (table.canvas.height !== h) { table.canvas.height = h; table.painted = false; }
      table.ctx.setTransform(m.dpr, 0, 0, m.dpr, 0, 0);
      bind(table); break;
    }
    case 'closeRace': {
      const race = races.get(m.id);
      if (race) { record(race, wall(), position(), false, true); unbind(race); race.picture.close(); races.delete(m.id); }
      break;
    }
    case 'racePointer': {
      const race = races.get(m.id);
      if (race?.owner === m.owner) race.picture.hover(m.point);
      return;
    }
    case 'tablePointer': {
      const table = tables.get(m.id);
      if (!table || table.owner !== m.owner) return;
      if (!m.point || !table.state) { table.hoverKey = undefined; if (table.state) table.state.hoverKey = undefined; dirty.add(table); wake(); return; }
      const hit = hitTable(table.state.layout, table.state.rows, { x: m.point[0], y: m.point[1] }, table.model.settings.density === 'compact' ? 22 : 28,
        table.model.headerHeight ?? 28, m.scrollLeft ?? table.scrollLeft, table.model.rankShown, table.model.rankWidth, table.scrollTop);
      if (table.hoverKey !== hit.rowKey) { table.hoverKey = hit.rowKey; table.state.hoverKey = hit.rowKey; dirty.add(table); wake(); }
      post({ type: 'hit', id: m.id, owner: table.owner, key: hit.rowKey ? `${hit.kind}:${hit.rowKey}${hit.column ? `:${hit.column}` : ''}` : hit.column ? `${hit.kind}:${hit.column}` : null });
      return;
    }
    case 'tableRead': tableReads.push(m); break;
    case 'tableScroll': {
      const table = tables.get(m.id);
      if (!table || table.owner !== m.owner) return;
      table.scrollLeft = m.scrollLeft; table.scrollTop = m.scrollTop ?? table.scrollTop; table.model.scrollLeft = m.scrollLeft; table.model.scrollTop = table.scrollTop; dirty.add(table); break;
    }
    case 'inspectRace': {
      const race = races.get(m.id), picture = race?.owner === m.owner && race.shown ? race.picture.inspect(m.target) : undefined;
      post({ type: 'raceInspection', id: m.id, owner: m.owner, request: m.request, seek: anchor.seek, picture }); return;
    }
    case 'inspect': post({ type: 'inspection', request: m.request, update: followed.inspect(m.id) }); return;
    case 'clock':
      const speedChanged = anchor.speed !== m.anchor.speed;
      if (speedChanged || anchor.seek !== m.anchor.seek || anchor.playing !== m.anchor.playing) responding = true;
      if (!m.anchor.playing && (anchor.playing || anchor.position !== m.anchor.position)) followed.paused();
      if (anchor.playing !== m.anchor.playing || !m.anchor.playing && anchor.position !== m.anchor.position) {
        for (const s of [...panels.values(), ...races.values(), ...tables.values()]) if (s.model.fixed === undefined) dirty.add(s);
      }
      if (speedChanged) {
        for (const s of [...panels.values(), ...races.values(), ...tables.values()]) if (s.model.fixed === undefined) {
          invalidate(s);
        }
      }
      if (anchor.playing !== m.anchor.playing || anchor.seek !== m.anchor.seek || speedChanged) cadence.reset();
      if (anchor.seek !== m.anchor.seek) {
        scheduledChanges.length = 0;
        for (const s of [...panels.values(), ...races.values(), ...tables.values()]) {
          if (s.model.fixed !== undefined) continue;
          if (!m.anchor.forwardSeek || (confirmedThrough(s) ?? -Infinity) < m.anchor.position) invalidate(s);
          else s.shown = undefined;
          dirty.add(s);
        }
      }
      if ((anchor.generation ?? 0) !== (m.anchor.generation ?? 0)) {
        unreported = true;
        for (const s of [...panels.values(), ...races.values(), ...tables.values()]) s.shown = undefined;
      }
      anchor = m.anchor; break;
    case 'surface': {
      let s = panels.get(m.id);
      if (!s) {
        const ctx = m.canvas?.getContext('2d');
        if (!ctx || !m.canvas) { post({ type: 'failed', page: m.id, reason: 'Panel drawing surface unavailable' }); break; }
        const pointer = new HeldPointer(key => post({ type: 'hit', id: m.id, owner: s!.owner, key }));
        panels.set(m.id, s = { kind: 'panel', id: m.id, owner: m.owner, model: m.model, canvas: m.canvas, ctx,
          program: new PanelProgram(ctx), pointer, date: new PictureDate() });
      } else unbind(s);
      const w = Math.max(1, Math.round(m.width * m.dpr)), h = Math.max(1, Math.round(m.height * m.dpr));
      const prior = s.model;
      const keep = s.date.drawn && prior.view === m.model.view && prior.follow === m.model.follow &&
        prior.offset === m.model.offset && prior.fixed === m.model.fixed && prior.structure === m.model.structure &&
        s.canvas.width === w && s.canvas.height === h && prior.cells.length === m.model.cells.length &&
        prior.cells.every((c, i) => {
          const next = m.model.cells[i]!;
          return (Object.keys(c) as (keyof typeof c)[]).every(key => c[key] === next[key]) &&
            Object.keys(c).length === Object.keys(next).length;
        });
      if (keep) m.model.cells = prior.cells;
      else invalidate(s);
      s.model = m.model; s.owner = m.owner; s.shown = undefined;
      if (s.canvas.width !== w) s.canvas.width = w;
      if (s.canvas.height !== h) s.canvas.height = h;
      s.ctx.setTransform(m.dpr, 0, 0, m.dpr, 0, 0);
      bind(s); break;
    }
    case 'pointer': {
      const s = panels.get(m.id);
      if (!s || s.owner !== m.owner) return;
      if (!m.point || !s.shown || !s.date.presentableAt(position())) s.pointer.clear();
      else { s.pointer.move(...m.point); s.pointer.present((x, y) => s.program.hit(x, y)); }
      return;
    }
    case 'dateFace': {
      let face = faces.get(m.id);
      if (!face) {
        const ctx = m.canvas?.getContext('2d');
        if (!ctx) break;
        faces.set(m.id, face = { ctx, model: m.face, width: m.width, height: m.height, dpr: m.dpr });
      }
      Object.assign(face, { model: m.face, width: m.width, height: m.height, dpr: m.dpr, drawn: undefined });
      face.ctx.canvas.width = Math.round(m.width * m.dpr); face.ctx.canvas.height = Math.round(m.height * m.dpr);
      break;
    }
    case 'detach': {
      faces.delete(m.id);
      const s = panels.get(m.id);
      if (s) { record(s, wall(), position(), false, true); unbind(s); s.pointer.clear(); panels.delete(m.id); }
      const table = tables.get(m.id);
      if (table) { record(table, wall(), position(), false, true); unbind(table); tables.delete(m.id); }
      break;
    }
  }
  wake();
};
