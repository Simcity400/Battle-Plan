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
  const clock = (position: number, seek = 1, speed = 1, generation = 0) => post({ type: 'clock', anchor: { at: performance.timeOrigin + now, position,
    seek, lastHour: 100, playing: true, speed, generation, forwardSeek: true } });
  clock(10);
  post({ type: 'surface', id: 1, owner: 1, canvas, width: 100, height: 20, dpr: 1,
    model: { widget: 'projects', view: 'special-projects', offset: 0, clocked: true, reply, structure: currentKey(reply, 10), cells: [cell] } });
  frame();
  assert.ok(sent.some(m => m.type === 'present' && m.surfaces[0]?.date));
  sent.length = 0;
  clock(10, 1, 2, 4); frame();
  assert.ok(sent.some(m => m.type === 'present' && m.surfaces.some(surface => surface.id === 1 && surface.head === 10)),
    'a speed anchor re-presents a moving surface after the worker applies the new rate');
  assert.ok(sent.some(m => m.type === 'present' && m.surfaces.some(surface => surface.id === 1 && surface.generation === 4)),
    'a panel notice carries the worker anchor generation');
  post({ type: 'audit', start: true });
  now += 17; clock(11); frame();
  now += 17; clock(12); frame();
  post({ type: 'audit', start: false, limit: 1 });
  const firstAudit = sent.filter(m => m.type === 'audit').at(-1);
  assert.equal(firstAudit?.done, false, 'the worker leaves the remaining audit records for the next chunk');
  post({ type: 'audit', start: false, limit: Number.MAX_SAFE_INTEGER });
  const audit = sent.filter(m => m.type === 'audit').at(-1);
  assert.deepEqual([...firstAudit?.records ?? [], ...audit?.records ?? []].filter(r => r.t === 'picture').map(r => r.hour), [11, 12],
    'held surfaces report the valid hour on each presented frame');
  clears.length = 0;
  now += 17; clock(24); frame();
  assert.equal(clears.length, 0, 'a new structure keeps its preceding picture until it can be drawn');
  post({ type: 'surface', id: 1, owner: 2, width: 100, height: 20, dpr: 1,
    model: { widget: 'projects', view: 'special-projects', offset: 0, clocked: true, reply, structure: currentKey(reply, 10), cells: [cell] } });
  now += 17; frame();
  assert.equal(clears.length, 0, 'remeasuring the same slots keeps their dated picture while the next rows are pending');
  sent.length = 0;
  clock(0, 2);
  assert.equal(clears.length, 0, 'a rewind leaves the picture to the next frame');
  frame();
  assert.ok(sent.some(m => m.type === 'present' && m.surfaces.some(s => s.id === 1 && s.seek === 2 && s.readHour === 0)),
    'the next frame presents the rewound hour');

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

  const retainedClears: number[][] = [], raceClears: number[][] = [];
  const retained = { getContext: () => ({ fillText() {}, clearRect: (...args: number[]) => retainedClears.push(args), setTransform() {} }) } as unknown as OffscreenCanvas;
  const raceInk = new Proxy({ measureText: (text: string) => ({ width: text.length * 8 }), clearRect: (...args: number[]) => raceClears.push(args) },
    { get: (o, k) => Reflect.get(o, k) ?? (() => {}) });
  const step = (position: number, seek: number) => post({ type: 'clock', anchor: { at: performance.timeOrigin + now, position,
    seek, lastHour: 100, playing: false, speed: 1, generation: 0, forwardSeek: true } });
  step(10, 10);
  post({ type: 'surface', id: 3, owner: 1, canvas: retained, width: 100, height: 20, dpr: 1,
    model: { widget: 'projects', view: 'special-projects', offset: 0, date: { since: 0, through: 10 }, reply,
      structure: currentKey(reply, 10), cells: [cell] } });
  post({ type: 'race', id: 3, owner: 1, canvas: { width: 500, height: 180, getContext: () => raceInk } as unknown as OffscreenCanvas,
    model: { view: 'nuclear-arsenal', offset: 0, date: { since: 0, through: 10 }, reply: { types: [], countries: [], missing: null, warnings: [] },
      top: 10, tags: [], split: false, overlay: false, colours: new Map(), looks: new Map(), scroll: 0, width: 500, height: 180, slots: 10, dpr: 1,
      tokens: { fontSize: { body: 14, small: 12 }, lineWidth: { grid: 1 } } as never } });
  now += 17; frame();
  const raced = (seek: number) => sent.flatMap(m => m.type === 'racePresent' && m.id === 3 && m.seek === seek ? [m] : []).at(-1);
  assert.equal(raced(10)?.hour, 10, 'the race is painted at the covered hour');
  retainedClears.length = raceClears.length = 0;
  step(11, 11);
  assert.deepEqual([retainedClears.length, raceClears.length], [0, 0], 'an uncovered paused step keeps the old pictures until the worker paints');
  now += 17; frame();
  assert.deepEqual([retainedClears.length, raceClears.length], [0, 0], 'the worker leaves an uncovered picture for its host to release');
  assert.deepEqual([raced(11)?.hour, raced(11)?.waiting], [null, false], 'an uncovered recorded race is presented without a picture and waits for nothing');
  post({ type: 'detach', id: 3 }); post({ type: 'closeRace', id: 3 });
  post({ type: 'race', id: 6, owner: 1, canvas: { width: 500, height: 180, getContext: () => raceInk } as unknown as OffscreenCanvas,
    model: { view: 'nuclear-arsenal', offset: 0, follow: 99, top: 10, tags: [], split: false, overlay: false, colours: new Map(), looks: new Map(),
      scroll: 0, width: 500, height: 180, slots: 10, dpr: 1, tokens: { fontSize: { body: 14, small: 12 }, lineWidth: { grid: 1 } } as never } });
  now += 17; frame();
  const followedRace = sent.flatMap(m => m.type === 'racePresent' && m.id === 6 ? [m] : []).at(-1);
  assert.deepEqual([followedRace?.hour, followedRace?.waiting], [null, true], 'a paused followed race whose hour is not read yet is waiting, not empty');
  post({ type: 'closeRace', id: 6 });

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

  const dayClears: number[][] = [], days: string[] = [];
  const day = { getContext: () => ({ fillText: (value: string) => days.push(value), clearRect: (...args: number[]) => dayClears.push(args), setTransform() {} }) } as unknown as OffscreenCanvas;
  const pause = (position: number, seek: number, speed: number, generation: number, forwardSeek = false) => post({ type: 'clock',
    anchor: { at: performance.timeOrigin + now, position, seek, lastHour: 100, playing: false, speed, generation, forwardSeek } });
  const presented = (generation: number) => sent.filter(m => m.type === 'present').flatMap(m => m.surfaces).filter(s => s.id === 5 && s.generation === generation).at(-1);
  pause(23, 13, 1, 5);
  post({ type: 'surface', id: 5, owner: 1, canvas: day, width: 100, height: 20, dpr: 1,
    model: { view: 'nuclear-arsenal', offset: 0, date: { since: 0, through: 99 },
      reply: { types: [], countries: [], missing: null, warnings: [] }, structure: 'empty', cells: [{ ...cell, key: 'empty' }] } });
  now += 17; frame();
  assert.deepEqual([presented(5)?.readHour, presented(5)?.holdsThrough], [23, 23], 'a paused notice is read at its hour and holds until midnight');
  const lastHourOfDay = days.at(-1);
  dayClears.length = 0;
  pause(23, 13, 168, 6);
  assert.equal(dayClears.length, 0, 'a paused speed change erases nothing before the next frame');
  now += 17; frame();
  assert.equal(dayClears.length, 0, 'a paused speed change erases nothing in the next frame');
  assert.deepEqual([presented(6)?.readHour, presented(6)?.holdsThrough], [23, 23], 'the kept picture is presented under the new anchor at its hour');
  pause(24, 14, 168, 7, true);
  now += 17; frame();
  assert.deepEqual([presented(7)?.readHour, presented(7)?.holdsThrough], [24, 47], 'a paused step past midnight presents the next day');
  assert.notEqual(days.at(-1), lastHourOfDay, 'the notice is repainted with the next day');

  const completing = { branchId: '1', tag: 'GER', unavailable: null, warnings: [], iterationTexts: { 2: 'Iterations: 2', 3: 'Iterations: 3' },
    identities: [{ firstHour: 100, lastHour: 199, slot: 5, countryId: 10 }], available: [{ firstHour: 100, lastHour: 199 }],
    people: [{ assigned: true, id: 77, name: 'Scientist', portrait: null, missing: null }],
    presentations: [{ name: 'Project', specialization: null, specializationName: null, icon: null, iconMissing: null, scientist: 0, scientistAsOf: null }],
    projects: [{ id: 'episode', start: { hour: 100, exact: false }, runs: [
      { firstHour: 100, lastHour: 147, presentation: 0, progress: 17, state: 'running', cancellation: null, completion: null, iterations: 2 },
      { firstHour: 148, lastHour: 199, presentation: 0, progress: 100, state: 'completed', cancellation: null, completion: { hour: 148, exact: true }, iterations: 3 }] }] };
  const layoutClears: number[][] = [], drawing: string[] = [];
  let backing = { width: 0, height: 0 };
  const layout = { get width() { return backing.width; }, set width(width: number) { backing = { ...backing, width }; drawing.push(`width ${width}`); },
    get height() { return backing.height; }, set height(height: number) { backing = { ...backing, height }; drawing.push(`height ${height}`); },
    getContext: () => ({ fillText: () => drawing.push('text'), clearRect: (...args: number[]) => layoutClears.push(args), setTransform() {} }) } as unknown as OffscreenCanvas;
  const project = (owner: number, hour: number, data: unknown = completing, width = 100) => post({ type: 'surface', id: 7, owner, width, height: 20, dpr: 1,
    ...(owner === 1 && { canvas: layout }), model: { widget: 'projects', view: 'special-projects', offset: 0, date: { since: 100, through: 199 }, reply: data,
      structure: currentKey(completing as never, hour), cells: [{ ...cell, key: 'episode' }] } });
  const projectNotice = () => sent.filter(m => m.type === 'present').flatMap(m => m.surfaces).filter(s => s.id === 7).at(-1);
  const stepTo = (position: number, seek: number, generation: number) => post({ type: 'clock',
    anchor: { at: performance.timeOrigin + now, position, seek, lastHour: 199, playing: false, speed: 1, generation, forwardSeek: true } });
  stepTo(147, 15, 8);
  project(1, 147);
  now += 17; frame();
  assert.equal(projectNotice()?.readHour, 147, 'the project list is painted at 147:00');
  layoutClears.length = 0;
  stepTo(148, 16, 9);
  now += 17; frame();
  assert.deepEqual([layoutClears.length, projectNotice()?.readHour, projectNotice()?.date, projectNotice()?.waiting], [0, undefined, undefined, true],
    'a paused step whose reply completes a project keeps the 147:00 pixels and waits for the new rows to be measured');
  drawing.length = 0;
  project(2, 147, completing, 120);
  now += 17; frame();
  assert.deepEqual([drawing, layoutClears.length, projectNotice()?.waiting], [[], 0, true],
    'a held panel resized before its new rows are measured keeps its backing store, and the pixels under the dim, until it can paint');
  project(3, 148, completing, 120);
  now += 17; frame();
  assert.deepEqual([drawing[0], drawing.at(-1), backing, projectNotice()?.owner, projectNotice()?.readHour, projectNotice()?.waiting],
    ['width 120', 'text', { width: 120, height: 20 }, 3, 148, false], 'the measured rows are painted at 148:00 in the frame that resizes the backing store');
  layoutClears.length = 0;
  project(4, 148, { ...completing, projects: 'refused' }, 120);
  now += 17; frame();
  assert.ok(layoutClears.length > 0 && projectNotice()?.owner === 4 && projectNotice()?.waiting === false && projectNotice()?.date === undefined,
    'a reply the view refuses releases the picture and erases it');
});
