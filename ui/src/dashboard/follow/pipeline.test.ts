import assert from 'node:assert/strict';
import { test } from 'node:test';
import { inProcess, NATIVE } from './inProcess.fixture.ts';
import { datedReading, datedStore, heldFor, joinReadings, onInstalled, readingOf } from './useFollowed.ts';
import { message } from './messages.fixture.ts';
import { cellValue, parseAnswer } from './message.ts';
import type { PageSurface } from './page.ts';
import type { LaneView } from '../../draw/lane';
import { installedEntry } from './store.ts';
import { classifyFailure, ShownError } from '../queryError.ts';
import { followFailure } from './useFollow.ts';

const surface = (): PageSurface<ImageBitmap> & { cleared: number } => ({ cleared: 0, install() {}, clear() { this.cleared++; } });

test('closing a pipeline closes every lane channel, though no follow ever opened them', () => {
  const { channels, close } = inProcess({ lanes: 2 });
  assert.deepEqual([...channels.values()], [false, false]);
  close();
  assert.deepEqual([...channels.values()], [true, true], 'a channel left open keeps the page, or a test process, alive');
});

test('astra 8: a child binding its first canvas, a view change and a new array of the same canvas keep the shared picture', async () => {
  const drawn: LaneView = { paint: input => ({ surfaces: input.binding.surfaces.map(() => ({ source: { width: 1, height: 1 }, content: 'drawn' })), fields: {}, motion: false }) };
  const { pipeline, clock, send, source, settle, close } = inProcess({ views: kind => kind === 'drawn' ? drawn : undefined });
  try {
    const id = pipeline.acquire('widget|0', { offset: 0, pin: null });
    const sub = pipeline.subscribe(id, {}, source, '1', 0, { product: 'series-state' }, 'followed')!;
    clock.seek(10);
    await settle(() => pipeline.status(sub).status === 'followed');
    send(message(10, [{ id: sub, answer: { keys: [1n], columns: 1, values: [7n] } }], false, NATIVE, clock.getSnapshot().seekSequence));
    assert.ok(await settle(() => pipeline.view(id)?.mode === 'picture'), 'the parent installs its picture');
    const canvas = surface();
    pipeline.present(id, 'drawn');
    assert.equal(pipeline.view(id)?.mode, 'picture', 'a new painter holds the installed picture until its own installs');
    pipeline.bind(id, [canvas], { surfaces: [{ width: 1, height: 1, dpr: 1 }] });
    assert.equal(pipeline.view(id)?.mode, 'picture', 'the first canvas has no pixels to withdraw');
    assert.ok(await settle(() => pipeline.view(id)?.mode === 'picture' && !pipeline.view(id)!.dim), JSON.stringify([pipeline.view(id), pipeline.page.stats, pipeline.page.widgets.get(id)?.binding]));
    pipeline.bind(id, [canvas], { surfaces: [{ width: 2, height: 1, dpr: 1 }] });
    assert.deepEqual([pipeline.view(id)?.mode, canvas.cleared], ['picture', 0], 'a resize names the same canvas: held dimmed, never cleared');
    assert.equal(pipeline.held(id, 'drawn', () => surface()), pipeline.held(id, 'drawn', () => surface()), 'a remounted child gets the canvas the pipeline holds');
  } finally { close(); }
});

test('review 4 finding 3: a copy joining a registered subscription takes its route at once and so sees its resets', async () => {
  const { pipeline, clock, send, source, settle, close } = inProcess();
  try {
    const query = { product: 'series-state', params: { branchId: '1', reads: 'every' } };
    const one = pipeline.acquire('one|0', { offset: 0, pin: null });
    const sub = pipeline.subscribe(one, {}, source, '1', 0, query, 'followed')!;
    clock.seek(10);
    await settle(() => pipeline.status(sub).status === 'followed');
    const two = pipeline.acquire('two|0', { offset: 0, pin: null });
    assert.equal(pipeline.subscribe(two, {}, source, '1', 0, query, 'followed'), sub);
    assert.equal(pipeline.page.widgets.get(two)!.route, pipeline.page.widgets.get(one)!.route, 'the late copy is on the route of the subscription it shares');
    const seek = clock.getSnapshot().seekSequence;
    send(message(10, [{ id: sub, answer: { keys: [1n], columns: 1, values: [5n] } }], false, NATIVE, seek));
    assert.ok(await settle(() => pipeline.view(two)?.mode === 'picture'));
    send(message(10, [], true, NATIVE, seek));
    assert.ok(await settle(() => pipeline.view(two)?.mode === 'reading'), 'an empty reset withdraws the late copy\'s picture');
  } finally { close(); }
});

