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

test('astra 13: two copies sharing a subscription scroll their own windows while paused; native gets the union, both install its version', async () => {
  const { pipeline, clock, send, source, asked, settle, close } = inProcess();
  try {
    const query = { product: 'series-state', params: { branchId: '1', reads: 'every' } };
    const one = pipeline.acquire('one|0', { offset: 0, pin: null }), two = pipeline.acquire('two|0', { offset: 0, pin: null });
    const sub = pipeline.subscribe(one, {}, source, '1', 0, query, 'followed')!;
    assert.equal(pipeline.subscribe(two, {}, source, '1', 0, query, 'followed'), sub, 'the copies share one subscription');
    clock.seek(10);
    await settle(() => pipeline.status(sub).status === 'followed');
    const windows = () => asked.filter(ask => ask.product === 'follow-window').map(ask => ask.params as { countries: string[]; version: number });
    pipeline.window(one, sub, { from: 0, count: 0, countries: ['GER'] });
    pipeline.window(two, sub, { from: 0, count: 0, countries: ['ITA'] });
    const told = windows().at(-1)!;
    assert.deepEqual(told.countries, ['GER', 'ITA'], 'native answers the union of what both copies show');
    const seek = clock.getSnapshot().seekSequence;
    send(message(10, [{ id: sub, answer: { keys: [1n, 2n], columns: 1, values: [5n, 6n], names: ['GER', 'ITA'], window: told.version - 1 } }], false, NATIVE, seek));
    send(message(11, [{ id: sub, hour: 10, answer: { keys: [1n, 2n], columns: 1, values: [5n, 6n], names: ['GER', 'ITA'], window: told.version } }], false, NATIVE, seek));
    assert.ok(await settle(() => [one, two].every(id => pipeline.view(id)?.mode === 'picture')), 'both copies install the answer of the current window, never the older one');
    pipeline.window(two, sub, { from: 0, count: 0, countries: ['FRA'] });
    assert.deepEqual(windows().at(-1)!.countries, ['FRA', 'GER'], 'scrolling one copy moves the union while paused');
    assert.ok(windows().at(-1)!.version > told.version);
  } finally { close(); }
});
