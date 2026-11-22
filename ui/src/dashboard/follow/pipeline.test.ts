import assert from 'node:assert/strict';
import { test } from 'node:test';
import { inProcess, NATIVE } from './inProcess.fixture.ts';
import { message } from './messages.fixture.ts';
import type { PageSurface } from './page.ts';
import type { LaneView, Painted } from '../../draw/lane';
import { parseAnswer, rowName } from './message.ts';

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

test('review 4 finding 2: each windowed copy owns its subscription: scrolling one never rebinds or delays the other, and clearing a window asks for every row', async () => {
  const { pipeline, clock, send, source, asked, settle, close } = inProcess();
  try {
    const query = { product: 'series-state', params: { branchId: '1', reads: 'every' } };
    const one = pipeline.acquire('one|0', { offset: 0, pin: null }), two = pipeline.acquire('two|0', { offset: 0, pin: null });
    const first = pipeline.subscribe(one, {}, source, '1', 0, query, 'followed', undefined, { from: 0, count: 0, countries: ['GER'] })!;
    const second = pipeline.subscribe(two, {}, source, '1', 0, query, 'followed', undefined, { from: 0, count: 0, countries: ['ITA'] })!;
    assert.notEqual(first, second, 'a windowed copy is never shared');
    clock.seek(10);
    await settle(() => [first, second].every(sub => pipeline.status(sub).status === 'followed'));
    const seek = clock.getSnapshot().seekSequence;
    const follows = asked.filter(ask => ask.product === 'follow').map(ask => ask.params as { id: number; window: { countries: string[]; version: number } });
    assert.deepEqual(follows.map(ask => ask.window.countries), [['GER'], ['ITA']], 'each copy registers with its own first window');
    const answer = (sub: number, name: string, value: bigint, version: number, hour = 10) =>
      send(message(hour, [{ id: sub, answer: { keys: [1n], columns: 1, values: [value], names: [name], window: version } }], false, NATIVE, seek));
    answer(first, 'GER', 5n, follows[0]!.window.version); answer(second, 'ITA', 6n, follows[1]!.window.version);
    assert.ok(await settle(() => [one, two].every(id => pipeline.view(id)?.mode === 'picture')));
    const binding = pipeline.page.widgets.get(one)!.binding;
    pipeline.window(two, second, { from: 0, count: 0, countries: ['FRA'] });
    const moved = asked.filter(ask => ask.product === 'follow-window').at(-1)!.params as { id: number; countries: string[]; version: number };
    assert.deepEqual([moved.id, moved.countries], [second, ['FRA']], 'only the scrolled copy asks native');
    assert.deepEqual([pipeline.page.widgets.get(one)!.binding, pipeline.view(one)!.dim], [binding, false], 'the stationary copy is not rebound, dimmed or held');
    answer(first, 'GER', 7n, follows[0]!.window.version, 11);
    clock.step(1);
    assert.ok(await settle(() => pipeline.view(one)?.h === 11), 'the stationary copy keeps advancing');
    const value = (id: number) => { const body = pipeline.view(id)?.fields?.[`answer:${id === one ? first : second}`] as Uint8Array; return parseAnswer(body); };
    const shown = value(one);
    assert.ok(shown.kind === 'rows' && rowName(shown, 0) === 'GER' && shown.rows === 1, 'each copy shows only its own rows');
    pipeline.window(two, second, null);
    const cleared = asked.filter(ask => ask.product === 'follow-window').at(-1)!.params as { every?: boolean; version: number };
    assert.deepEqual([cleared.every, cleared.version > moved.version], [true, true], 'removing the last window asks native for every row under a new version');
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

test('review 4 finding 1: a widget whose incoming ranking names a country its columns lack asks for it and holds its previous picture meanwhile', async () => {
  const needing: LaneView = { paint: (input): Painted => {
    const ranking = input.pictures[0]!.parts[0]!.value, columns = input.subs[1]!;
    const names = ranking.kind === 'answer' && ranking.answer.kind === 'rows' ? Array.from({ length: ranking.answer.rows }, (_, row) => rowName(ranking.answer as never, row)!) : [];
    if (columns.countries && names.some(name => !columns.countries!.has(name))) return { surfaces: [], fields: {}, motion: false, pending: true, needs: [{ sub: columns.id, countries: names }] };
    return { surfaces: [], fields: { names: { value: names, content: names.join() } }, motion: false };
  } };
  const { pipeline, clock, send, source, asked, settle, close } = inProcess({ views: kind => kind === 'needing' ? needing : undefined });
  try {
    const id = pipeline.acquire('board|0', { offset: 0, pin: null, view: 'needing' });
    pipeline.present(id, 'needing');
    const ranking = pipeline.subscribe(id, {}, source, '1', 0, { product: 'series-state', params: { branchId: '1', top: 2 } }, 'followed')!;
    const columns = pipeline.subscribe(id, {}, source, '1', 0, { product: 'series-state', params: { branchId: '1', reads: 'every' } }, 'followed', undefined,
      { from: 0, count: 0, countries: [], most: 64 })!;
    clock.seek(10);
    await settle(() => [ranking, columns].every(sub => pipeline.status(sub).status === 'followed'));
    const seek = clock.getSnapshot().seekSequence;
    const windowOf = () => { const told = [...asked].reverse().find(ask => ask.params.id === columns && (ask.product === 'follow-window' || ask.product === 'follow'))!.params as { countries?: string[]; version?: number; window?: { countries: string[]; version: number } }; return told.window ?? told as { countries: string[]; version: number }; };
    const rank = (hour: number, names: string[]) => send(message(hour, [{ id: ranking, answer: { keys: names.map((_, at) => BigInt(at + 1)), columns: 1, values: names.map(() => 1n), names } }], false, NATIVE, seek));
    const column = (hour: number, names: string[]) => send(message(hour, [{ id: columns, answer: { keys: names.map((_, at) => BigInt(at + 1)), columns: 1, values: names.map(() => 1n), names, window: windowOf().version } }], false, NATIVE, seek));
    column(10, []);
    rank(10, ['GER', 'ITA']);
    assert.ok(await settle(() => windowOf().countries.join() === 'GER,ITA'), 'the incoming ranking asks for its countries');
    column(10, ['GER', 'ITA']);
    assert.ok(await settle(() => pipeline.view(id)?.mode === 'picture'));
    rank(11, ['FRA', 'GER']);
    clock.step(1);
    assert.ok(await settle(() => windowOf().countries.join() === 'FRA,GER,ITA'), 'FRA entering the top asks for FRA before the ranking installs');
    assert.deepEqual([pipeline.view(id)?.h, pipeline.view(id)?.fields?.names], [10, ['GER', 'ITA']], 'the previous complete picture holds meanwhile');
    column(11, ['FRA', 'GER', 'ITA']);
    assert.ok(await settle(() => pipeline.view(id)?.h === 11), 'and installs once the columns hold FRA');
    assert.deepEqual(pipeline.view(id)?.fields?.names, ['FRA', 'GER']);
  } finally { close(); }
});
