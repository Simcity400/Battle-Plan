import assert from 'node:assert/strict';
import { test } from 'node:test';
import { encode } from '@msgpack/msgpack';
import { Lane } from '../../draw/lane.ts';
import { fieldsView } from '../../draw/views/fields.ts';
import { Page, type Bitmap, type PageMessage } from './page.ts';
import { splitForLanes } from './split.ts';
import { cellValue, DATED_PANEL_VERSION, parseAnswer, rowKey } from './message.ts';
import { answerBody } from './messages.fixture.ts';
import { createLaneView, type LaneBinding, type LaneFamily } from '../../families/laneRuntime.ts';
import { driveLane, installRecordingCanvas, LANE_THEME, type RecordedCanvas } from '../../families/laneRuntime.fixture.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { BoundedReadView, FamilyProjection } from '../../families/contract.ts';

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

function panelPacket(records: { id: number; hour: number; kind: 'data' | 'completed'; data?: unknown; stepped?: boolean }[], seek: number): Uint8Array {
  const parts = records.map(record => {
    const body = record.kind === 'data' ? encode(record.data) : new Uint8Array();
    const out = new Uint8Array((16 + body.length + 7) & ~7), view = new DataView(out.buffer);
    view.setUint32(0, record.kind === 'completed' ? 0 : record.id, true);
    view.setInt32(4, record.hour, true);
    view.setUint8(8, record.kind === 'data' ? 1 : 4);
    view.setUint8(9, record.stepped ? 1 : 0);
    view.setUint32(12, body.length, true);
    out.set(body, 16);
    return out;
  });
  const out = new Uint8Array(24 + parts.reduce((sum, part) => sum + part.length, 0)), view = new DataView(out.buffer);
  view.setUint32(0, 0x324c4e50, true);
  view.setUint16(4, DATED_PANEL_VERSION, true);
  view.setUint32(8, records.length, true);
  view.setUint32(16, NATIVE, true);
  view.setUint32(20, seek, true);
  let at = 24;
  for (const part of parts) { out.set(part, at); at += part.length; }
  return out;
}

test('differential: panels with reused live cells survive seeks, pauses, speed changes, late joins and unmounts without lane faults', async () => {
  for (let seed = 1; seed <= 20; seed++) {
    const rnd = rng(0x91a7 + seed * 3571);
    const lanes: Lane<Fake>[] = [], laneFailures: string[] = [];
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
      fail: reason => laneFailures.push(`seed ${seed} lane ${index}: ${reason}`),
    }));
    const CELL_BASE = 0x8000_0000;
    const active = new Map<number, { sub: number; cell: number; lane: number; lastVal: bigint | null }>();
    let nextWidget = 1;
    const joinWidget = (seek: number) => {
      const id = nextWidget++, lane = id % LANES, sub = id * LANES + lane, cell = (CELL_BASE | (id * LANES + lane)) >>> 0;
      lanes[lane]!.follow(sub, 'panel', 'demo');
      lanes[lane]!.bindRoute(sub, ROUTE, NATIVE, seek);
      lanes[lane]!.mount(id, [sub], 'fields');
      lanes[lane]!.bind(id, { generation: 1, surfaces: [], offset: 0, pin: null });
      page.mount(id, { route: ROUTE, lane, surfaces: [] });
      active.set(id, { sub, cell, lane, lastVal: null });
    };
    const leaveWidget = (id: number) => {
      const w = active.get(id);
      if (!w) return;
      lanes[w.lane]!.unmount(id);
      lanes[w.lane]!.unfollow(w.sub);
      page.unmount(id);
      active.delete(id);
    };
    const panelData = (cell: number, hour: number) => ({
      shape: `S`, hour,
      live: { groups: [{ table: 1, columns: [0], keys: ['1'], follow: cell }], rows: ['r0'], kinds: [{ id: 'k', op: 'value', scale: 1 }], cells: [0, 0, 1, 0, 0, 0] },
    });
    joinWidget(0);
    joinWidget(0);
    let seek = 0, clock = 0, playing = false, speed = 24, step = 0;
    const sendReset = (hour: number) => {
      const answers = [...active.values()].map(w => {
        const v = BigInt(hour * 10 + w.sub);
        w.lastVal = v;
        return { id: w.cell, hour, body: answerBody({ keys: [1n], columns: 1, values: [v] }) };
      });
      const fSplit = splitForLanes(message(hour, seek, true, answers), LANES);
      assert.ok(fSplit.kind === 'follow');
      fSplit.lanes.forEach((b, l) => { if (b) lanes[l]!.receive('follow', b); });
      const pRecords = [
        ...[...active.values()].map(w => ({ id: w.sub, hour, kind: 'data' as const, stepped: true, data: panelData(w.cell, hour) })),
        { id: 0, hour, kind: 'completed' as const },
      ];
      const pSplit = splitForLanes(panelPacket(pRecords, seek), LANES);
      assert.ok(pSplit.kind === 'panels');
      pSplit.lanes.forEach((b, l) => { if (b) lanes[l]!.receive('panels', b); });
    };
    page.publish(0, { playing: false, seek: 0, step: 0, speed });
    sendReset(0);
    await flush(); page.frame(); await flush();
    for (let tick = 0; tick < 80; tick++) {
      const action = rnd(12);
      if (action === 0) {
        seek++; clock = rnd(500); playing = false;
        page.publish(clock, { playing, seek, step: 0, speed });
        await flush();
        sendReset(clock);
      } else if (action === 1) {
        playing = !playing;
      } else if (action === 2) {
        speed = [24, 168, 720, 1440][rnd(4)]!;
      } else if (action === 3 && active.size < 4) {
        joinWidget(seek);
        const w = [...active.values()].at(-1)!;
        const v = BigInt(clock * 10 + w.sub);
        w.lastVal = v;
        const pSplit = splitForLanes(panelPacket([{ id: w.sub, hour: clock, kind: 'data', stepped: true, data: panelData(w.cell, clock) }, { id: 0, hour: clock, kind: 'completed' }], seek), LANES);
        if (pSplit.kind === 'panels') pSplit.lanes.forEach((b, l) => { if (b) lanes[l]!.receive('panels', b); });
        const fSplit = splitForLanes(message(clock, seek, false, [{ id: w.cell, hour: clock, body: answerBody({ keys: [1n], columns: 1, values: [v] }) }]), LANES);
        if (fSplit.kind === 'follow') fSplit.lanes.forEach((b, l) => { if (b) lanes[l]!.receive('follow', b); });
      } else if (action === 4 && active.size > 1) {
        leaveWidget([...active.keys()][0]!);
      }
      if (playing) {
        step = Math.max(1, Math.round(speed / 60));
        clock += step;
        const answers = [...active.values()].filter(w => w.lastVal !== null).map(w => {
          const v = BigInt(clock * 10 + w.sub);
          w.lastVal = v;
          return { id: w.cell, hour: clock, body: answerBody({ keys: [1n], columns: 1, values: [v], changed: true }) };
        });
        const fSplit = splitForLanes(message(clock, seek, false, answers), LANES);
        if (fSplit.kind === 'follow') fSplit.lanes.forEach((b, l) => { if (b) lanes[l]!.receive('follow', b); });
        const pSplit = splitForLanes(panelPacket([{ id: 0, hour: clock, kind: 'completed' }], seek), LANES);
        if (pSplit.kind === 'panels') pSplit.lanes.forEach((b, l) => { if (b) lanes[l]!.receive('panels', b); });
      }
      page.publish(clock, { playing, seek, step, speed });
      await flush();
      if (!playing) { page.frame(); await flush(); }
      for (const [id, w] of active) {
        const view = page.view(id);
        assert.notEqual(view.mode, 'failed', `seed ${seed} tick ${tick} widget ${id}: ${view.reason}`);
        if (view.mode === 'picture' && view.h !== undefined) {
          const live = view.fields?.[`live:${w.sub}`] as Map<string, number | bigint | string> | undefined;
          assert.equal(live?.get('r0\u0000k'), view.h * 10 + w.sub, `seed ${seed} tick ${tick} widget ${id} live value at ${view.h}`);
        }
      }
    }
    assert.deepEqual(laneFailures, []);
  }
});