test('review 4 finding 14: under audit each lane records the frames it served, and a paused install is sampled when it commits', async () => {
  const { capturePictures } = await import('../pulled.ts');
  const { pipeline, clock, send, source, settle, close } = inProcess();
  try {
    await capturePictures(true);
    const id = pipeline.acquire('widget|0', { offset: 0, pin: null });
    const sub = pipeline.subscribe(id, {}, source, '1', 0, { product: 'series-state' }, 'followed')!;
    clock.seek(10);
    await settle(() => pipeline.status(sub).status === 'followed');
    send(message(10, [{ id: sub, answer: { keys: [1n], columns: 1, values: [7n] } }], false, NATIVE, clock.getSnapshot().seekSequence));
    assert.ok(await settle(() => pipeline.view(id)?.mode === 'picture'));
    const records = await capturePictures(false) as { t: string; lane?: number; seek?: number; head?: number; widget?: string; hour?: number | null }[];
    const frames = records.filter(record => record.t === 'workerFrame');
    assert.ok(frames.some(frame => frame.lane === 0 && frame.seek === clock.getSnapshot().seekSequence && frame.head === 10), JSON.stringify(frames.slice(-3)));
    assert.ok(records.some(record => record.t === 'picture' && record.widget === 'widget' && record.hour === 10), 'the picture installed while paused is sampled, under its widget id');
    const sampled = records.find(record => record.t === 'picture' && record.widget === 'widget' && record.hour === 10) as { entry?: number; parts?: string[]; eligibleHour?: number | null; binding?: number };
    assert.deepEqual([sampled.entry, sampled.parts?.length, sampled.binding], [pipeline.view(id)!.id, Object.keys(pipeline.page.widgets.get(id)!.content).length, pipeline.page.widgets.get(id)!.installed!.binding],
      'review 5 finding 19: the sample names the installed picture, its parts and the binding it was drawn for');
  } finally { close(); }
});

test('review 5 finding 7: a subscription dropped while its channel opens or its follow is answered is never left followed natively', async () => {
  let drop: (() => void) | null = null;
  const { pipeline, source, asked, settle, close } = inProcess({ follow: () => { drop?.(); drop = null; return { available: true }; } });
  try {
    const early = pipeline.acquire('early|0', { offset: 0, pin: null }), earlyToken = {};
    pipeline.subscribe(early, earlyToken, source, '1', 0, { product: 'series-state', params: { n: 1 } }, 'followed');
    pipeline.unsubscribe(early, earlyToken);
    const late = pipeline.acquire('late|0', { offset: 0, pin: null }), lateToken = {};
    const sub = pipeline.subscribe(late, lateToken, source, '1', 0, { product: 'series-state', params: { n: 2 } }, 'followed')!;
    drop = () => pipeline.unsubscribe(late, lateToken);
    await settle(() => asked.some(ask => ask.product === 'unfollow'));
    const ids = (product: string) => asked.filter(ask => ask.product === product).map(ask => ask.params.id);
    assert.deepEqual(ids('follow'), [sub], 'the copy dropped before the channel opened was never followed');
    assert.deepEqual(ids('unfollow'), [sub], 'the copy dropped while its follow was answered is unfollowed when the reply lands');
  } finally { close(); }
});

