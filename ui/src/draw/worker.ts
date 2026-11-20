import { PresentationRate, DisplayRate } from './frameRate.ts';
import { anchorHour } from './kit.ts';
import type { ClockAnchor, FromWorker, ToWorker } from './kit';
import { Followed } from './followed.ts';
import type { FollowItem } from '../dashboard/follow/message';
import { stretch } from './weakCpu.ts';
import { PictureDate, pictureHour } from './pictureDate.ts';
import { HeldPointer } from './pointer.ts';
import { PanelProgram } from './retained/panel.ts';
import { panelViews, raceViews } from './panels/views.ts';
import type { PanelModel } from './panels/view';
import type { DrawSample, Present } from './notifications';

const post = (message: FromWorker) => self.postMessage(message);
const wall = () => performance.timeOrigin + performance.now();
interface Panel {
  kind: 'panel'; id: number; owner: number; model: PanelModel; canvas: OffscreenCanvas; ctx: OffscreenCanvasRenderingContext2D;
  date: PictureDate; program: PanelProgram; pointer: HeldPointer; shown?: boolean; reply?: unknown; through?: number; nextText?: number;
}
interface Race {
  kind: 'race'; id: number; owner: number; model: Extract<ToWorker, { type: 'race' }>['model']; picture: InstanceType<(typeof raceViews)[keyof typeof raceViews]>; shown?: boolean; through?: number;
  reply?: unknown; nextText?: number; confirmed?: number; since?: number;
}
type Surface = Panel | Race;
const panels = new Map<number, Panel>(), races = new Map<number, Race>();
const subscriptions = new Map<number, Set<Surface>>(), dirty = new Set<Surface>();
const scheduledChanges: { at: number; surface: Surface }[] = [];
let anchor: ClockAnchor = { position: 0, at: 0, speed: 1, playing: false, lastHour: 0, seek: 0 };
let rate = 1, cap = 60, frameNumber = 0;
let audit: DrawSample[] | undefined;
let auditOffset = 0;
const cadence = new PresentationRate(), display = new DisplayRate();
const position = () => anchorHour(anchor, wall());
let scheduled = false;
function wake() { if (!scheduled) { scheduled = true; requestAnimationFrame(frame); } }
let dataPort: MessagePort | undefined;
const followed = new Followed(what => dataPort?.postMessage(what), id => {
  for (const s of subscriptions.get(id) ?? []) dirty.add(s);
});

