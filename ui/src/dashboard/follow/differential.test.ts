import assert from 'node:assert/strict';
import { test } from 'node:test';
import { Lane } from '../../draw/lane.ts';
import { fieldsView } from '../../draw/views/fields.ts';
import { Page, type Bitmap, type PageMessage } from './page.ts';
import { splitForLanes } from './split.ts';

interface Fake extends Bitmap { closed: boolean }
const NATIVE = 3, ROUTE = 1, LANES = 2, HOURS = 2000;

function rng(seed: number) {
  return (n: number) => { seed ^= seed << 13; seed ^= seed >>> 17; seed ^= seed << 5; return (seed >>> 0) % n; };
}

function recording(rnd: (n: number) => number, subs: readonly number[]) {
  const changes = new Map<number, number[]>();
  for (const sub of subs) {
    const at = [0];
    for (let hour = 0; hour < HOURS; hour += 1 + rnd(30)) if (hour) at.push(hour);
    changes.set(sub, at);
  }
  const startOf = (sub: number, hour: number) => { const list = changes.get(sub)!; let lo = 0; for (const at of list) if (at <= hour) lo = at; return lo; };
  return { valueAt: (sub: number, hour: number) => BigInt(sub * 100_000 + startOf(sub, hour)), changesAt: (sub: number, hour: number) => changes.get(sub)!.includes(hour) };
}

function answerBody(sub: number, value: bigint): Uint8Array {
  const out = new Uint8Array(72), view = new DataView(out.buffer);
  view.setUint8(0, 1); view.setUint8(1, 1); view.setUint32(4, 1, true); view.setUint32(8, 1, true); view.setUint32(12, 0, true);
  view.setBigUint64(16, BigInt(sub), true); view.setBigUint64(24, 0n, true); view.setBigInt64(32, value, true);
  view.setUint32(48, 0xffff_ffff, true); view.setUint32(56, 0xffff_ffff, true); view.setUint32(64, 0, true);
  return out;
}

function message(hour: number, seek: number, reset: boolean, answers: { id: number; hour: number; body: Uint8Array }[]): Uint8Array {
  const size = 24 + answers.reduce((sum, answer) => sum + 16 + answer.body.length, 0), out = new Uint8Array(size), view = new DataView(out.buffer);
  view.setUint32(0, 0x334c4f46, true); view.setUint32(4, NATIVE << 1 | (reset ? 1 : 0), true); view.setInt32(8, hour, true);
  view.setUint32(12, answers.length, true); view.setUint32(16, seek, true);
  let at = 24;
  for (const answer of answers) {
    view.setUint32(at, answer.id, true); view.setInt32(at + 4, answer.hour, true); view.setUint32(at + 8, answer.body.length, true);
    out.set(answer.body, at + 16); at += 16 + answer.body.length;
  }
  return out;
}

const valueOf = (body: unknown) => body instanceof Uint8Array ? new DataView(body.buffer, body.byteOffset).getBigInt64(32, true) : null;
const flush = async () => { for (let i = 0; i < 4; i++) await new Promise<void>(resolve => setImmediate(resolve)); };

interface Fault { property: string; detail: string }