test('review 5 finding 14: a failed stream stays failed through a resubscription and a new canvas until it recovers', async () => {
  const drawn: LaneView = { paint: input => ({ surfaces: input.binding.surfaces.map(() => ({ source: { width: 1, height: 1 }, content: `drawn:${input.hour}` })), fields: {}, motion: false }) };
  const { pipeline, clock, send, fail, source, settle, close } = inProcess({ views: kind => kind === 'drawn' ? drawn : undefined });
  try {
    const id = pipeline.acquire('widget|0', { offset: 0, pin: null });
    const query = { product: 'series-state' };
    const sub = pipeline.subscribe(id, {}, source, '1', 0, query, 'followed')!;
    pipeline.present(id, 'drawn');
    pipeline.bind(id, [surface()], { surfaces: [{ width: 1, height: 1, dpr: 1 }] });
    clock.seek(10);
    await settle(() => pipeline.status(sub).status === 'followed');
    send(message(10, [{ id: sub, answer: { keys: [1n], columns: 1, values: [7n] } }], false, NATIVE, clock.getSnapshot().seekSequence));
    assert.ok(await settle(() => pipeline.view(id)?.mode === 'picture'));
    fail('The pushed answers stopped');
    assert.ok(await settle(() => pipeline.view(id)?.mode === 'failed'));
    const token = {};
    pipeline.subscribe(id, token, source, '1', 0, { product: 'series-state', params: { again: true } }, 'followed');
    pipeline.unsubscribe(id, token);
    pipeline.bind(id, [surface()], { surfaces: [{ width: 2, height: 1, dpr: 1 }] });
    await settle(() => false, 20);
    assert.deepEqual([pipeline.view(id)?.mode, pipeline.view(id)?.reason], ['failed', 'The pushed answers stopped'], 'no rebind revives the picture from before the failure');
    clock.seek(12);
    send(message(12, [{ id: sub, answer: { keys: [1n], columns: 1, values: [8n] } }], true, NATIVE, clock.getSnapshot().seekSequence));
    assert.ok(await settle(() => pipeline.view(id)?.mode === 'picture'), 'a new stream recovers it');
  } finally { close(); }
});

test('review 5 finding 16: each pointer is answered with the row under its own position, never the one whose tooltip is installed', async () => {
  const drawn: LaneView = { paint: input => ({ surfaces: input.binding.surfaces.map(() => ({ source: { width: 1, height: 1 }, content: 'rows' })), fields: {}, motion: false,
    hit: point => point[0] < 5 ? 'race:A' : 'race:B' }) };
  const { pipeline, clock, send, source, settle, close } = inProcess({ views: kind => kind === 'drawn' ? drawn : undefined });
  try {
    const id = pipeline.acquire('widget|0', { offset: 0, pin: null });
    const sub = pipeline.subscribe(id, {}, source, '1', 0, { product: 'series-state' }, 'followed')!;
    pipeline.present(id, 'drawn');
    pipeline.bind(id, [surface()], { surfaces: [{ width: 10, height: 1, dpr: 1 }] });
    clock.seek(10);
    await settle(() => pipeline.status(sub).status === 'followed');
    send(message(10, [{ id: sub, answer: { keys: [1n], columns: 1, values: [7n] } }], false, NATIVE, clock.getSnapshot().seekSequence));
    assert.ok(await settle(() => pipeline.view(id)?.mode === 'picture'));
    const heard: (string | null)[] = [];
    pipeline.pointer(id, 0, [1, 0]);
    assert.ok(await settle(() => pipeline.inspecting(id) === 'race:A'), 'hovering A inspects A');
    pipeline.pointer(id, 0, [9, 0], key => heard.push(key));
    pipeline.pointer(id, 0, [1, 0]);
    pipeline.pointer(id, 0, [9, 0], key => heard.push(key));
    assert.ok(await settle(() => heard.length === 2));
    assert.deepEqual(heard, ['race:B', 'race:B'], 'a click on B acts on B though A was inspected in between');
  } finally { close(); }
});