test('differential: ordering hazards (batched resets, deferred crop, speculative early open entries, sticky faults, panel decode faults + STANDS + stale seeks)', async () => {
  const started = performance.now();
  const faults: Fault[] = [];
  let checked = 0;
  for (let seed = 1; seed <= 40; seed++) {
    const rnd = rng(0x7c3d + seed * 4999);
    const widgets = [{ id: 1, subs: [3] }, { id: 2, subs: [5] }];
    const subs = [3, 5];
    const truth = recording(rnd, subs);
    const pendingCrops: (() => void)[] = [];
    const deferredPresentations: Parameters<Page<Fake>['offer']>[0][] = [];
    const lanes: Lane<Fake>[] = [];
    const laneFailures: string[] = [];
    const subFailedCounts = new Map<number, number>();
    const page = new Page<Fake>({ lanes: 1, post: (_lane, m: PageMessage) => {
      if (m.type === 'frame') lanes[0]!.frame(m);
      else if (m.type === 'stream') lanes[0]!.stream(m.route, m.seek, m.target);
      else lanes[0]!.fail(m.route);
    } });
    const fields = fieldsView(() => undefined);
    lanes.push(new Lane<Fake>({
      lane: 0, lanes: 1,
      views: () => ({
        paint: (input, state, content) => {
          const painted = fields.paint(input, state, content);
          const id = `s:${input.pictures[0]!.parts[0]!.version}`;
          return { ...painted, surfaces: [{ source: { width: 1, height: 1 }, content: id }] };
        },
      }),
      crop: sources => new Promise<Fake[]>(resolve => {
        const done = () => resolve(sources.map((): Fake => ({ closed: false, width: 1, close() { this.closed = true; } })));
        if (rnd(3) === 0) pendingCrops.push(done);
        else done();
      }),
      present: presentation => {
        if (deferredPresentations.length || rnd(4) === 0) deferredPresentations.push(presentation);
        else page.offer(presentation);
      },
      release: () => {}, later: () => {},
      fail: reason => laneFailures.push(`seed ${seed}: ${reason}`),
      failSub: (sub, reason) => {
        subFailedCounts.set(sub, (subFailedCounts.get(sub) ?? 0) + 1);
        for (const w of widgets) if (w.subs.includes(sub)) page.name(w.id, 'failed', reason);
      },
      failWidget: (widget, reason) => page.name(widget, 'failed', reason),
    }));
    for (const w of widgets) {
      for (const sub of w.subs) { lanes[0]!.follow(sub, 'followed'); lanes[0]!.bindRoute(sub, ROUTE, NATIVE); }
      lanes[0]!.mount(w.id, w.subs, 'fields');
      lanes[0]!.bind(w.id, { generation: 1, surfaces: [{ width: 1, height: 1, dpr: 1 }], offset: 0, pin: null });
      page.mount(w.id, { route: ROUTE, lane: 0, surfaces: [{ install() {}, clear() {} }] });
    }
    let seek = 0, clock = 0, playing = true, speed = 720, step = 12, nativeHead = 0;
    const sent = new Map<number, string | bigint[]>();
    const faulted = new Set<number>();
    const makeAnswers = (hour: number, whole: boolean) => subs.flatMap(sub => {
      const reason = truth.reason(sub, hour), rows = truth.rows(sub, hour), last = sent.get(sub);
      if (reason !== null) {
        if (!whole && last === reason) return [];
        sent.set(sub, reason);
        return [{ id: sub, hour, body: answerBody({ notRecorded: reason }) }];
      }
      if (!whole && Array.isArray(last) && !faulted.has(sub) && rnd(10)) {
        const changed = KEYS.flatMap((_k, r) => last[r] !== rows![r] ? [r] : []);
        sent.set(sub, rows!);
        if (!changed.length) return [];
        return [{ id: sub, hour, body: answerBody({ keys: changed.map(r => KEYS[r]!), columns: 1, values: changed.map(r => rows![r]!), changed: true }) }];
      }
      sent.set(sub, rows!);
      return [{ id: sub, hour, body: answerBody({ keys: KEYS, columns: 1, values: rows! }) }];
    });
    page.publish(0, { playing, seek, step, speed });
    lanes[0]!.receive('follow', message(0, 0, true, makeAnswers(0, true)));
    await flush();
    for (let tick = 0; tick < 80; tick++) {
      const action = rnd(14);
      if (action === 0) {
        seek++; clock = rnd(HOURS - 200); playing = false; nativeHead = clock; sent.clear();
        lanes[0]!.receive('follow', message(clock, seek, true, makeAnswers(clock, true)));
      } else if (action === 1) {
        playing = !playing;
      } else if (action === 2) {
        nativeHead = Math.min(HOURS - 2, clock);
        sent.clear();
        lanes[0]!.receive('follow', message(nativeHead, seek, true, makeAnswers(nativeHead, true)));
      } else if (action === 3 && !faulted.has(3)) {
        faulted.add(3);
        lanes[0]!.receive('follow', message(nativeHead, seek, false, [{ id: 3, hour: nativeHead, body: answerBody({ keys: [999n], columns: 1, values: [1n], changed: true }) }]));
      }
      if (playing) {
        step = 12 + rnd(13);
        clock = Math.min(HOURS - 50, clock + step);
      }
      const targetHead = Math.min(HOURS - 1, clock + (playing ? 36 : 0));
      while (nativeHead < targetHead) {
        nativeHead++;
        lanes[0]!.receive('follow', message(nativeHead, seek, false, makeAnswers(nativeHead, false)));
      }
      while (pendingCrops.length && rnd(2) === 0) pendingCrops.shift()!();
      while (deferredPresentations.length && (rnd(2) === 0 || !playing)) page.offer(deferredPresentations.shift()!);
      page.publish(clock, { playing, seek, step, speed });
      await flush();
      if (!playing) {
        for (let d = 0; d < 4; d++) {
          while (pendingCrops.length) pendingCrops.shift()!();
          await flush();
          while (deferredPresentations.length) page.offer(deferredPresentations.shift()!);
          page.frame();
        }
      }
      for (const w of widgets) {
        const view = page.view(w.id), sub = w.subs[0]!;
        if (faulted.has(sub)) {
          if (view.mode !== 'failed') faults.push({ property: 'sticky-fault', detail: `seed ${seed} tick ${tick} w${w.id} mode ${view.mode} after sub ${sub} faulted` });
          if ((subFailedCounts.get(sub) ?? 0) !== 1) faults.push({ property: 'fault-once', detail: `seed ${seed} tick ${tick} sub ${sub} failSub called ${subFailedCounts.get(sub) ?? 0} times` });
          continue;
        }
        if (view.mode === 'failed') faults.push({ property: 'sibling-isolated', detail: `seed ${seed} tick ${tick} healthy sibling w${w.id} failed: ${view.reason}` });
        if (view.mode === 'picture') {
          checked++;
          const h = view.h!, c = page.widgets.get(w.id)!.installed!.c;
          const certified = Math.min(c, nativeHead, HOURS - 1);
          if (certified > h && !truth.same(sub, h, certified)) {
            faults.push({ property: 'certified-change', detail: `seed ${seed} tick ${tick} w${w.id} certified ${h}-${c} across a recording change` });
          }
          if (h > view.target) faults.push({ property: 'ahead', detail: `seed ${seed} tick ${tick} w${w.id} h ${h} > target ${view.target}` });
          if (!playing && view.target <= c && view.dim) faults.push({ property: 'spurious-dim', detail: `seed ${seed} tick ${tick} w${w.id} dimmed at ${view.target} with c=${c}` });
        }
      }
    }
    assert.deepEqual(laneFailures, []);
  }
  for (let seed = 1; seed <= 20; seed++) {
    const rnd = rng(0x2b19 + seed * 2713);
    const laneFailures: string[] = [];
    const page = new Page<Fake>({ lanes: 1, post: (_lane, m: PageMessage) => {
      if (m.type === 'frame') lane.frame(m); else if (m.type === 'stream') lane.stream(m.route, m.seek, m.target);
    } });
    const lane = new Lane<Fake>({
      lane: 0, lanes: 1, views: () => fieldsView(() => undefined),
      crop: async () => [], present: p => page.offer(p), release: () => {}, later: () => {},
      fail: reason => laneFailures.push(`panel seed ${seed}: ${reason}`),
      failSub: (sub, reason) => page.name(sub === 3 ? 1 : 2, 'failed', reason),
      failWidget: (widget, reason) => page.name(widget, 'failed', reason),
    });
    lane.follow(3, 'panel', 'demo'); lane.bindRoute(3, ROUTE, NATIVE, 0);
    lane.mount(1, [3], 'fields'); lane.bind(1, { generation: 1, surfaces: [], offset: 0, pin: null });
    page.mount(1, { route: ROUTE, lane: 0, surfaces: [] });
    lane.follow(5, 'panel', 'demo'); lane.bindRoute(5, ROUTE, NATIVE, 0);
    lane.mount(2, [5], 'fields'); lane.bind(2, { generation: 1, surfaces: [], offset: 0, pin: null });
    page.mount(2, { route: ROUTE, lane: 0, surfaces: [] });
    let seek = 0, hour = 0, expectFailed = false;
    const corruptPanel = (id: number, atHour: number, atSeek: number) => {
      const p = panelPacket([{ id, hour: atHour, kind: 'data', stepped: true, data: { shape: `H${atHour}` } }, { id: 0, hour: atHour, kind: 'completed' }], atSeek);
      p[24 + 16] = 0xc1;
      return p;
    };
    const standsPanel = (atHour: number, atSeek: number) => {
      const rec = new Uint8Array(16), rv = new DataView(rec.buffer);
      rv.setUint32(0, 3, true); rv.setInt32(4, atHour, true); rv.setUint8(8, 3); rv.setUint8(9, 1); rv.setUint32(12, 0, true);
      const out = new Uint8Array(24 + 16), dv = new DataView(out.buffer);
      dv.setUint32(0, 0x324c4e50, true); dv.setUint16(4, DATED_PANEL_VERSION, true); dv.setUint32(8, 1, true); dv.setUint32(16, NATIVE, true); dv.setUint32(20, atSeek, true);
      out.set(rec, 24);
      return out;
    };
    page.publish(0, { playing: false, seek: 0 });
    lane.receive('panels', panelPacket([
      { id: 3, hour: 0, kind: 'data', stepped: true, data: { shape: 'H0' } },
      { id: 5, hour: 0, kind: 'data', stepped: true, data: { shape: 'S0' } },
      { id: 0, hour: 0, kind: 'completed' },
    ], 0));
    await flush(); page.frame(); await flush();
    for (let step = 1; step <= 40; step++) {
      hour += 5;
      const op = rnd(5);
      if (op === 0) {
        seek++;
        page.publish(hour, { playing: false, seek }); await flush();
        lane.receive('panels', panelPacket([
          { id: 3, hour, kind: 'data', stepped: true, data: { shape: `H${hour}` } },
          { id: 5, hour, kind: 'data', stepped: true, data: { shape: `S${hour}` } },
          { id: 0, hour, kind: 'completed' },
        ], seek));
      } else if (op === 1) {
        expectFailed = true;
        lane.receive('panels', corruptPanel(3, hour, seek));
      } else if (op === 2) {
        lane.receive('panels', standsPanel(hour, seek));
      } else if (op === 3 && seek > 0) {
        lane.receive('panels', corruptPanel(5, hour, seek - 1));
      } else {
        lane.receive('panels', panelPacket([
          { id: 3, hour, kind: 'data', stepped: true, data: { shape: `H${hour}` } },
          { id: 5, hour, kind: 'data', stepped: true, data: { shape: `S${hour}` } },
          { id: 0, hour, kind: 'completed' },
        ], seek));
      }
      page.publish(hour, { playing: false, seek }); await flush(); await flush(); page.frame(); await flush(); page.frame();
      const view = page.view(1), sibling = page.view(2);
      if (expectFailed && view.mode !== 'failed') faults.push({ property: 'panel-fault', detail: `panel seed ${seed} step ${step} mode ${view.mode}, expected sticky failed` });
      if (!expectFailed && view.mode === 'failed') faults.push({ property: 'panel-stale', detail: `panel seed ${seed} step ${step} unexpectedly failed: ${view.reason}` });
      if (sibling.mode === 'failed') faults.push({ property: 'panel-sibling', detail: `panel seed ${seed} step ${step} healthy sibling failed: ${sibling.reason}` });
    }
    assert.deepEqual(laneFailures, []);
  }
  assert.deepEqual(faults.slice(0, 5), [], `ordering differential faults (${faults.length} total, checked ${checked} frames in ${Math.round(performance.now() - started)}ms)`);
  assert.ok(checked > 500, `checked ${checked} frames`);
});