async function run(seed: number): Promise<{ faults: Fault[]; frames: number; installed: number; ages: number[] }> {
  const rnd = rng(seed);
  const widgets = [{ id: 1, subs: [3] }, { id: 2, subs: [2] }, { id: 3, subs: [3, 5] }, { id: 4, subs: [4] }];
  const subs = [...new Set(widgets.flatMap(widget => widget.subs))];
  const truth = recording(rnd, subs);
  const lanes: Lane<Fake>[] = [];
  const page = new Page<Fake>({ lanes: LANES, post: (lane, m: PageMessage) => {
    for (const [index, target] of lanes.entries()) {
      if (lane !== 'all' && lane !== index) continue;
      if (m.type === 'frame') target.frame(m); else if (m.type === 'stream') target.stream(m.route, m.seek, m.target); else target.fail(m.route);
    }
  } });
  for (let index = 0; index < LANES; index++) lanes.push(new Lane<Fake>({
    lane: index, lanes: LANES, views: () => fieldsView(() => undefined),
    crop: async sources => sources.map((): Fake => ({ closed: false, width: 1, close() { this.closed = true; } })),
    present: presentation => page.offer(presentation), release: () => {}, charge: () => {}, fail: () => {}, later: () => {},
  }));
  for (const widget of widgets) {
    const lane = widget.id % LANES;
    for (const sub of widget.subs) { assert.equal(sub % LANES, lane, 'a subscription id carries the lane of its widget'); lanes[lane]!.follow(sub, 'followed'); lanes[lane]!.bindRoute(sub, ROUTE, NATIVE); }
    lanes[lane]!.mount(widget.id, widget.subs, 'fields');
    page.mount(widget.id, { route: ROUTE, lane, surfaces: [] });
  }
  let seek = 0, head = 0, nativeHead = -1, clock = 0, playing = false, speed = 24, step = 0;
  const queue: { due: number; lanes: (Uint8Array | null)[] }[] = [];
  let tick = 0, lastDue = 0;
  const emit = (bytes: Uint8Array) => {
    const split = splitForLanes(bytes, LANES);
    if (split.kind === 'failure') return;
    lastDue = Math.max(lastDue, tick + rnd(4));
    queue.push({ due: lastDue, lanes: split.lanes });
  };
  const native = (to: number) => {
    while (nativeHead < to && nativeHead < HOURS - 1) {
      nativeHead++;
      const answers = subs.filter(sub => truth.changesAt(sub, nativeHead)).map(sub => ({ id: sub, hour: nativeHead, body: answerBody(sub, truth.valueAt(sub, nativeHead)) }));
      emit(message(nativeHead, seek, false, answers));
    }
  };
  const faults: Fault[] = [], ages: number[] = [];
  const shown = new Map<number, number | undefined>();
  let frames = 0, installed = 0;
  page.publish(0, { playing: false, seek: 0, step: 0, speed });
  emit(message(0, 0, true, subs.map(sub => ({ id: sub, hour: 0, body: answerBody(sub, truth.valueAt(sub, 0)) }))));
  nativeHead = 0;
  for (; tick < 300; tick++) {
    const action = rnd(20);
    let sought = false;
    if (action === 0) {
      seek++; sought = true; clock = rnd(HOURS - 100); playing = false;
      nativeHead = clock;
      emit(message(clock, seek, true, subs.map(sub => ({ id: sub, hour: clock, body: answerBody(sub, truth.valueAt(sub, clock)) }))));
    } else if (action === 1) playing = !playing;
    else if (action === 2) speed = [24, 168, 720, 1440][rnd(4)]!;
    else if (action === 3 && !playing) clock = Math.min(HOURS - 2, clock + 1);
    if (playing) { step = Math.max(1, Math.round(speed / 60) + rnd(3) - 1); clock = Math.min(HOURS - 2, clock + step); }
    head = clock;
    native(Math.min(HOURS - 1, head + (playing ? 2 * step + rnd(30) : rnd(3))));
    for (const held of queue.filter(item => item.due <= tick)) held.lanes.forEach((bytes, lane) => { if (bytes) lanes[lane]!.receive('follow', bytes); });
    queue.splice(0, queue.length, ...queue.filter(item => item.due > tick));
    page.publish(clock, { playing, seek, step, speed });
    await flush();
    if (!playing) { page.frame(); await flush(); page.frame(); }
    frames++;
    for (const widget of widgets) {
      const view = page.view(widget.id), t = view.target, last = shown.get(widget.id);
      if (view.mode === 'picture') {
        installed++;
        const h = view.h!, through = view.through!, c = page.widgets.get(widget.id)!.installed!.c;
        if (h > t) faults.push({ property: 'ahead', detail: `seed ${seed} tick ${tick} widget ${widget.id} h ${h} > t ${t}` });
        if (through > t || through < h) faults.push({ property: 'through', detail: `seed ${seed} tick ${tick} widget ${widget.id} h ${h} through ${through} t ${t}` });
        for (const sub of widget.subs) {
          const value = valueOf(view.fields?.[`answer:${sub}`]);
          if (value !== truth.valueAt(sub, h)) faults.push({ property: 'value', detail: `seed ${seed} tick ${tick} widget ${widget.id} sub ${sub} at ${h}: ${value} != ${truth.valueAt(sub, h)}` });
          for (let hour = h; hour <= Math.min(c, nativeHead, HOURS - 1); hour++) if (truth.valueAt(sub, hour) !== value) {
            faults.push({ property: 'coverage', detail: `seed ${seed} tick ${tick} widget ${widget.id} sub ${sub} certified ${h}-${c} but changes at ${hour}` }); break;
          }
        }
        const dim = (!playing && t > c);
        if (view.dim !== dim) faults.push({ property: 'dim', detail: `seed ${seed} tick ${tick} widget ${widget.id} dim ${view.dim} want ${dim}` });
        ages.push(t - through);
        shown.set(widget.id, view.id);
      } else {
        if (last !== undefined && !sought && view.mode === 'reading') faults.push({ property: 'blank', detail: `seed ${seed} tick ${tick} widget ${widget.id} went to reading without a seek` });
        shown.set(widget.id, undefined);
      }
    }
  }
  return { faults, frames, installed, ages };
}

test('differential: every committed frame of every widget agrees with an independent read of the recording', async () => {
  const faults: Fault[] = [];
  let frames = 0, installed = 0;
  const ages: number[] = [];
  for (let seed = 1; seed <= 60; seed++) {
    const result = await run(0x51ed + seed * 7919);
    faults.push(...result.faults); frames += result.frames; installed += result.installed; ages.push(...result.ages);
  }
  const byProperty = new Map<string, Fault[]>();
  for (const fault of faults) { let list = byProperty.get(fault.property); if (!list) byProperty.set(fault.property, list = []); list.push(fault); }
  for (const property of ['ahead', 'through', 'value', 'coverage', 'dim', 'blank'])
    assert.deepEqual((byProperty.get(property) ?? []).slice(0, 3).map(fault => fault.detail), [], `${property}: ${byProperty.get(property)?.length ?? 0} faults`);
  assert.ok(installed > frames * 3, `pictures were installed: ${installed} widget frames of ${frames * 4}`);
});