test('review 6 C: a route nobody follows is sent no clock, so a widget scrolled back in installs on the same seek', async () => {
  const { pipeline, clock, send, source, asked, settle, close } = inProcess();
  try {
    const query = { product: 'series-state' };
    const first = pipeline.acquire('widget|0', { offset: 0, pin: null });
    const sub = pipeline.subscribe(first, {}, source, '1', 0, query, 'followed')!;
    clock.seek(10);
    await settle(() => pipeline.status(sub).status === 'followed');
    pipeline.drop('widget|0');
    const before = asked.filter(ask => ask.product === 'follow-clock').length;
    clock.step(1);
    await settle(() => false, 10);
    assert.equal(asked.filter(ask => ask.product === 'follow-clock').length, before, 'no clock command without a follower');
    const again = pipeline.acquire('widget|0', { offset: 0, pin: null });
    const back = pipeline.subscribe(again, {}, source, '1', 0, query, 'followed')!;
    await settle(() => pipeline.status(back).status === 'followed');
    send(message(11, [{ id: back, answer: { keys: [1n], columns: 1, values: [3n] } }], false, NATIVE, clock.getSnapshot().seekSequence));
    assert.ok(await settle(() => pipeline.view(again)?.mode === 'picture'), JSON.stringify(pipeline.view(again)));
  } finally { close(); }
});

test('review 6 B: widgets read one reading state from the installed entries, and a refused one is never read as empty', () => {
  const covered = readingOf({ mode: 'picture', h: 10, through: 12, target: 12, dim: false, tooltip: null });
  const refused = readingOf({ mode: 'failed', reason: 'Casualties dealt are not recorded', target: 12, dim: false, tooltip: null });
  const pending = readingOf({ mode: 'reading', target: 12, dim: false, tooltip: null });
  assert.deepEqual(joinReadings([covered, refused, pending]), { state: 'refused', reason: 'Casualties dealt are not recorded' });
  assert.deepEqual(joinReadings([covered, pending]), { state: 'pending' });
  assert.deepEqual(joinReadings([covered, readingOf({ mode: 'picture', h: 11, through: 11, target: 12, dim: true, tooltip: null }), null]),
    { state: 'covered', hour: 11, dim: true }, 'the joined picture is dated at its latest hour and dimmed if any part is');
});

test('review 7 A: a component joining a shown widget keeps its picture, dimmed, until the joined picture installs', async () => {
  const { pipeline, clock, send, source, settle, close } = inProcess();
  try {
    const id = pipeline.acquire('widget|0', { offset: 0, pin: null });
    const first = pipeline.subscribe(id, {}, source, '1', 0, { product: 'series-state', params: { n: 1 } }, 'followed')!;
    clock.seek(10);
    await settle(() => pipeline.status(first).status === 'followed');
    send(message(10, [{ id: first, answer: { keys: [1n], columns: 1, values: [7n] } }], false, NATIVE, clock.getSnapshot().seekSequence));
    assert.ok(await settle(() => pipeline.view(id)?.mode === 'picture'));
    const second = pipeline.subscribe(id, {}, source, '1', 0, { product: 'series-state', params: { n: 2 } }, 'followed')!;
    assert.deepEqual([pipeline.view(id)?.mode, pipeline.view(id)?.dim], ['picture', true], 'the shown picture stays, dimmed, while the join is pending');
    await settle(() => pipeline.status(second).status === 'followed');
    send(message(10, [{ id: second, answer: { keys: [1n], columns: 1, values: [8n] } }], false, NATIVE, clock.getSnapshot().seekSequence));
    assert.ok(await settle(() => pipeline.view(id)?.mode === 'picture' && pipeline.view(id)?.dim === false), JSON.stringify(pipeline.view(id)));
  } finally { close(); }
});