test('differential: N3 rewinding into a carried panel while native switches and reuses a newer cell group never drops its reset table', async () => {
  const CELL_BASE = 0x8000_0000;
  for (let seed = 1; seed <= 20; seed++) {
    const rnd = rng(0x4a71 + seed * 4219);
    const laneFailures: string[] = [];
    const page = new Page<Fake>({ lanes: 1, post: (_lane, m: PageMessage) => {
      if (m.type === 'frame') lane.frame(m); else if (m.type === 'stream') lane.stream(m.route, m.seek, m.target);
    } });
    const lane = new Lane<Fake>({
      lane: 0, lanes: 1, views: () => fieldsView(() => undefined),
      crop: async () => [], present: p => page.offer(p), release: () => {}, later: () => {},
      fail: reason => laneFailures.push(`seed ${seed}: ${reason}`),
    });
    lane.follow(3, 'panel', 'demo'); lane.bindRoute(3, ROUTE, NATIVE, 0);
    lane.mount(1, [3], 'fields'); lane.bind(1, { generation: 1, surfaces: [], offset: 0, pin: null });
    page.mount(1, { route: ROUTE, lane: 0, surfaces: [] });
    const panelWith = (cell: number, tag: string) => ({
      shape: tag,
      live: { groups: [{ table: 1, columns: [0], keys: ['1'], follow: cell }], rows: ['r0'], kinds: [{ id: 'k', op: 'value', scale: 1 }], cells: [0, 0, 1, 0, 0, 0] },
    });
    let seek = 0, hour = 10, nextCell = 1, activeCell = (CELL_BASE | nextCell++) >>> 0;
    let carriedHour = 10;
    const emitResetAndPanel = (atHour: number, cell: number, resetFirst: boolean) => {
      const fBytes = message(atHour, seek, true, [{ id: cell, hour: atHour, body: answerBody({ keys: [1n], columns: 1, values: [BigInt(atHour * 10)] }) }]);
      const pBytes = panelPacket([{ id: 3, hour: atHour, kind: 'data', stepped: true, data: panelWith(cell, `S${seek}:${atHour}:${cell}`) }, { id: 0, hour: atHour + 10, kind: 'completed' }], seek);
      if (resetFirst) { lane.receive('follow', fBytes); lane.receive('panels', pBytes); }
      else { lane.receive('panels', pBytes); lane.receive('follow', fBytes); }
    };
    page.publish(hour, { playing: false, seek: 0 });
    emitResetAndPanel(hour, activeCell, false);
    await flush(); page.frame(); await flush();
    for (let step = 1; step <= 30; step++) {
      const op = rnd(4);
      if (op === 0) {
        carriedHour = hour + 5;
        hour += 20;
        activeCell = (CELL_BASE | nextCell++) >>> 0;
        lane.receive('panels', panelPacket([{ id: 3, hour, kind: 'data', stepped: true, data: panelWith(activeCell, `S${seek}:${hour}:${activeCell}`) }, { id: 0, hour: hour + 10, kind: 'completed' }], seek));
        lane.receive('follow', message(hour + 10, seek, false, [{ id: activeCell, hour, body: answerBody({ keys: [1n], columns: 1, values: [BigInt(hour * 10)] }) }]));
        seek++;
        page.publish(carriedHour, { playing: false, seek });
        await flush(); await flush(); page.frame(); await flush();
        lane.receive('panels', panelPacket([{ id: 0, hour: carriedHour, kind: 'completed' }], seek));
        emitResetAndPanel(carriedHour, activeCell, true);
        hour = carriedHour + 1;
        lane.receive('follow', message(hour, seek, false, [{ id: activeCell, hour, body: answerBody({ keys: [1n], columns: 1, values: [BigInt(hour * 10)], changed: true }) }]));
        lane.receive('panels', panelPacket([{ id: 0, hour, kind: 'completed' }], seek));
      } else {
        hour += 2;
        lane.receive('follow', message(hour, seek, false, [{ id: activeCell, hour, body: answerBody({ keys: [1n], columns: 1, values: [BigInt(hour * 10)], changed: true }) }]));
        lane.receive('panels', panelPacket([{ id: 0, hour, kind: 'completed' }], seek));
      }
      page.publish(hour, { playing: false, seek });
      await flush(); await flush(); page.frame(); await flush(); page.frame();
      const view = page.view(1);
      assert.notEqual(view.mode, 'failed', `seed ${seed} step ${step} failed: ${view.reason}`);
    }
    assert.deepEqual(laneFailures, []);
  }
});

