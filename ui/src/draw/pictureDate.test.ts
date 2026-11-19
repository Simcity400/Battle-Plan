import assert from 'node:assert/strict';
import test from 'node:test';
import { PictureDate } from './pictureDate.ts';
import { PanelProgram } from './retained/panel.ts';
import type { RowCell } from './panels/view';
import type { ToWorker, FromWorker } from './kit';
import { currentKey } from '../widgets/special-projects/specialProjectsData.ts';

test('a confirmation writes no pixels and a changed value repaints only its slot', () => {
  const painted: string[] = [], erased: number[] = [];
  const context = { fillText: (value: string) => painted.push(value), clearRect: (x: number) => erased.push(x) };
  const program = new PanelProgram(context as unknown as OffscreenCanvasRenderingContext2D);
  const slot: RowCell = { row: 'A', key: 'A:count', text: '3', x: 5, y: 0, width: 40, height: 15,
    baseline: 10, color: 'black', font: '12px Arial', align: 'right' };
  const cells = [slot, { ...slot, x: 65, row: 'B', key: 'B:count' }];
  program.write(cells, new Map());
  program.write(cells, new Map());
  program.write(cells, new Map([['B:count', '123']]));
  assert.deepEqual(painted, ['3', '3', '123']);
  assert.deepEqual(erased, [5, 65, 65]);
  assert.equal(program.hit(70, 5), 'B');
  program.clear();
  assert.equal(program.hit(70, 5), null);
});

test('a fixed surface only writes and hands off confirmed coverage of its selected hour', () => {
  const date = new PictureDate();
  for (const hour of [null, 39, 41]) {
    date.set(hour, 40);
    assert.equal(date.canPaintAt(80), false);
    date.paint();
    assert.equal(date.presentableAt(80), false);
  }
  date.set(45, 40, 38);
  assert.equal(date.canPaintAt(80), true);
  date.paint();
  assert.equal(date.presentableAt(80), true);
  date.set(20, 20);
  assert.equal(date.presentableAt(80), false);
});

test('the picture audit detects text that extends beyond its reserved slot', () => {
  const ctx = { fillText() {}, clearRect() {}, measureText: (text: string) => ({ width: text.length * 8 }) };
  const panel = new PanelProgram(ctx as unknown as OffscreenCanvasRenderingContext2D);
  const cell: RowCell = { row: '1', key: '1:percent', text: '0%', x: 0, y: 0, width: 16, height: 16,
    baseline: 12, color: 'black', font: '12px Arial', align: 'left' };
  panel.write([cell], new Map([['1:percent', '100%']]));
  assert.equal(panel.snapshot()[0]?.overflow, 16);
  panel.write([{ ...cell, width: 32 }], new Map([['1:percent', '100%']]));
  assert.equal(panel.snapshot()[0]?.overflow, undefined);
});