test('review 7 D: a widget rejoining a route that native serves from a new host installs on the same seek', async () => {
  let route = NATIVE;
  const { pipeline, clock, send, source, settle, close } = inProcess({ follow: () => ({ available: true, route }) });
  try {
    const query = { product: 'series-state' };
    const first = pipeline.acquire('widget|0', { offset: 0, pin: null });
    const sub = pipeline.subscribe(first, {}, source, '1', 0, query, 'followed')!;
    clock.seek(10);
    await settle(() => pipeline.status(sub).status === 'followed');
    const seek = clock.getSnapshot().seekSequence;
    for (const hour of [9, 10]) send(message(hour, [{ id: sub, answer: { keys: [1n], columns: 1, values: [BigInt(hour)] } }], true, NATIVE, seek));
    assert.ok(await settle(() => pipeline.view(first)?.mode === 'picture' && pipeline.view(first)?.h === 10));
    pipeline.drop('widget|0');
    route = NATIVE + 1;
    const again = pipeline.acquire('widget|0', { offset: 0, pin: null });
    const back = pipeline.subscribe(again, {}, source, '1', 0, query, 'followed')!;
    await settle(() => pipeline.status(back).status === 'followed');
    send(message(10, [{ id: back, answer: { keys: [1n], columns: 1, values: [3n] } }], false, route, seek));
    assert.ok(await settle(() => pipeline.view(again)?.mode === 'picture'), JSON.stringify(pipeline.view(again)));
  } finally { close(); }
});

test('review 7 F: a pinned placement that shows an asked answer stays sampled with the hour and rows it shows', async () => {
  const { capturePictures } = await import('../pulled.ts');
  const { pipeline, clock, close } = inProcess();
  try {
    await capturePictures(true);
    const id = pipeline.acquire('pinned|0', { offset: 0, pin: 40 });
    pipeline.answer(id, -1000, { hour: 40, rows: () => [{ row: 'a', key: 'a', value: '7' }] });
    clock.seek(50);
    const records = await capturePictures(false) as { t: string; placement?: number; owner?: number; surface?: string; hour?: number | null; fixedHour?: number; mode?: string; rows?: unknown[] }[];
    const sampled = records.filter(record => record.t === 'picture' && record.placement === id);
    assert.ok(sampled.length > 0 && sampled.every(record => record.hour === 40 && record.fixedHour === 40 && record.mode === 'picture' && record.rows?.length === 1), JSON.stringify(sampled.slice(-2)));

    await capturePictures(true);
    const table = pipeline.acquire('table|0', { offset: 0, pin: 40, view: 'table' });
    pipeline.audit(table, -1, () => [{ row: 'GER', key: 'GER', value: '1' }]);
    pipeline.answer(table, -1001, { hour: 40, rows: () => [{ row: 'x', key: 'x', value: '1' }] });
    pipeline.answer(table, -1002, { hour: 40, rows: () => [{ row: 'y', key: 'y', value: '2' }] });
    pipeline.answer(table, -1001, null);
    const owned = (await capturePictures(false) as typeof records).filter(record => record.t === 'picture' && record.placement === table);
    const last = owned.slice(-2);
    assert.deepEqual(last.map(record => [record.owner, record.surface]).sort(), [[-1, 'table'], [-1002, 'table']],
      'review 8 5: a changed pinned answer is sampled at once, each component under its own owner and the active renderer, and only the departing one is removed');
  } finally { close(); }
});

test('review 8 1: every page command carries its session, and a clock command queued before the last follower left never reaches the route it rejoins', async () => {
  let open: (() => void) | null = null;
  const held = new Promise<void>(resolve => { open = resolve; });
  let gated = true;
  const { pipeline, clock, source, asked, settle, close } = inProcess({ session: 'page-2', gate: product => product === 'follow-clock' && gated ? held : undefined });
  try {
    const query = { product: 'series-state' };
    const first = pipeline.acquire('widget|0', { offset: 0, pin: null });
    const sub = pipeline.subscribe(first, {}, source, '1', 0, query, 'followed')!;
    clock.seek(10);
    await settle(() => pipeline.status(sub).status === 'followed' && asked.some(ask => ask.product === 'follow-clock'));
    clock.step(1);
    pipeline.drop('widget|0');
    const again = pipeline.acquire('widget|0', { offset: 0, pin: null });
    const back = pipeline.subscribe(again, {}, source, '1', 0, query, 'followed')!;
    await settle(() => pipeline.status(back).status === 'followed');
    gated = false;
    open!();
    await settle(() => false, 20);
    const clocks = asked.filter(ask => ask.product === 'follow-clock');
    assert.equal(clocks.length, 2, 'the first command and the rejoined route\'s own; the one queued for the left route is dropped');
    assert.ok(asked.every(ask => ask.params.session === 'page-2'), 'follow, unfollow and follow-clock all name the page session');
  } finally { close(); }
});

