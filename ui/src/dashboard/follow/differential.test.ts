import assert from 'node:assert/strict';
import { test } from 'node:test';
import { Lane } from '../../draw/lane.ts';
import { fieldsView } from '../../draw/views/fields.ts';
import { Page, type Bitmap, type PageMessage } from './page.ts';
import { splitForLanes } from './split.ts';
import { cellValue, parseAnswer, rowKey } from './message.ts';
import { answerBody } from './messages.fixture.ts';

interface Fake extends Bitmap { closed: boolean }
const NATIVE = 3, ROUTE = 1, LANES = 2, HOURS = 2000, ROWS = 6;

function rng(seed: number) {
  return (n: number) => { seed ^= seed << 13; seed ^= seed >>> 17; seed ^= seed << 5; return (seed >>> 0) % n; };
}

interface Truth {
  rows(sub: number, hour: number): bigint[] | null;
  reason(sub: number, hour: number): string | null;
  same(sub: number, from: number, to: number): boolean;
}

function recording(rnd: (n: number) => number, subs: readonly number[]): Truth {
  const changes = new Map<string, number[]>(), unrecorded = new Map<number, [number, number][]>();
  for (const sub of subs) {
    for (let row = 0; row < ROWS; row++) {
      const at = [0];
      for (let hour = 0; hour < HOURS; hour += 1 + rnd(40)) if (hour) at.push(hour);
      changes.set(`${sub}:${row}`, at);
    }
    const spans: [number, number][] = [];
    for (let hour = rnd(300); hour < HOURS; hour += 200 + rnd(400)) { const length = 5 + rnd(40); spans.push([hour, hour + length]); hour += length; }
    unrecorded.set(sub, spans);
  }
  const startOf = (sub: number, row: number, hour: number) => { let lo = 0; for (const at of changes.get(`${sub}:${row}`)!) if (at <= hour) lo = at; return lo; };
  const reason = (sub: number, hour: number) => {
    const span = unrecorded.get(sub)!.find(([first, last]) => first <= hour && hour <= last);
    return span ? `sub ${sub} was not recorded from ${span[0]}` : null;
  };
  const rows = (sub: number, hour: number) => reason(sub, hour) === null
    ? Array.from({ length: ROWS }, (_, row) => BigInt(sub * 1_000_000 + row * 10_000 + startOf(sub, row, hour))) : null;
  const key = (sub: number, hour: number) => JSON.stringify([reason(sub, hour), rows(sub, hour)?.map(String)]);
  return { rows, reason, same: (sub, from, to) => { const first = key(sub, from); for (let hour = from + 1; hour <= to; hour++) if (key(sub, hour) !== first) return false; return true; } };
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

const KEYS = Array.from({ length: ROWS }, (_, row) => BigInt(row + 1));
const flush = async () => { for (let i = 0; i < 4; i++) await new Promise<void>(resolve => setImmediate(resolve)); };

interface Fault { property: string; detail: string }

async function run(seed: number): Promise<{ faults: Fault[]; frames: number; installed: number; failed: number }> {
  const rnd = rng(seed);
  const widgets = [{ id: 1, subs: [3] }, { id: 2, subs: [2] }, { id: 3, subs: [3, 5] }, { id: 4, subs: [4] }];
  const subs = [...new Set(widgets.flatMap(widget => widget.subs))];
  const lag = new Map(subs.map(sub => [sub, rnd(3) === 0 ? 1 + rnd(5) : 0]));
  const truth = recording(rnd, subs);
  const lanes: Lane<Fake>[] = [], failures = new Map<number, string>();
  const page = new Page<Fake>({ lanes: LANES, post: (lane, m: PageMessage) => {
    for (const [index, target] of lanes.entries()) {
      if (lane !== 'all' && lane !== index) continue;
      if (m.type === 'frame') target.frame(m); else if (m.type === 'stream') target.stream(m.route, m.seek, m.target); else target.fail(m.route);
    }
  } });
  for (let index = 0; index < LANES; index++) lanes.push(new Lane<Fake>({
    lane: index, lanes: LANES, views: () => fieldsView(() => undefined),
    crop: async sources => sources.map((): Fake => ({ closed: false, width: 1, close() { this.closed = true; } })),
    present: presentation => page.offer(presentation), release: () => {}, later: () => {},
    fail: reason => { failures.set(index, reason); for (const widget of widgets) if (widget.id % LANES === index) page.name(widget.id, 'failed', reason); },
  }));
  for (const widget of widgets) {
    const lane = widget.id % LANES;
    for (const sub of widget.subs) { assert.equal(sub % LANES, lane, 'a subscription id carries the lane of its widget'); lanes[lane]!.follow(sub, 'followed'); lanes[lane]!.bindRoute(sub, ROUTE, NATIVE); }
    lanes[lane]!.mount(widget.id, widget.subs, 'fields');
    lanes[lane]!.bind(widget.id, { generation: 1, surfaces: [], offset: 0, pin: null });
    page.mount(widget.id, { route: ROUTE, lane, surfaces: [] });
  }
  let seek = 0, nativeHead = -1, clock = 0, playing = false, speed = 24, step = 0;
  const queue: { due: number; lanes: (Uint8Array | null)[] }[] = [];
  let tick = 0, lastDue = 0;
  const sent = new Map<number, string | bigint[]>();
  const emit = (bytes: Uint8Array) => {
    const split = splitForLanes(bytes, LANES);
    if (split.kind === 'failure') return;
    lastDue = Math.max(lastDue, tick + rnd(4));
    queue.push({ due: lastDue, lanes: split.lanes });
  };
  const known = (sub: number, hour: number) => hour - lag.get(sub)!;
  const answers = (hour: number, whole: boolean) => subs.flatMap(sub => {
    const at = known(sub, hour);
    if (at < 0) return [];
    const reason = truth.reason(sub, at), rows = truth.rows(sub, at), last = sent.get(sub);
    const limit = lag.get(sub) ? [{ id: sub, hour: at, body: new Uint8Array() }] : [];
    if (reason !== null) {
      if (!whole && last === reason) return limit;
      sent.set(sub, reason);
      return [{ id: sub, hour: at, body: answerBody({ notRecorded: reason }) }, ...limit];
    }
    if (!whole && Array.isArray(last) && rnd(10)) {
      const changed = KEYS.flatMap((_key, row) => last[row] !== rows![row] ? [row] : []);
      sent.set(sub, rows!);
      if (!changed.length) return limit;
      return [{ id: sub, hour: at, body: answerBody({ keys: changed.map(row => KEYS[row]!), columns: 1, values: changed.map(row => rows![row]!), changed: true }) }, ...limit];
    }
    sent.set(sub, rows!);
    return [{ id: sub, hour: at, body: answerBody({ keys: KEYS, columns: 1, values: rows! }) }, ...limit];
  });
  const native = (to: number) => {
    while (nativeHead < to && nativeHead < HOURS - 1) { nativeHead++; emit(message(nativeHead, seek, false, answers(nativeHead, false))); }
  };
  const faults: Fault[] = [];
  const shown = new Map<number, number | undefined>();
  let frames = 0, installed = 0, failed = 0;
  const failAt = rnd(2) === 0 ? 100 + rnd(150) : -1;
  page.publish(0, { playing: false, seek: 0, step: 0, speed });
  sent.clear();
  emit(message(0, 0, true, answers(0, true)));
  nativeHead = 0;
  for (; tick < 300; tick++) {
    const action = rnd(20);
    let sought = false;
    if (tick === failAt) {
      try { lanes[1]!.receive('follow', message(nativeHead, seek, false, []).subarray(0, 20)); }
      catch (error) { lanes[1]!['host'].fail(error instanceof Error ? error.message : String(error)); }
    }
    if (action === 0) {
      seek++; sought = true; clock = rnd(HOURS - 100); playing = false;
      nativeHead = clock;
      sent.clear();
      emit(message(clock, seek, true, answers(clock, true)));
    } else if (action === 1) playing = !playing;
    else if (action === 2) speed = [24, 168, 720, 1440][rnd(4)]!;
    else if (action === 3 && !playing) clock = Math.min(HOURS - 2, clock + 1);
    if (playing) { step = Math.max(1, Math.round(speed / 60) + rnd(3) - 1); clock = Math.min(HOURS - 2, clock + step); }
    native(Math.min(HOURS - 1, clock + (playing ? 2 * step + rnd(30) : rnd(3))));
    for (const held of queue.filter(item => item.due <= tick)) held.lanes.forEach((bytes, lane) => { if (bytes) lanes[lane]!.receive('follow', bytes); });
    queue.splice(0, queue.length, ...queue.filter(item => item.due > tick));
    page.publish(clock, { playing, seek, step, speed });
    await flush();
    if (!playing) { page.frame(); await flush(); page.frame(); }
    frames++;
    for (const widget of widgets) {
      const view = page.view(widget.id), t = view.target, last = shown.get(widget.id), lane = widget.id % LANES, at = `seed ${seed} tick ${tick} widget ${widget.id}`;
      if (failures.has(lane)) {
        failed++;
        if (view.mode !== 'failed' || view.reason !== failures.get(lane)) faults.push({ property: 'failure', detail: `${at} shows ${view.mode} after its lane failed: ${failures.get(lane)}` });
        continue;
      }
      if (view.mode === 'picture') {
        installed++;
        const h = view.h!, through = view.through!, c = page.widgets.get(widget.id)!.installed!.c;
        if (h > t) faults.push({ property: 'ahead', detail: `${at} h ${h} > t ${t}` });
        if (through > t || through < h) faults.push({ property: 'through', detail: `${at} h ${h} through ${through} t ${t}` });
        for (const sub of widget.subs) {
          const body = view.fields?.[`answer:${sub}`] as Uint8Array | undefined, answer = body && parseAnswer(body);
          const reason = truth.reason(sub, h), rows = truth.rows(sub, h);
          if (!answer) { faults.push({ property: 'value', detail: `${at} sub ${sub} has no answer` }); continue; }
          if (answer.kind === 'notRecorded') {
            if (answer.reason !== reason) faults.push({ property: 'reason', detail: `${at} sub ${sub} at ${h} says "${answer.reason}", the recording ${reason === null ? 'has values' : `"${reason}"`}` });
          } else if (rows === null) faults.push({ property: 'reason', detail: `${at} sub ${sub} at ${h} shows values where the recording says "${reason}"` });
          else {
            const values = Array.from({ length: answer.rows }, (_, row) => [rowKey(answer, row), cellValue(answer, row)] as const);
            const want = KEYS.map((key, row) => [key, rows[row]!] as const);
            if (JSON.stringify(values.map(String)) !== JSON.stringify(want.map(String))) faults.push({ property: 'value', detail: `${at} sub ${sub} at ${h}: ${values.join(';')} != ${want.join(';')}` });
          }
          const certified = Math.min(c, known(sub, nativeHead), HOURS - 1);
          if (certified > h && !truth.same(sub, h, certified)) faults.push({ property: 'coverage', detail: `${at} sub ${sub} certified ${h}-${c} but the recording changes within` });
        }
        const dim = (!playing && t > c);
        if (view.dim !== dim) faults.push({ property: 'dim', detail: `${at} dim ${view.dim} want ${dim}` });
        shown.set(widget.id, view.id);
      } else {
        if (last !== undefined && !sought && view.mode === 'reading') faults.push({ property: 'blank', detail: `${at} went to reading without a seek` });
        shown.set(widget.id, undefined);
      }
    }
  }
  return { faults, frames, installed, failed };
}

test('differential: every committed frame of every widget agrees with an independent read of the recording: tables, changed rows, limits, reasons and lane failures', async () => {
  const faults: Fault[] = [];
  let frames = 0, installed = 0, failed = 0;
  for (let seed = 1; seed <= 60; seed++) {
    const result = await run(0x51ed + seed * 7919);
    faults.push(...result.faults); frames += result.frames; installed += result.installed; failed += result.failed;
  }
  const byProperty = new Map<string, Fault[]>();
  for (const fault of faults) { let list = byProperty.get(fault.property); if (!list) byProperty.set(fault.property, list = []); list.push(fault); }
  for (const property of ['ahead', 'through', 'value', 'reason', 'coverage', 'dim', 'blank', 'failure'])
    assert.deepEqual((byProperty.get(property) ?? []).slice(0, 3).map(fault => fault.detail), [], `${property}: ${byProperty.get(property)?.length ?? 0} faults`);
  assert.ok(installed > frames * 2, `pictures were installed: ${installed} widget frames of ${frames * 4}`);
  assert.ok(failed > 0, 'a lane failed in some runs, and its widgets were checked for it');
});