function unbind(s: Surface) {
  const id = s.model.follow;
  if (id !== undefined) {
    const dependents = subscriptions.get(id);
    dependents?.delete(s);
    if (dependents?.size === 0) subscriptions.delete(id);
  }
  const due = scheduledChanges.findIndex(q => q.surface === s);
  if (due >= 0) scheduledChanges.splice(due, 1);
  dirty.delete(s);
}
function bind(s: Surface) {
  if (s.model.follow !== undefined) {
    let dependents = subscriptions.get(s.model.follow);
    if (!dependents) subscriptions.set(s.model.follow, dependents = new Set());
    dependents.add(s);
  }
  dirty.add(s);
}
function invalidate(s: Surface) {
  if (s.kind === 'panel') {
    s.program.clear(); s.pointer.clear(); s.date.drawn = undefined; s.reply = undefined;
  } else s.picture.clear();
  s.through = s.shown = undefined; dirty.add(s);
}
function confirmedThrough(s: Surface) {
  if (s.kind === 'race') return s.confirmed;
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
function report(s: Surface, head: number, notices: Present[]) {
  const shown = s.through !== undefined;
  if (s.shown === shown) return;
  s.shown = shown;
  if (s.kind === 'race') {
    post({ type: 'racePresent', id: s.id, owner: s.owner, seek: anchor.seek, generation: anchor.generation ?? 0,
      head: s.model.fixed ?? head, hour: s.through ?? null, at: wall() });
  } else notices.push({ id: s.id, owner: s.owner, seek: anchor.seek, generation: anchor.generation ?? 0,
    head: s.model.fixed ?? head, date: s.date.drawn });
}

function record(s: Surface, at: number, head: number, sample = false, detached = false) {
  if (!audit || !s.model.widget) return;
  const target = Math.floor(s.model.fixed ?? head);
  const entry = s.model.follow === undefined ? undefined : followed.read(s.model.follow);
  const input = s.model.follow === undefined ? s.model.date : entry?.date;
  const eligible = s.kind === 'panel' && s.model.clocked && s.model.reply ? target + s.model.offset :
    pictureHour(input, target + s.model.offset, s.model.fixed === undefined ? undefined : target + s.model.offset);
  let confirmed = s.kind === 'panel' ? s.date.drawn?.through : s.confirmed;
  if (s.reply && s.reply === entry?.reply && !entry.hasFollowedCells)
    confirmed = entry.date.through - s.model.offset;
  const hour = s.through === undefined ? undefined : Math.max(s.through,
    Math.min(target, confirmed ?? s.through, (s.nextText ?? Infinity) - 1));
  audit.push({ t: 'picture', widget: s.model.widget, surface: s.model.view,
    placement: s.id, owner: s.owner, at, seek: anchor.seek, head: target, hour: detached ? null : hour ?? null, renderer: 'worker',
    since: s.kind === 'panel' ? s.date.drawn?.since : s.since,
    through: confirmed === undefined ? undefined : Math.min(confirmed, (s.nextText ?? Infinity) - 1),
    eligibleHour: eligible === null ? null : eligible - s.model.offset, inputThrough: entry && entry.date.through - s.model.offset,
    receivedThrough: s.model.follow === undefined ? undefined : followed.receivedThrough(s.model.follow),
    offset: s.model.offset, fixedHour: s.model.fixed, detached,
    ...(sample && hour !== undefined && { rows: s.kind === 'race' ? s.picture.rows() : s.program.snapshot() }) });
}
function frame(at: number) {
  scheduled = false;
  if (anchor.playing && !cap) {
    const hz = display.sample(at);
    if (hz !== undefined) post({ type: 'displayRate', hz });
  }
  const began = performance.now(), sampleTime = performance.timeOrigin + began;
  const head = anchorHour(anchor, sampleTime);
  if (anchor.playing) {
    const step = anchor.speed / cap, through = followed.stepHour;
    // At integral step sizes, take the first display frame after each native step becomes eligible.
    const start = through !== undefined && Number.isInteger(step) && step > 0 && head < anchor.lastHour
      ? through + Math.floor((head - through) / step) * step : undefined;
    const boundary = start === undefined ? undefined : anchor.at - performance.timeOrigin + (start - anchor.position) * 1000 / anchor.speed;
    if (!cadence.due(began, cap, boundary)) { wake(); return; }
  }
  followed.drain(head);
  const updates = [...followed.structures(anchor.playing)];
  if (updates.length) post({ type: 'structure', updates });
  while (scheduledChanges.length && scheduledChanges.at(-1)!.at <= head) dirty.add(scheduledChanges.pop()!.surface);
  const changed = audit && new Set<string>();
  const notices: Present[] = [];
  const applied = audit ? performance.now() : began;
  for (const s of dirty) {
    if (paint(s, head) && s.model.widget) changed?.add(s.model.widget);
    report(s, head, notices);
  }
  if (notices.length) post({ type: 'present', surfaces: notices });
  const end = performance.now(), workMs = end - began;
  stretch(workMs, rate, end);
  if (audit) {
    const presented = wall();
    const sample = frameNumber++ % 60 === 0;
    for (const s of panels.values()) record(s, presented, head, sample);
    for (const s of races.values()) record(s, presented, head, sample);
    audit.push({ t: 'workerFrame', at: presented, head, seek: anchor.seek, playing: anchor.playing, workMs,
      applyMs: applied - began, paintMs: end - applied, rate, changed: changed!.size });
  }
  dirty.clear();
  if (anchor.playing && (panels.size || races.size || followed.size)) wake();
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
        for (const item of data) if (item.kind === 'failure') post({ type: 'followFailure', reason: item.reason });
        followed.land(data, anchor.playing ? post : undefined); wake();
      };
      return;
    case 'add': followed.add(m.id, m.branch, m.product); return;
    case 'remove': followed.remove(m.id); break;
    case 'seek': followed.seek(m.branch, m.seek, m.retainThrough); break;
    case 'bind': followed.bind(m.branch, m.route, m.seek); break;
    case 'options':
      rate = m.rate;
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
    case 'inspectRace': {
      const race = races.get(m.id), picture = race?.owner === m.owner && race.shown ? race.picture.inspect(m.target) : undefined;
      post({ type: 'raceInspection', id: m.id, owner: m.owner, request: m.request, seek: anchor.seek, picture }); return;
    }
    case 'inspect': post({ type: 'inspection', request: m.request, reading: followed.snapshot(m.id) }); return;
    case 'clock':
      const speedChanged = anchor.speed !== m.anchor.speed;
      if (anchor.playing && !m.anchor.playing) followed.paused();
      if (anchor.playing !== m.anchor.playing || !m.anchor.playing && anchor.position !== m.anchor.position) {
        for (const s of [...panels.values(), ...races.values()]) if (s.model.fixed === undefined) dirty.add(s);
      }
      if (speedChanged) {
        // A host hides moving canvases while this anchor crosses the worker boundary.
        // Repaint them under the new rate before reporting them visible again.
        for (const s of [...panels.values(), ...races.values()]) if (s.model.fixed === undefined) {
          invalidate(s);
        }
      }
      if (anchor.playing !== m.anchor.playing || anchor.seek !== m.anchor.seek || speedChanged) cadence.reset();
      if (anchor.seek !== m.anchor.seek) {
        scheduledChanges.length = 0;
        for (const s of [...panels.values(), ...races.values()]) {
          if (s.model.fixed !== undefined) continue;
          if (!m.anchor.forwardSeek || (confirmedThrough(s) ?? -Infinity) < m.anchor.position) invalidate(s);
          else s.shown = undefined;
          dirty.add(s);
        }
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
    case 'detach': {
      const s = panels.get(m.id);
      if (s) { record(s, wall(), position(), false, true); unbind(s); s.pointer.clear(); panels.delete(m.id); }
      break;
    }
  }
  wake();
};