test('review 8 3: a numeric update keeps the structure React reads, while date readers see each painted hour', async () => {
  const { pipeline, clock, send, source, settle, close } = inProcess();
  try {
    const id = pipeline.acquire('widget|0', { offset: 0, pin: null });
    const sub = pipeline.subscribe(id, {}, source, '1', 0, { product: 'series-state' }, 'followed')!;
    clock.seek(10);
    await settle(() => pipeline.status(sub).status === 'followed');
    const seek = clock.getSnapshot().seekSequence;
    send(message(10, [{ id: sub, answer: { keys: [1n], columns: 1, values: [7n] } }], false, NATIVE, seek));
    assert.ok(await settle(() => pipeline.view(id)?.mode === 'picture'));
    const structure = pipeline.view(id)!.structure, dated = datedStore(pipeline, id);
    let told = 0;
    const stop = dated.subscribe(() => told++);
    assert.equal(dated.read(), 10);
    for (let at = 0; at < 6; at++) clock.step(1);
    send(message(16, [{ id: sub, answer: { keys: [1n], columns: 1, values: [8n] } }], false, NATIVE, seek));
    assert.ok(await settle(() => dated.read() === 16), String(dated.read()));
    assert.ok(told > 0, 'a date reader hears the later paint hour');
    assert.equal(pipeline.view(id)!.structure, structure, 'the value change keeps the structure React commits on');
    send(message(16, [{ id: sub, answer: { keys: [1n, 2n], columns: 1, values: [8n, 9n] } }], false, NATIVE, seek));
    assert.ok(await settle(() => pipeline.view(id)!.structure !== structure), 'a new row is a new structure');
    stop();
  } finally { close(); }
});

test('review 9: value-dependent output runs on every install, while React keeps one structure', async () => {
  const { pipeline, clock, send, source, settle, close } = inProcess();
  try {
    const id = pipeline.acquire('arsenal|0', { offset: 0, pin: null });
    const sub = pipeline.subscribe(id, {}, source, '1', 0, { product: 'series-state' }, 'followed')!;
    clock.seek(10);
    await settle(() => pipeline.status(sub).status === 'followed');
    const seek = clock.getSnapshot().seekSequence;
    send(message(10, [{ id: sub, answer: { keys: [1n], columns: 1, values: [2n] } }], false, NATIVE, seek));
    assert.ok(await settle(() => pipeline.view(id)?.mode === 'picture'));
    const totals: bigint[] = [];
    const stop = onInstalled(pipeline, id, () => totals.push(parseAnswer(pipeline.view(id)!.fields!['answer:' + sub] as Uint8Array).kind === 'notRecorded' ? -1n
      : cellValue(parseAnswer(pipeline.view(id)!.fields!['answer:' + sub] as Uint8Array) as Exclude<ReturnType<typeof parseAnswer>, { kind: 'notRecorded' }>, 0) as bigint));
    const structure = pipeline.view(id)!.structure;
    for (let at = 0; at < 2; at++) clock.step(1);
    send(message(12, [{ id: sub, answer: { keys: [1n], columns: 1, values: [3n] } }], false, NATIVE, seek));
    assert.ok(await settle(() => totals.at(-1) === 3n), JSON.stringify(totals.map(String)));
    assert.equal(pipeline.view(id)!.structure, structure, 'the stockpile going from 2 to 3 with the same rows is painted, not committed');
    stop();
    assert.deepEqual([heldFor({ key: 'parent|0', id: 4 }, 'parent|0|detail'), heldFor({ key: 'detail', id: 5 }, 'detail')], [null, 5],
      'R2-S5: a detail never times the id acquired for its parent');
  } finally { close(); }
});