test('differential: M2 future-seek panel replay preserves exact panel coverage across interleaved data, completed, stands and waiting packets', async () => {
  const rawPanel = (records: { id: number; hour: number; kind: 'data' | 'stands' | 'completed' | 'waiting'; data?: unknown; stepped?: boolean }[], seek: number): Uint8Array => {
    const parts = records.map(record => {
      const body = record.kind === 'data' ? encode(record.data) : new Uint8Array();
      const out = new Uint8Array((16 + body.length + 7) & ~7), view = new DataView(out.buffer);
      view.setUint32(0, record.kind === 'completed' ? 0 : record.id, true);
      view.setInt32(4, record.hour, true);
      view.setUint8(8, { data: 1, stands: 3, completed: 4, waiting: 5 }[record.kind]);
      view.setUint8(9, record.stepped ? 1 : 0);
      view.setUint32(12, body.length, true);
      out.set(body, 16);
      return out;
    });
    const out = new Uint8Array(24 + parts.reduce((sum, part) => sum + part.length, 0)), view = new DataView(out.buffer);
    view.setUint32(0, 0x324c4e50, true); view.setUint16(4, DATED_PANEL_VERSION, true); view.setUint32(8, records.length, true);
    view.setUint32(16, NATIVE, true); view.setUint32(20, seek, true);
    let at = 24;
    for (const part of parts) { out.set(part, at); at += part.length; }
    return out;
  };
  for (let seed = 1; seed <= 30; seed++) {
    const rnd = rng(0x55a1 + seed * 6133);
    const makePair = () => {
      const page = new Page<Fake>({ lanes: 1, post: (_lane, m: PageMessage) => {
        if (m.type === 'frame') lane.frame(m); else if (m.type === 'stream') lane.stream(m.route, m.seek, m.target);
      } });
      const lane = new Lane<Fake>({
        lane: 0, lanes: 1, views: () => fieldsView(() => (panel => (panel.data as { shape?: string } | undefined)?.shape)),
        crop: async () => [], present: p => page.offer(p), release: () => {}, later: () => {},
        fail: reason => { throw new Error(reason); },
      });
      lane.follow(3, 'panel', 'demo'); lane.bindRoute(3, ROUTE, NATIVE, 0);
      lane.mount(1, [3], 'fields'); lane.bind(1, { generation: 1, surfaces: [], offset: 0, pin: null });
      page.mount(1, { route: ROUTE, lane: 0, surfaces: [] });
      return { page, lane };
    };
    const buffered = makePair(), direct = makePair();
    buffered.page.publish(0, { playing: false, seek: 0 });
    direct.page.publish(0, { playing: false, seek: 1 });
    await flush();
    const packets: Uint8Array[] = [];
    let hour = 10;
    packets.push(rawPanel([{ id: 3, hour, kind: 'data', stepped: true, data: { shape: `H${hour}` } }, { id: 0, hour, kind: 'completed' }], 1));
    for (let step = 0; step < 25; step++) {
      const op = rnd(6);
      if (op === 5 && hour >= 14) {
        const prev = hour - 2;
        packets.push(rawPanel([{ id: 3, hour: prev, kind: 'data', stepped: true, data: { shape: `R${step}` } }], 1));
        packets.push(rawPanel([{ id: 0, hour, kind: 'completed' }], 1));
        continue;
      }
      hour += 1 + rnd(4);
      if (op <= 1) packets.push(rawPanel([{ id: 0, hour, kind: 'completed' }], 1));
      else if (op === 2) packets.push(rawPanel([{ id: 3, hour, kind: 'data', stepped: true, data: { shape: `H${hour}` } }, { id: 0, hour, kind: 'completed' }], 1));
      else if (op === 3) packets.push(rawPanel([{ id: 3, hour, kind: 'stands', stepped: true }], 1));
      else packets.push(rawPanel([{ id: 3, hour, kind: 'waiting' }], 1));
    }
    for (const pkt of packets) {
      buffered.lane.receive('panels', pkt);
      direct.lane.receive('panels', pkt);
    }
    buffered.page.publish(10, { playing: false, seek: 1 });
    await flush();
    for (let check = 10; check <= hour; check++) {
      const a = buffered.lane.facts.select(3, check);
      const b = direct.lane.facts.select(3, check);
      assert.deepEqual(a ? [a.h, a.c, (a.value as { data?: unknown }).data] : null, b ? [b.h, b.c, (b.value as { data?: unknown }).data] : null,
        `seed ${seed} hour ${check}: future-seek replay diverged from direct delivery`);
    }
  }
});