test('a clocked panel keeps its eligible pixels while React measures a new structure', async t => {
  let now = 1000;
  t.mock.method(performance, 'now', () => now);
  let render: FrameRequestCallback | undefined;
  const sent: FromWorker[] = [];
  const scope = { postMessage: (message: FromWorker) => sent.push(message), onmessage: undefined as ((e: { data: ToWorker }) => void) | undefined };
  t.after(() => { Reflect.deleteProperty(globalThis, 'self'); Reflect.deleteProperty(globalThis, 'requestAnimationFrame'); });
  Object.assign(globalThis, { self: scope, requestAnimationFrame: (fn: FrameRequestCallback) => { render = fn; return 1; } });
  await import('./worker.ts');
  const post = (data: ToWorker) => scope.onmessage!({ data });
  const frame = () => { const cb = render!; render = undefined; cb(performance.now()); };
  const reply = { branchId: '1', tag: 'GER', unavailable: null, iterationTexts: {},
    identities: [{ firstHour: 0, lastHour: 99, slot: 5, countryId: 10 }],
    available: [{ firstHour: 0, lastHour: 23 }], projects: [], presentations: [], people: [], warnings: [] };
  const clears: number[][] = [];
  const ctx = new Proxy({ clearRect: (...args: number[]) => clears.push(args), measureText: (s: string) => ({ width: s.length * 5 }) },
    { get: (o, k) => Reflect.get(o, k) ?? (() => {}) });
  const canvas = { getContext: () => ctx } as unknown as OffscreenCanvas;
  const cell: RowCell = { key: 'none', row: 'none', text: 'No projects in progress.', x: 0, y: 0,
    width: 90, height: 12, baseline: 10, font: '12px Arial', color: 'black', align: 'left' };
  const clock = (position: number, seek = 1) => post({ type: 'clock', anchor: { at: performance.timeOrigin + now, position,
    seek, lastHour: 100, playing: true, speed: 1, forwardSeek: true } });
  clock(10);
  post({ type: 'surface', id: 1, owner: 1, canvas, width: 100, height: 20, dpr: 1,
    model: { widget: 'projects', view: 'special-projects', offset: 0, clocked: true, reply, structure: currentKey(reply, 10), cells: [cell] } });
  frame();
  assert.ok(sent.some(m => m.type === 'present' && m.surfaces[0]?.date));
  post({ type: 'audit', start: true });
  now += 17; clock(11); frame();
  now += 17; clock(12); frame();
  post({ type: 'audit', start: false });
  const audit = sent.filter(m => m.type === 'audit').at(-1);
  assert.deepEqual(audit?.records.filter(r => r.t === 'picture').map(r => r.hour), [11, 12],
    'held surfaces report the valid hour on each presented frame');
  clears.length = 0;
  now += 17; clock(24); frame();
  assert.equal(clears.length, 0, 'a new structure keeps its preceding picture until it can be drawn');
  post({ type: 'surface', id: 1, owner: 2, width: 100, height: 20, dpr: 1,
    model: { widget: 'projects', view: 'special-projects', offset: 0, clocked: true, reply, structure: currentKey(reply, 10), cells: [cell] } });
  now += 17; frame();
  assert.equal(clears.length, 0, 'remeasuring the same slots keeps their dated picture while the next rows are pending');
  clock(0, 2); frame();
  assert.ok(clears.length > 0, 'rewind erases the retained picture');

  const painted: string[] = [];
  const ink = { getContext: () => ({ fillText: (value: string) => painted.push(value), clearRect() {}, setTransform() {} }) } as unknown as OffscreenCanvas;
  post({ type: 'surface', id: 2, owner: 1, canvas: ink, width: 100, height: 20, dpr: 1,
    model: { view: 'nuclear-arsenal', offset: 0, date: { since: 0, through: 99 },
      reply: { types: [], countries: [], missing: null, warnings: [] }, structure: 'empty', cells: [{ ...cell, key: 'empty' }] } });
  now += 17; clock(23, 2); frame();
  const firstDay = painted.at(-1);
  now += 17; clock(24, 2); frame();
  assert.ok(firstDay && painted.at(-1) !== firstDay, 'the held notice repaints at midnight without a new reading');
  post({ type: 'clock', anchor: { position: 100, seek: 2, lastHour: 100, playing: false, speed: 1, at: performance.timeOrigin + now } });
  now += 17; frame();
  const paused = sent.filter(m => m.type === 'present').flatMap(m => m.surfaces).filter(s => s.id === 2).at(-1);
  assert.ok(paused && !paused.date, 'a paused hour outside the confirmed interval clears the previous picture');

  const retainedClears: number[][] = [];
  const retained = { getContext: () => ({ fillText() {}, clearRect: (...args: number[]) => retainedClears.push(args), setTransform() {} }) } as unknown as OffscreenCanvas;
  clock(10, 10);
  post({ type: 'surface', id: 3, owner: 1, canvas: retained, width: 100, height: 20, dpr: 1,
    model: { widget: 'projects', view: 'special-projects', offset: 0, date: { since: 0, through: 10 }, reply,
      structure: currentKey(reply, 10), cells: [cell] } });
  now += 17; frame();
  retainedClears.length = 0;
  clock(11, 11);
  assert.ok(retainedClears.length > 0, 'an uncovered forward seek clears the old picture before the next frame');
  post({ type: 'detach', id: 3 });

  const coveredClears: number[][] = [];
  const covered = { getContext: () => ({ fillText() {}, clearRect: (...args: number[]) => coveredClears.push(args), setTransform() {} }) } as unknown as OffscreenCanvas;
  post({ type: 'surface', id: 4, owner: 1, canvas: covered, width: 100, height: 20, dpr: 1,
    model: { widget: 'projects', view: 'special-projects', offset: 0, date: { since: 0, through: 99 }, reply,
      structure: currentKey(reply, 10), cells: [cell] } });
  now += 17; frame();
  coveredClears.length = 0; sent.length = 0;
  clock(12, 12);
  assert.equal(coveredClears.length, 0, 'a covered forward seek retains the old picture');
  now += 17; frame();
  assert.ok(sent.some(m => m.type === 'present' && m.surfaces.some(s => s.id === 4 && s.date?.through === 99 && s.seek === 12)),
    'a retained picture is reported under the new seek');
});