test('review 9: a rejoin keeps its route\'s named failure, and a detail\'s samples name their widget', async () => {
  const { capturePictures } = await import('../pulled.ts');
  const { pipeline, clock, source, settle, fail, close } = inProcess();
  try {
    const query = { product: 'series-state' };
    const first = pipeline.acquire('widget|0', { offset: 0, pin: null });
    const sub = pipeline.subscribe(first, {}, source, '1', 0, query, 'followed')!;
    clock.seek(10);
    await settle(() => pipeline.status(sub).status === 'followed');
    fail('The followed stream broke');
    assert.ok(await settle(() => pipeline.view(first)?.mode === 'failed'));
    pipeline.drop('widget|0');
    const again = pipeline.acquire('widget|0', { offset: 0, pin: null });
    const back = pipeline.subscribe(again, {}, source, '1', 0, query, 'followed')!;
    await settle(() => pipeline.status(back).status === 'followed');
    assert.deepEqual([pipeline.view(again)?.mode, pipeline.view(again)?.reason], ['failed', 'The followed stream broke'], 'R3-S4');
    await capturePictures(true);
    const detail = pipeline.acquire('card|0|detail', { offset: 0, pin: 40, widget: 'card' });
    pipeline.answer(detail, -1000, { hour: 40, rows: () => [] });
    const named = (await capturePictures(false) as { t: string; placement?: number; widget?: string }[]).filter(record => record.t === 'picture' && record.placement === detail);
    assert.ok(named.length > 0 && named.every(record => record.widget === 'card'), 'R3-S7: ' + JSON.stringify(named.slice(-1)));
  } finally { close(); }
});

test('review 10: a reading\'s hour is the installed date when it is read, and the installed structure is made once', () => {
  let hour: number | null = 10;
  const reading = datedReading({ state: 'covered', hour: 10, dim: false }, { read: () => hour });
  hour = 16;
  assert.equal(reading.state === 'covered' && reading.hour, 16, 'a same-structure install from 10 to 16 reads 16');
  assert.deepEqual(datedReading({ state: 'pending' }, { read: () => 16 }), { state: 'pending' });
});

