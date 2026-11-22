import assert from 'node:assert/strict';
import { test } from 'node:test';
import { inProcess, NATIVE } from './inProcess.fixture.ts';
import { message } from './messages.fixture.ts';
import type { PageSurface } from './page.ts';
import type { LaneView } from '../../draw/lane';

const surface = (): PageSurface<ImageBitmap> & { cleared: number } => ({ cleared: 0, install() {}, clear() { this.cleared++; } });

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