test('differential: Round 5 ordering hazards (same-packet valid+corrupt, reused cell answer-before-panel, unbound overflow, unbound superseded seek, stale refusal after switch, late registration after sub-failed, shared widget tokens, and credit invariant)', async () => {
  const { managedInProcess, NATIVE: IN_PROC_NATIVE } = await import('./inProcess.fixture.ts');
  const { message: followMsg } = await import('./messages.fixture.ts');
  const CELL_BASE = 0x8000_0400;
  const followCharge = (bytes: Uint8Array) => {
    const data = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    let total = 0;
    for (let i = 0, at = 24, count = data.getUint32(12, true); i < count; i++) {
      const length = data.getUint32(at + 8, true);
      total += 16 + 2 * Math.max(length, data.getUint32(at + 12, true));
      at += 16 + length;
    }
    return total;
  };
  const panelCharge = (bytes: Uint8Array) => {
    const data = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    let total = 0;
    for (let i = 0, at = 24, count = data.getUint32(8, true); i < count; i++) {
      const kind = data.getUint8(at + 8), length = data.getUint32(at + 12, true), size = ((16 + length + 7) & ~7);
      if (kind !== 4) total += 2 * size;
      at += size;
    }
    return total;
  };
  const panelWith = (cell: number, tag: string) => ({
    shape: tag,
    live: { groups: [{ table: 1, columns: [0], keys: ['1'], follow: cell }], rows: ['r0'], kinds: [{ id: 'k', op: 'value', scale: 1 }], cells: [0, 0, 1, 0, 0, 0] },
  });

  for (let seed = 1; seed <= 25; seed++) {
    const rnd = rng(0x6d31 + seed * 5381);
    const released: number[] = [];
    const subFailures: [number, string][] = [];
    const widgetSubs = new Map<number, number[]>();
    let holdCrop = false;
    const pendingCrops: (() => void)[] = [];
    const page = new Page<Fake>({ lanes: 1, post: (_lane, m: PageMessage) => {
      if (m.type === 'frame') lane.frame(m); else if (m.type === 'stream') lane.stream(m.route, m.seek, m.target);
    } });
    const fields = fieldsView(() => (panel => (panel.data as { shape?: string } | undefined)?.shape));
    const lane = new Lane<Fake>({
      lane: 0, lanes: 1,
      views: () => ({
        paint: (input, state, content) => {
          const p = fields.paint(input, state, content);
          return { ...p, surfaces: [{ source: { width: 1, height: 1 }, content: `s:${input.subs.map(s => s.id).join(',')}:${input.pictures[0]!.h}` }] };
        },
      }),
      crop: sources => new Promise<Fake[]>(resolve => {
        const done = () => resolve(sources.map((): Fake => ({ closed: false, width: 1, close() { this.closed = true; } })));
        if (holdCrop) pendingCrops.push(done); else done();
      }),
      present: p => page.offer(p),
      release: bytes => released.push(bytes),
      later: () => {},
      earlyBuffer: 256,
      fail: reason => { throw new Error(`seed ${seed}: lane failed: ${reason}`); },
      failSub: (sub, reason) => {
        subFailures.push([sub, reason]);
        for (const [w, subs] of widgetSubs) if (subs.includes(sub)) page.name(w, 'failed', reason);
      },
      failWidget: (w, reason) => page.name(w, 'failed', reason),
    });
    let totalCharged = 0;
    const totalReleased = () => released.reduce((a, b) => a + b, 0);
    const checkCredit = (label: string) => assert.equal(totalReleased() + lane.retainedBytes(), totalCharged, `seed ${seed} ${label}`);
    const sendF = (pkt: Uint8Array, label: string) => { totalCharged += followCharge(pkt); lane.receive('follow', pkt); checkCredit(label); };
    const sendP = (pkt: Uint8Array, label: string) => { totalCharged += panelCharge(pkt); lane.receive('panels', pkt); checkCredit(label); };
    const mountW = (w: number, sub: number, channel: 'followed' | 'panel', bindSeek?: number) => {
      lane.follow(sub, channel, channel === 'panel' ? 'demo' : undefined);
      widgetSubs.set(w, [sub]);
      lane.mount(w, [sub], 'fields');
      page.mount(w, { route: ROUTE, lane: 0, surfaces: [{ install() {}, clear() {} }] });
      lane.bind(w, { generation: 1, surfaces: [{ width: 1, height: 1, dpr: 1 }], offset: 0, pin: null });
      if (bindSeek !== undefined) { lane.bindRoute(sub, ROUTE, NATIVE, bindSeek); checkCredit(`bind ${sub}`); }
    };
    const pump = async (hour: number, seek: number) => {
      page.publish(hour, { playing: false, seek });
      await flush();
      while (pendingCrops.length) pendingCrops.shift()!();
      await flush(); page.frame(); await flush(); page.frame();
      checkCredit(`pump ${hour}:${seek}`);
    };

    const scenario = seed % 5;
    if (scenario === 0) {
      mountW(1, 3, 'followed', 0);
      mountW(2, 5, 'followed', 0);
      sendF(message(10, 0, false, [
        { id: 3, hour: 10, body: answerBody({ keys: [1n], columns: 1, values: [10n] }) },
        { id: 3, hour: 10, body: answerBody({ keys: [9n], columns: 1, values: [99n], changed: true }) },
        { id: 5, hour: 10, body: answerBody({ keys: [1n], columns: 1, values: [50n] }) },
      ]), 'same-packet follow valid+corrupt');
      await pump(10, 0);
      assert.equal(page.view(1).mode, 'failed');
      assert.equal(page.view(2).mode, 'picture');
      await pump(10, 1);
      assert.equal(page.view(1).mode, 'failed', 'failed sub never carries pre-fault same-packet table across seek');
    } else if (scenario === 1) {
      const cell = (CELL_BASE | (seed * 2 + 1)) >>> 0;
      mountW(1, 3, 'panel', 0);
      const badBatch = panelPacket([
        { id: 3, hour: 10, kind: 'data', stepped: true, data: panelWith(cell, 'bad-owner') },
        { id: 3, hour: 10, kind: 'data', stepped: true, data: { live: { groups: 'bad', rows: [], kinds: [], cells: [] } } },
        { id: 0, hour: 10, kind: 'completed' },
      ], 0);
      sendP(badBatch, 'same-packet panel valid+corrupt');
      assert.equal(page.view(1).mode, 'failed');
      if (rnd(2) === 0) sendF(message(15, 0, false, [{ id: cell, hour: 15, body: answerBody({ keys: [1n], columns: 1, values: [15n], changed: true }) }]), 'stale delta');
      sendF(message(20, 0, false, [{ id: cell, hour: 20, body: answerBody({ keys: [1n], columns: 1, values: [220n] }) }]), 'fresh whole before healthy panel');
      mountW(2, 5, 'panel', 0);
      sendP(panelPacket([{ id: 5, hour: 20, kind: 'data', stepped: true, data: panelWith(cell, 'healthy-owner') }, { id: 0, hour: 20, kind: 'completed' }], 0), 'healthy panel reusing cell');
      await pump(20, 0);
      assert.equal(page.view(1).mode, 'failed');
      assert.equal(page.view(2).mode, 'picture');
      assert.equal((page.view(2).fields?.['live:5'] as Map<string, unknown> | undefined)?.get('r0\u0000k'), 220);
      sendF(message(21, 0, false, [{ id: cell, hour: 21, body: answerBody({ keys: [1n], columns: 1, values: [221n], changed: true }) }]), 'delta after reuse');
      sendP(panelPacket([{ id: 0, hour: 21, kind: 'completed' }], 0), 'completed 21');
      await pump(21, 0);
      assert.equal((page.view(2).fields?.['live:5'] as Map<string, unknown> | undefined)?.get('r0\u0000k'), 221);
    } else if (scenario === 2) {
      mountW(1, 3, 'panel', 0);
      mountW(2, 5, 'panel');
      await pump(10, 0);
      for (let i = 0; i < 4; i++) {
        sendP(panelPacket([
          { id: 3, hour: 100 + i, kind: 'data', stepped: true, data: { shape: 'A', pad: 'x'.repeat(100) } },
          { id: 5, hour: 100 + i, kind: 'data', stepped: true, data: { shape: 'B', pad: 'x'.repeat(100) } },
        ], 1), `overflow ${i}`);
      }
      await pump(100, 1);
      assert.equal(page.view(1).mode, 'failed');
      lane.bindRoute(5, ROUTE, NATIVE, 1); checkCredit('late bind after overflow');
      await pump(100, 1);
      assert.equal(page.view(2).mode, 'failed', 'unbound sub told of overflow when it binds');
    } else if (scenario === 3) {
      mountW(1, 3, 'panel');
      const bad1 = panelPacket([{ id: 3, hour: 10, kind: 'data', stepped: true, data: { shape: 'bad1' } }], 1);
      bad1[24 + 16] = 0xc1;
      const good2 = panelPacket([{ id: 3, hour: 20, kind: 'data', stepped: true, data: { shape: 'good2' } }, { id: 0, hour: 20, kind: 'completed' }], 2);
      if (rnd(2) === 0) { sendP(bad1, 'unbound bad seek 1'); sendP(good2, 'unbound good seek 2'); }
      else { sendP(good2, 'unbound good seek 2 first'); sendP(bad1, 'unbound bad seek 1 late'); }
      lane.bindRoute(3, ROUTE, NATIVE, 1); checkCredit('bind at seek 1');
      assert.equal(subFailures.length, 0);
      await pump(20, 2);
      assert.equal(page.view(1).mode, 'picture');
      assert.equal((page.view(1).fields?.['model:3'] as { structure?: string }).structure, 'good2');
    } else {
      mountW(1, 3, 'followed', 0);
      holdCrop = true;
      sendF(message(10, 0, false, [{ id: 3, hour: 10, body: answerBody({ keys: [1n], columns: 1, values: [10n] }) }]), 'initial');
      page.publish(10, { playing: false, seek: 0 });
      await flush();
      page.name(1, 'failed', 'old sub failed');
      holdCrop = false;
      while (pendingCrops.length) pendingCrops.shift()!();
      await flush();
      lane.follow(5, 'followed'); lane.bindRoute(5, ROUTE, NATIVE, 0);
      widgetSubs.set(1, [5]);
      lane.mount(1, [5], 'fields');
      page.rebind(1, { subscription: true });
      lane.bind(1, { generation: page.widgets.get(1)!.binding, surfaces: [{ width: 1, height: 1, dpr: 1 }], offset: 0, pin: null });
      sendF(message(10, 0, false, [{ id: 5, hour: 10, body: answerBody({ keys: [1n], columns: 1, values: [55n] }) }]), 'healthy switch');
      await pump(10, 0);
      assert.equal(page.view(1).mode, 'picture', 'stale refusal from old binding never halts healthy switch');
    }
  }

  let releaseFollow!: () => void;
  let notifyFollow!: () => void;
  const followStarted = new Promise<void>(resolve => { notifyFollow = resolve; });
  const env = managedInProcess({
    gate: (product, params) => {
      if (product === 'follow' && (params.query as { params?: { gate?: boolean } })?.params?.gate) {
        notifyFollow();
        return new Promise<void>(resolve => { releaseFollow = resolve; });
      }
      return undefined;
    },
  });
  try {
    env.clock.seek(10);
    const p = env.pipeline;
    const wLate = p.acquire('diff-late|0', { offset: 0, pin: null });
    const sLate = p.subscribe(wLate, {}, env.source, '1', 0, { product: 'series-state', params: { gate: true } }, 'followed')!;
    await followStarted;
    const huge = { keys: Array.from({ length: 12000 }, (_, i) => BigInt(i + 1)), columns: 1, values: Array.from({ length: 12000 }, () => 1n) };
    const pkt = followMsg(10, [{ id: sLate, answer: huge }], false, IN_PROC_NATIVE, env.clock.getSnapshot().seekSequence);
    for (let i = 0; i < 20 && p.status(sLate).status !== 'failed'; i++) env.send(pkt);
    await env.waitView(p, wLate, v => v?.mode === 'failed');
    const unfollowed = env.nextEvent('unfollow');
    releaseFollow();
    await unfollowed;
    assert.equal(p.status(sLate).status, 'failed');
    assert.equal(p.view(wLate)?.mode, 'failed');

    const wShared = p.acquire('diff-shared|0', { offset: 0, pin: null });
    const tok1 = {}, tok2 = {};
    const c1 = env.nextEvent('follow-clock');
    const sShared = p.subscribe(wShared, tok1, env.source, '1', 0, { product: 'series-state', params: { shared: 1 } }, 'followed')!;
    p.subscribe(wShared, tok2, env.source, '1', 0, { product: 'series-state', params: { shared: 1 } }, 'followed');
    await c1;
    p.unsubscribe(wShared, tok2);
    await new Promise<void>(resolve => setImmediate(resolve));
    env.send(followMsg(10, [{ id: sShared, answer: { keys: [1n], columns: 1, values: [1n] } }], true, IN_PROC_NATIVE, env.clock.getSnapshot().seekSequence));
    await env.waitView(p, wShared, v => v?.mode === 'picture');
    env.send(followMsg(12, [{ id: sShared, answer: { keys: [99n], columns: 1, values: [99n], changed: true } }], false, IN_PROC_NATIVE, env.clock.getSnapshot().seekSequence));
    await env.waitView(p, wShared, v => v?.mode === 'failed');
    assert.equal(p.status(sShared).status, 'failed');
    assert.ok(p.status(sShared).error ?? p.widgetError(wShared));
  } finally {
    env.close();
  }
});