test('transport or host errors on open or follow stay retryable failures distinct from recording refusals, and retry recovers them', async () => {
  let openAttempts = 0;
  const opened = inProcess({
    open: async () => {
      openAttempts++;
      if (openAttempts === 1) throw new Error('IPC pipe disconnected');
      return 'session-1';
    },
  });
  try {
    const id = opened.pipeline.acquire('open-fail|0', { offset: 0, pin: null });
    const sub = opened.pipeline.subscribe(id, {}, opened.source, '1', 0, { product: 'series-state' }, 'followed')!;
    opened.clock.seek(10);
    assert.ok(await opened.settle(() => opened.pipeline.status(sub).status === 'failed'));
    assert.equal(opened.pipeline.status(sub).reason, 'IPC pipe disconnected');
    assert.equal(opened.pipeline.view(id)?.mode, 'failed');
    opened.pipeline.retry(sub);
    assert.ok(await opened.settle(() => opened.pipeline.status(sub).status === 'followed'));
    opened.send(message(10, [{ id: sub, answer: { keys: [1n], columns: 1, values: [9n] } }], false, NATIVE, opened.clock.getSnapshot().seekSequence));
    assert.ok(await opened.settle(() => opened.pipeline.view(id)?.mode === 'picture'));
  } finally { opened.close(); }

  let followAttempts = 0;
  const asked = inProcess({
    follow: (_id, query) => {
      if ((query.params as { mode?: string } | undefined)?.mode === 'refused') return { available: false, reason: 'Table absent in format 141' };
      followAttempts++;
      if (followAttempts === 1) throw new Error('Host follow timed out');
      return { available: true };
    },
  });
  try {
    const failedId = asked.pipeline.acquire('follow-fail|0', { offset: 0, pin: null });
    const failedSub = asked.pipeline.subscribe(failedId, {}, asked.source, '1', 0, { product: 'series-state', params: { mode: 'fail' } }, 'followed')!;
    const refusedId = asked.pipeline.acquire('follow-refuse|0', { offset: 0, pin: null });
    const refusedSub = asked.pipeline.subscribe(refusedId, {}, asked.source, '1', 0, { product: 'series-state', params: { mode: 'refused' } }, 'followed')!;
    asked.clock.seek(10);
    assert.ok(await asked.settle(() => asked.pipeline.status(failedSub).status === 'failed' && asked.pipeline.status(refusedSub).status === 'refused'));
    assert.equal(asked.pipeline.status(failedSub).reason, 'Host follow timed out');
    assert.equal(asked.pipeline.status(refusedSub).reason, 'Table absent in format 141');
    asked.pipeline.retry(failedSub);
    assert.ok(await asked.settle(() => asked.pipeline.status(failedSub).status === 'followed'));
    asked.send(message(10, [{ id: failedSub, answer: { keys: [1n], columns: 1, values: [4n] } }], false, NATIVE, asked.clock.getSnapshot().seekSequence));
    assert.ok(await asked.settle(() => asked.pipeline.view(failedId)?.mode === 'picture'));
  } finally { asked.close(); }

  let clockAttempts = 0;
  const clockFail = inProcess({
    followClock: () => {
      clockAttempts++;
      if (clockAttempts === 1) throw new ShownError('Host stream clock stalled.', 'clock rpc 504');
      return { available: true };
    },
  });
  try {
    const id = clockFail.pipeline.acquire('clock-fail|0', { offset: 0, pin: null });
    const sub = clockFail.pipeline.subscribe(id, {}, clockFail.source, '1', 0, { product: 'series-state' }, 'followed')!;
    const entry = installedEntry(clockFail.pipeline, id, sub);
    clockFail.clock.seek(10);
    assert.ok(await clockFail.settle(() => clockFail.pipeline.view(id)?.mode === 'failed'));
    assert.equal(entry.status, 'failed');
    const shownErr = followFailure({ status: entry.status, reason: entry.reason, error: entry.error, structure: '', entry, hidden: false, reading: null });
    assert.ok(shownErr);
    assert.deepEqual(classifyFailure(shownErr.error), { kind: 'retryable', reason: 'Host stream clock stalled.', detail: 'clock rpc 504' });
    shownErr.retry();
    assert.ok(await clockFail.settle(() => clockAttempts === 2 && clockFail.pipeline.view(id)?.mode === 'reading'));
    clockFail.send(message(10, [{ id: sub, answer: { keys: [1n], columns: 1, values: [11n] } }], false, NATIVE, clockFail.clock.getSnapshot().seekSequence));
    assert.ok(await clockFail.settle(() => clockFail.pipeline.view(id)?.mode === 'picture'));
  } finally { clockFail.close(); }

  let openCalls = 0;
  const streamDrop = inProcess({ open: async () => `session-${++openCalls}` });
  try {
    const id = streamDrop.pipeline.acquire('stream-drop|0', { offset: 0, pin: null });
    const sub = streamDrop.pipeline.subscribe(id, {}, streamDrop.source, '1', 0, { product: 'series-state' }, 'followed')!;
    const entry = installedEntry(streamDrop.pipeline, id, sub);
    streamDrop.clock.seek(10);
    assert.ok(await streamDrop.settle(() => streamDrop.pipeline.status(sub).status === 'followed'));
    streamDrop.send(message(10, [{ id: sub, answer: { keys: [1n], columns: 1, values: [2n] } }], false, NATIVE, streamDrop.clock.getSnapshot().seekSequence));
    assert.ok(await streamDrop.settle(() => streamDrop.pipeline.view(id)?.mode === 'picture'));
    streamDrop.fail('The pushed answers stopped');
    assert.ok(await streamDrop.settle(() => streamDrop.pipeline.view(id)?.mode === 'failed'));
    assert.equal(entry.status, 'failed');
    entry.retry!();
    assert.ok(await streamDrop.settle(() => openCalls === 2 && streamDrop.pipeline.status(sub).status === 'followed'));
    streamDrop.send(message(10, [{ id: sub, answer: { keys: [1n], columns: 1, values: [3n] } }], true, NATIVE, streamDrop.clock.getSnapshot().seekSequence));
    assert.ok(await streamDrop.settle(() => streamDrop.pipeline.view(id)?.mode === 'picture'));
  } finally { streamDrop.close(); }
});