test('dated admission (#117): branch, seek, requested hour and coverage agree across panel, canvas and worker against an eager reference through advance, rewind, pin, gap, late replies and remount', async () => {
  type AdmissionModel = { readonly owner: string; readonly hour: number; readonly bucket: number; readonly value: number | null; readonly refused: string | null };
  type AdmissionLayout = { readonly line: string };
  type AdmissionBinding = LaneBinding<Record<string, never>> & { readonly family: 'admission-117' };

  const truth = (owner: 'b1' | 'b2', hour: number): { readonly refused: string | null; readonly value: number | null } => {
    const bucket = Math.floor(hour / 10);
    if (owner === 'b1') {
      if (hour >= 50 && hour <= 69) return { refused: 'branch 1 gap 50-69', value: null };
      return { refused: null, value: (hour < 50 ? 100 : 200) + bucket };
    }
    return { refused: null, value: 500 + bucket };
  };

  registerFamilyProjection<FamilyProjection<AdmissionModel, Record<string, never>>>({
    kind: 'admission-117',
    family: 'admission-117',
    project(view: BoundedReadView): AdmissionModel {
      const panel = view.panel<{ owner: string; value: number } | null>('main');
      const refused = view.refused('main') ?? null;
      const bucket = Math.floor(view.hour / 10);
      return {
        owner: panel?.owner ?? '',
        hour: view.hour,
        bucket,
        value: panel ? panel.value : null,
        refused,
      };
    },
    nextChange(view: BoundedReadView): number {
      return Math.floor(view.hour / 10) * 10 + 10;
    },
  });

  const admissionFamily: LaneFamily<AdmissionBinding, AdmissionModel, AdmissionLayout> = {
    family: 'admission-117' as never,
    boundary: () => Infinity,
    redraw: () => Infinity,
    status: model => (model.refused !== null ? { refused: model.refused } : 'picture'),
    view: [],
    layout: model => ({ line: `${model.owner}|b${model.bucket}|v${model.value}` }),
    paint(ctx, frame) {
      ctx.clearRect(0, 0, frame.surface.width, frame.surface.height);
      ctx.fillStyle = frame.theme.text;
      ctx.fillText(frame.layout.line, 4, 12);
      return false;
    },
    hit: () => 'row0',
    tooltip: (frame, key) => ({ rowKey: key, content: { title: `${frame.layout.line}@${frame.hour}`, rows: [] } as never }),
    structure: frame => ({ nav: [frame.layout.line] }),
    mirror: frame => [frame.layout.line],
    audit: frame => [frame.layout.line],
    visibleArt: () => [],
  };

  const marksOf = (canvas: RecordedCanvas): string[] => {
    const out: string[] = [];
    for (const [k, v] of canvas.marks) out.push(`${k}=${v}`);
    return out.sort();
  };

  const recording = installRecordingCanvas();
  try {
    const eagerRef = (owner: 'b1' | 'b2', hour: number) => {
      const sample = truth(owner, hour);
      const driver = driveLane(createLaneView(admissionFamily, () => {}), recording.canvases, { width: 200, height: 60, dpr: 1 });
      const value = { kind: 'panel' as const, bytes: null, data: sample.refused ? null : { owner, value: sample.value! }, refused: sample.refused };
      const subId = owner === 'b1' ? 3 : 5;
      const fact = { h: hour, c: hour, value, refused: sample.refused, cells: [], seeds: new Map(), channel: 'panel' as const, version: hour, bytes: 1 };
      const painted = driver.paint({
        label: `eager-${owner}-${hour}`,
        owner,
        hour,
        speed: 0,
        subs: [{ id: subId, channel: 'panel', product: 'admission', route: null }],
        pictures: [{ h: hour, c: hour, parts: [fact], value, refused: sample.refused }],
        binding: { family: 'admission-117', kind: 'admission-117', options: {}, sources: { main: subId }, scrollTop: 0, theme: LANE_THEME },
        inspect: 'row0',
      });
      return {
        mode: painted.mode ?? 'picture',
        reason: painted.reason,
        picture: driver.picture(),
        mirror: painted.fields.mirror?.value,
        nav: painted.fields.nav?.value,
        tooltip: (painted.fields.tooltip?.value as { text?: unknown } | undefined)?.text ?? null,
      };
    };

    interface SnapBitmap extends Bitmap { readonly snapshot: readonly string[]; closed: boolean }
    const makeSurface = () => {
      const state = { snapshot: null as readonly string[] | null, clears: 0, installs: 0 };
      return {
        state,
        surface: {
          install(bitmap: SnapBitmap) { state.installs++; state.snapshot = bitmap.snapshot; bitmap.close(); },
          clear() { state.clears++; state.snapshot = null; },
        },
      };
    };

    const heldPresentations: Parameters<Page<SnapBitmap>['offer']>[0][] = [];
    let holdNext = false;
    let lane!: Lane<SnapBitmap>;
    const page = new Page<SnapBitmap>({
      lanes: 1,
      post: (_lane, m: PageMessage) => {
        if (m.type === 'frame') lane.frame(m);
        else if (m.type === 'stream') lane.stream(m.route, m.seek, m.target);
        else lane.fail(m.route);
      },
    });
    const view = createLaneView(admissionFamily, () => {});
    lane = new Lane<SnapBitmap>({
      lane: 0,
      lanes: 1,
      clock: () => 100,
      views: () => view,
      crop: async sources => sources.map((source): SnapBitmap => ({
        width: source.width,
        snapshot: marksOf(source as unknown as RecordedCanvas),
        closed: false,
        close() { this.closed = true; },
      })),
      present: presentation => {
        if (holdNext) { holdNext = false; heldPresentations.push(presentation); }
        else page.offer(presentation);
      },
      release: () => {},
      fail: reason => { throw new Error(reason); },
      later: () => {},
    });

    const packetFor = (native: number, sub: number, owner: 'b1' | 'b2', atHour: number, completedHour: number, seek: number): Uint8Array => {
      const sample = truth(owner, atHour);
      const rec = sample.refused !== null
        ? { id: sub, hour: atHour, kind: 'refused' as const, data: sample.refused }
        : { id: sub, hour: atHour, kind: 'data' as const, data: { owner, value: sample.value! } };
      const parts = [rec, { id: 0, hour: completedHour, kind: 'completed' as const }].map(record => {
        const body = record.kind === 'data' ? encode(record.data) : record.kind === 'refused' ? new TextEncoder().encode(record.data) : new Uint8Array();
        const out = new Uint8Array((16 + body.length + 7) & ~7), dv = new DataView(out.buffer);
        dv.setUint32(0, record.kind === 'completed' ? 0 : record.id, true);
        dv.setInt32(4, record.hour, true);
        dv.setUint8(8, record.kind === 'data' ? 1 : record.kind === 'refused' ? 0 : 4);
        dv.setUint8(9, 1);
        dv.setUint32(12, body.length, true);
        out.set(body, 16);
        return out;
      });
      const out = new Uint8Array(24 + parts.reduce((sum, p) => sum + p.length, 0)), dv = new DataView(out.buffer);
      dv.setUint32(0, 0x324c4e50, true); dv.setUint16(4, DATED_PANEL_VERSION, true); dv.setUint32(8, parts.length, true);
      dv.setUint32(16, native, true); dv.setUint32(20, seek, true);
      let at = 24;
      for (const part of parts) { out.set(part, at); at += part.length; }
      return out;
    };

    const s1 = makeSurface();
    let activeSurface = s1;
    let bindingGen = 1;
    let seek = 0;

    const modelFor = (sub: number): AdmissionBinding => ({
      family: 'admission-117', kind: 'admission-117', options: {}, sources: { main: sub }, scrollTop: 0, theme: LANE_THEME,
    });

    lane.follow(3, 'panel', 'admission');
    lane.bindRoute(3, 1, 11, 0);
    lane.mount(1, [3], 'admission-117', modelFor(3));
    lane.bind(1, { generation: bindingGen, owner: 'b1', surfaces: [{ width: 200, height: 60, dpr: 1 }], offset: 0, pin: null });
    page.mount(1, { route: 1, lane: 0, owner: 'b1', bounds: [10, 120], surfaces: [s1.surface] });
    page.inspect(1, 'row0');

    const settle = async (hour: number, opts: { playing: boolean; seek?: number; step?: number; speed?: number }) => {
      page.publish(hour, { playing: opts.playing, seek: opts.seek ?? seek, step: opts.step ?? 0, speed: opts.speed ?? 0 });
      await flush();
      page.frame();
      await flush();
      page.frame();
    };

    const assertMatchesEager = (owner: 'b1' | 'b2', expectedHour: number, label: string, opts: { dim?: boolean; mirrorWithheld?: boolean } = {}) => {
      const v = page.view(1);
      const ref = eagerRef(owner, expectedHour);
      assert.equal(v.mode, ref.mode, `${label}: mode`);
      assert.equal(v.dim, opts.dim ?? false, `${label}: dim`);
      assert.equal(v.painted, expectedHour, `${label}: painted hour matches scripted hour`);
      if (ref.mode === 'picture') {
        assert.deepEqual(activeSurface.state.snapshot, ref.picture, `${label}: canvas pixels match eager reference`);
        if (opts.mirrorWithheld) {
          assert.equal(v.fields?.mirror, undefined, `${label}: cadence-withheld mirror is withheld while playing`);
        } else {
          assert.deepEqual(v.fields?.mirror, ref.mirror, `${label}: panel mirror matches eager reference`);
        }
        assert.deepEqual(v.fields?.nav, ref.nav, `${label}: panel nav matches eager reference`);
        assert.deepEqual(v.tooltip, ref.tooltip, `${label}: panel tooltip matches eager reference`);
      } else {
        assert.equal(v.reason, ref.reason, `${label}: refusal reason`);
        assert.equal(activeSurface.state.snapshot, null, `${label}: canvas cleared on refusal`);
      }
    };

    lane.receive('panels', packetFor(11, 3, 'b1', 10, 19, seek));
    await settle(10, { playing: false });
    assertMatchesEager('b1', 10, 'initial paused b1@10');

    lane.receive('panels', packetFor(11, 3, 'b1', 20, 29, seek));
    await settle(22, { playing: true, step: 12, speed: 24 });
    assertMatchesEager('b1', 22, 'advance playing b1@22', { mirrorWithheld: true });
    assert.equal(page.view(1).h, 20);
    assert.equal(page.view(1).through, 22);

    lane.receive('panels', packetFor(11, 3, 'b1', 30, 39, seek));
    await settle(35, { playing: false });
    assertMatchesEager('b1', 35, 'advance paused b1@35');
    assert.equal(page.view(1).h, 30);
    assert.equal(page.view(1).through, 35);

    await settle(45, { playing: false });
    assertMatchesEager('b1', 35, 'paused past confirmed coverage dims while keeping b1@35', { dim: true });
    assert.equal(page.view(1).through, 39);

    const uncoveredBmp: SnapBitmap = { width: 200, snapshot: ['t:4:12=#eef1f6|b1|uncovered'], closed: false, close() { this.closed = true; } };
    const installsBeforeUncovered = activeSurface.state.installs;
    page.offer({
      lane: 0,
      serial: 99_001,
      entries: [{
        widget: 1,
        binding: bindingGen,
        owner: 'b1',
        id: 99_001,
        h: 35,
        c: 39,
        lo: 30,
        hi: 49,
        gen: { seek, reset: 0 },
        follows: false,
        mode: 'picture',
        surfaces: [uncoveredBmp],
        content: { 0: 'uncovered-surf', nav: 'uncovered-nav', mirror: 'uncovered-mirror', tooltip: 'uncovered-tip' },
        fields: {
          nav: ['b1|uncovered'],
          mirror: ['b1|uncovered'],
          tooltip: { key: 'row0', text: { rowKey: 'row0', hour: 35, content: { title: 'uncovered', rows: [] } } },
        },
      }],
    });
    page.frame();
    assert.equal(uncoveredBmp.closed, true, 'uncovered replacement bitmap is closed');
    assert.equal(activeSurface.state.installs, installsBeforeUncovered, 'uncovered replacement never installs over dimmed b1@35');
    assert.ok((page.stats.rejected.uncovered ?? 0) >= 1, 'uncovered replacement is rejected as uncovered');
    assertMatchesEager('b1', 35, 'installed picture, fields and tooltip survive uncovered replacement', { dim: true });

    holdNext = true;
    lane.receive('panels', packetFor(11, 3, 'b1', 40, 49, seek));
    await settle(45, { playing: false });
    assert.equal(heldPresentations.length, 1, 'captured presentation to deliver late');
    const latePres = heldPresentations.pop()!;
    lane.receive('panels', packetFor(11, 3, 'b1', 50, 59, seek));
    await settle(55, { playing: false });
    assertMatchesEager('b1', 55, 'gap [50,69] shows named refusal and clears canvas');

    page.offer(latePres);
    page.frame();
    assert.equal(latePres.entries.every(e => e.surfaces.every(s => !s || s === 'same' || s.closed)), true, 'late pre-gap presentation bitmap is closed');
    assertMatchesEager('b1', 55, 'late pre-gap presentation never overwrites gap refusal');

    lane.receive('panels', packetFor(11, 3, 'b1', 70, 79, seek));
    await settle(75, { playing: false });
    assertMatchesEager('b1', 75, 'leaving gap restores fresh picture b1@75');
    assert.equal(page.view(1).h, 70);

    holdNext = true;
    lane.receive('panels', packetFor(11, 3, 'b1', 80, 89, seek));
    await settle(85, { playing: false });
    const preRewindPres = heldPresentations.pop()!;

    seek++;
    await settle(25, { playing: false, seek });
    page.offer(preRewindPres);
    page.frame();
    assert.equal(preRewindPres.entries.every(e => e.surfaces.every(s => !s || s === 'same' || s.closed)), true, 'late pre-rewind presentation rejected and closed');
    lane.receive('panels', packetFor(11, 3, 'b1', 20, 29, seek));
    await settle(25, { playing: false, seek });
    assertMatchesEager('b1', 25, 'rewind b1@25 matches eager reference');
    assert.equal(page.view(1).h, 20);

    bindingGen++;
    lane.follow(4, 'pinned', 'admission');
    lane.exact(4, 32, { owner: 'b1', value: truth('b1', 32).value! }, null);
    lane.mount(1, [4], 'admission-117', modelFor(4));
    lane.bind(1, { generation: bindingGen, owner: 'b1', surfaces: [{ width: 200, height: 60, dpr: 1 }], offset: 0, pin: 32 });
    page.rebind(1, { owner: 'b1', pin: 32, subscription: true });
    await settle(25, { playing: false, seek });
    assertMatchesEager('b1', 32, 'pinned at 32 shows b1@32');

    seek++;
    lane.receive('panels', packetFor(11, 3, 'b1', 80, 89, seek));
    await settle(85, { playing: false, seek });
    assertMatchesEager('b1', 32, 'pinned at 32 ignores clock advance to 85');

    bindingGen++;
    lane.exact(4, 55, null, truth('b1', 55).refused);
    lane.bind(1, { generation: bindingGen, owner: 'b1', surfaces: [{ width: 200, height: 60, dpr: 1 }], offset: 0, pin: 55 });
    page.rebind(1, { owner: 'b1', pin: 55 });
    await settle(85, { playing: false, seek });
    assertMatchesEager('b1', 55, 'pinning inside gap shows gap refusal and clears canvas');

    bindingGen++;
    lane.mount(1, [3], 'admission-117', modelFor(3));
    lane.bind(1, { generation: bindingGen, owner: 'b1', surfaces: [{ width: 200, height: 60, dpr: 1 }], offset: 0, pin: null });
    page.rebind(1, { owner: 'b1', pin: null, subscription: true });
    await settle(85, { playing: false, seek });
    assertMatchesEager('b1', 85, 'unpinning restores live hour b1@85');
    assert.equal(page.view(1).h, 80);

    holdNext = true;
    lane.receive('panels', packetFor(11, 3, 'b1', 90, 99, seek));
    await settle(95, { playing: false, seek });
    const preRemountPres = heldPresentations.pop()!;

    const s2 = makeSurface();
    activeSurface = s2;
    bindingGen++;
    page.remount(1, [s2.surface]);
    lane.bind(1, { generation: bindingGen, owner: 'b1', surfaces: [{ width: 200, height: 60, dpr: 1 }], offset: 0, pin: null }, undefined, false, true);
    assert.equal(page.view(1).mode, 'reading', 'remount enters reading immediately');
    assert.equal(s2.state.snapshot, null, 'remounted surface starts blank');

    page.offer(preRemountPres);
    page.frame();
    assert.equal(preRemountPres.entries.every(e => e.surfaces.every(s => !s || s === 'same' || s.closed)), true, 'late pre-remount presentation is rejected and closed');
    assert.equal(s2.state.installs, 0, 'late pre-remount presentation never installs on fresh surface');
    assert.equal(s2.state.snapshot, null, 'fresh surface stays blank after late pre-remount reply');
    assert.equal(page.view(1).mode, 'reading', 'page stays reading after late pre-remount reply');
    await settle(95, { playing: false, seek });
    assertMatchesEager('b1', 95, 'remount installs fresh picture matching eager reference');
    assert.equal(page.view(1).h, 90);

    holdNext = true;
    lane.receive('panels', packetFor(11, 3, 'b1', 100, 109, seek));
    await settle(105, { playing: false, seek });
    const preBranchPres = heldPresentations.pop()!;

    seek++;
    bindingGen++;
    lane.follow(5, 'panel', 'admission');
    lane.bindRoute(5, 2, 22, seek);
    lane.mount(1, [5], 'admission-117', modelFor(5));
    lane.bind(1, { generation: bindingGen, owner: 'b2', surfaces: [{ width: 200, height: 60, dpr: 1 }], offset: 0, pin: null });
    page.mountRoute(1, 2);
    page.rebind(1, { owner: 'b2', bounds: [20, 140], subscription: true });

    await settle(12, { playing: false, seek });
    assert.equal(page.view(1).mode, 'outside', 'target 12 outside branch 2 bounds [20,140] reports outside');
    assert.equal(activeSurface.state.snapshot, null, 'canvas cleared outside branch bounds');

    page.offer(preBranchPres);
    page.frame();
    assert.equal(preBranchPres.entries.every(e => e.surfaces.every(s => !s || s === 'same' || s.closed)), true, 'late branch-1 presentation rejected after switch to branch 2');

    lane.receive('panels', packetFor(22, 5, 'b2', 30, 39, seek));
    await settle(35, { playing: false, seek });
    assert.equal(page.view(1).tooltip, null, 'branch switch cleared branch-1 hover inspection');
    page.inspect(1, 'row0');
    await settle(35, { playing: false, seek });
    assertMatchesEager('b2', 35, 'branch 2 b2@35 matches eager reference with zero branch-1 bleed');
    assert.equal(page.view(1).h, 30);

    const wrongOwnerBmp: SnapBitmap = { width: 200, snapshot: ['t:4:12=#eef1f6|b1|wrong-owner'], closed: false, close() { this.closed = true; } };
    const installsBeforeOwner = activeSurface.state.installs;
    page.offer({
      lane: 0,
      serial: 99_002,
      entries: [{
        widget: 1,
        binding: bindingGen,
        owner: 'b1',
        id: 99_002,
        h: 35,
        c: 39,
        lo: 30,
        hi: 39,
        gen: { seek, reset: 0 },
        follows: false,
        mode: 'picture',
        surfaces: [wrongOwnerBmp],
        content: { 0: 'wrong-owner-surf', nav: 'wrong-owner-nav', mirror: 'wrong-owner-mirror', tooltip: 'wrong-owner-tip' },
        fields: {
          nav: ['b1|wrong-owner'],
          mirror: ['b1|wrong-owner'],
          tooltip: { key: 'row0', text: { rowKey: 'row0', hour: 35, content: { title: 'wrong-owner', rows: [] } } },
        },
      }],
    });
    page.frame();
    assert.equal(wrongOwnerBmp.closed, true, 'current-seek wrong-owner bitmap is closed');
    assert.equal(activeSurface.state.installs, installsBeforeOwner, 'current-seek wrong-owner reply never installs on branch 2');
    assert.ok((page.stats.rejected.owner ?? 0) >= 1, 'current-seek wrong-owner reply is rejected by owner admission');
    assertMatchesEager('b2', 35, 'branch 2 canvas and panel survive current-seek wrong-owner delivery');
  } finally {
    recording.restore();
  }
});
