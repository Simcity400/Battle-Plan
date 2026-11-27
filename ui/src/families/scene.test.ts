import assert from 'node:assert/strict';
import test from 'node:test';
import type { Scene } from '../draw/lane.ts';
import { artHandleKey } from './art/types.ts';
import { FutureDataError } from './boundedView.ts';
import { projectScene, sceneReadView } from './scene.ts';
import { validateGenericTailReply, validateRowTailReply } from './table/rowSource.ts';

const part = (version: number, value: unknown) => ({ h: 100, c: 130, value, refused: null, cells: [], seeds: new Map(), channel: 'panel', version, bytes: 0 });

test('the scene reader serves each source by name, and only the call that ran the projection returns its identity and art demands', () => {
  const scene = {
    widget: 1,
    subs: [{ id: 7, channel: 'panel', route: null }, { id: 8, channel: 'panel', route: null }],
    pictures: [
      { h: 100, c: 130, parts: [part(3, { kind: 'panel', bytes: null, data: { partner: 'BUL' }, refused: null })] },
      { h: 100, c: 130, parts: [part(5, { kind: 'panel', bytes: null, data: null, refused: 'Not recorded in this campaign' })] },
    ],
    model: null,
  } as unknown as Scene;
  const sources = { sources: { main: 7, other: 8 }, staticPanels: { pinned: { level: 2 } } };
  const pending = { status: 'pending' } as const;
  const identity = {
    demandOwner: 'view' as const,
    resolve: (kind: 'country', key: string, atHour: number) => ({
      kind, key, atHour, holder: pending, tag: pending, name: pending, colour: pending, flagChoice: pending,
      validInterval: [atHour, atHour] as const, nextChangeAt: Infinity,
    }),
  };

  const plain = sceneReadView(scene, sources, 120, { identity: identity as never });
  assert.deepEqual(plain.revisions, { main: 3, other: 5 });
  assert.deepEqual(plain.panel('main'), { partner: 'BUL' });
  assert.deepEqual(plain.panel('pinned'), { level: 2 });
  assert.equal(plain.refused('other'), 'Not recorded in this campaign');
  assert.equal(plain.live('other'), undefined);

  const projected = projectScene(scene, sources, 120, { identity: identity as never }, view => {
    const reply = view.panel<{ partner: string }>('main')!;
    view.art({ kind: 'url', url: 'asset://flag.png' });
    view.art({ kind: 'sprite', key: 'GFX_unresolved', frame: 0 } as never);
    return view.who('country', reply.partner, 120).name.status;
  });
  assert.equal(projected.model, 'pending');
  assert.deepEqual(projected.identityDemands, [['country', 'BUL', 120]]);
  assert.deepEqual(projected.artUrls, ['asset://flag.png']);
  assert.deepEqual(projected.artAsks.map(handle => handle.kind), ['sprite']);
});

test('a projection’s identity and art reads are compared, not the snapshot: a change only to keys it never read leaves them unchanged, at one lookup per read', () => {
  const scene = { widget: 1, subs: [], pictures: [], model: null } as unknown as Scene;
  const ready = (value: string) => ({ status: 'ready' as const, value });
  let lookups = 0;
  const identityOf = (names: Record<string, string>) => ({
    resolve: (kind: 'country', key: string, atHour: number) => {
      lookups++;
      return { kind, key, atHour, holder: ready(key), name: ready(names[key] ?? key), validInterval: [atHour, atHour + 5] as const, nextChangeAt: atHour + 5 };
    },
  });
  const artOf = (status: 'pending' | 'noArtExpected') => ({ resolve: () => ({ status }) });
  const projected = projectScene(scene, { sources: {} }, 120, { identity: identityOf({ GER: 'Germany' }) as never, art: artOf('noArtExpected') as never }, view => {
    view.art({ kind: 'url', url: 'asset://flag.png' });
    return view.who('country', 'GER', 120).name.status;
  });
  assert.equal(projected.boundary, 125, 'a name read at the displayed hour bounds the model at the end of its run');
  assert.deepEqual([projected.reads.identity, projected.reads.art], [1, 1]);
  lookups = 0;
  assert.equal(projected.reads.unchanged({ identity: identityOf({ GER: 'Germany', ITA: 'Italy' }) as never, art: artOf('noArtExpected') as never }), true);
  assert.equal(lookups, 1, 'one lookup per read key, whatever the snapshot holds');
  assert.equal(projected.reads.unchanged({ identity: identityOf({ GER: 'German Reich' }) as never, art: artOf('noArtExpected') as never }), false);
  assert.equal(projected.reads.unchanged({ identity: identityOf({ GER: 'Germany' }) as never, art: artOf('pending') as never }), false);
});

test('a read compares the whole observable result: the interval, a failure’s retryability, a ready frame, and the URL the read would demand', () => {
  const scene = { widget: 1, subs: [], pictures: [], model: null } as unknown as Scene;
  const identityFrom = (from: number, name: { status: 'ready'; value: string } | { status: 'failed'; reason: string; retryable: boolean }) => ({
    resolve: (kind: 'country', key: string, atHour: number) => ({ kind, key, atHour, holder: { status: 'ready', value: key }, name, validInterval: [from, 200] as const, nextChangeAt: 200 }),
  });
  const named = { status: 'ready' as const, value: 'Germany' };
  const read = projectScene(scene, { sources: {} }, 120, { identity: identityFrom(90, named) as never }, view => view.who('country', 'GER', 100).name.status);
  assert.equal(read.reads.unchanged({ identity: identityFrom(90, named) as never }), true);
  assert.equal(read.reads.unchanged({ identity: identityFrom(110, named) as never }), false, 'a corrected run that now starts after the historical reading is a change, though name and end agree');
  const failing = (retryable: boolean) => identityFrom(90, { status: 'failed', reason: 'Lookup failed', retryable });
  const failed = projectScene(scene, { sources: {} }, 120, { identity: failing(true) as never }, view => view.who('country', 'GER', 100).name.status);
  assert.equal(failed.reads.unchanged({ identity: failing(false) as never }), false, 'a failure that can no longer be retried is a change');

  const sprite = { kind: 'sprite', key: 'GFX_tech', frame: 0 } as never;
  const pending = { resolve: () => ({ status: 'pending' }) };
  const unresolved = projectScene(scene, { sources: {} }, 120, { art: pending as never }, view => view.art(sprite).status);
  assert.deepEqual([unresolved.artUrls, unresolved.artAsks.length], [[], 1]);
  const resolution = { status: 'ready', assetId: 'a', url: 'asset://tech.png', origin: 'game', file: 'tech.dds', frame: { index: 0, count: 1, width: 64, height: 64 }, provenance: 'current' };
  const resolutions = { [artHandleKey(sprite)]: resolution };
  assert.equal(unresolved.reads.unchanged({ art: pending as never, artResolutions: resolutions as never }), false,
    'resolved but not yet decoded is still pending, yet the read now demands a URL, so the model is rebuilt to ask for it');
  const readyOf = (width: number) => ({ resolve: () => ({ ...resolution, frame: { index: 0, count: 1, width, height: 64 } }) });
  const ready = projectScene(scene, { sources: {} }, 120, { art: readyOf(64) as never }, view => view.art(sprite).status);
  assert.equal(ready.reads.unchanged({ art: readyOf(64) as never }), true);
  assert.equal(ready.reads.unchanged({ art: readyOf(48) as never }), false, 'another frame size is a change');
});

test('a followed panel is judged by the hour its record proves from, not the later hour its reply was read at', () => {
  const recordAt = (h: number, data: unknown) => ({
    widget: 1,
    subs: [{ id: 7, channel: 'panel', route: null }],
    pictures: [{ h, c: 105, parts: [{ ...part(2, { kind: 'panel', bytes: null, data, refused: null }), h, c: 105 }] }],
    model: null,
  } as unknown as Scene);
  const readAhead = { hour: 105, from: 96, until: 140, rows: [{ country: 'GER' }] };
  assert.deepEqual(sceneReadView(recordAt(100, readAhead), { sources: { main: 7 } }, 102).panel('main'), readAhead,
    'a reply read at 105 whose record proves it from 100 is shown at 102');
  assert.throws(() => sceneReadView(recordAt(103, readAhead), { sources: { main: 7 } }, 102).panel('main'), FutureDataError,
    'a record that proves its value only from 103 is refused at 102');
  const unrecorded = { widget: 1, subs: [], pictures: [], model: null } as unknown as Scene;
  assert.throws(() => sceneReadView(unrecorded, { sources: {}, staticPanels: { main: readAhead } }, 102).panel('main'), FutureDataError,
    'a panel with no record is judged by the hour it was read at');
});

test('a log frozen on an answered snapshot reads that snapshot, not the live head’s refusal', () => {
  const scene = {
    widget: 1,
    subs: [{ id: 7, channel: 'panel', route: null }],
    pictures: [{ h: 160, c: 170, parts: [part(2, { kind: 'panel', bytes: null, data: null, refused: 'Not recorded at this hour' })] }],
    model: null,
  } as unknown as Scene;
  const raw = { total: '300', offset: '256', rows: [{ key: 'old', hour: 100 }], missing: [], counts: [] };
  const frozen = { 256: { hour: 100, offset: 256, limit: 256, raw, reply: validateGenericTailReply(raw, 100) } };
  const view = sceneReadView(scene, { sources: { main: 7 }, pagedRows: { main: frozen } }, 165);
  assert.equal(view.refused('main'), undefined);
  assert.deepEqual([view.rows('main')?.at, view.rows('main')?.total, view.rows('main')?.refused], [100, 300, null]);
  const live = sceneReadView(scene, { sources: { main: 7 } }, 165);
  assert.equal(live.refused('main'), 'Not recorded at this hour', 'at the live head the refusal stands');
});

test('S3: sceneReadView applies validateTail to live and static log heads so malformed product rows surface as refused slots', () => {
  const validateTail = (raw: unknown, readAt?: number) => validateRowTailReply(raw, row => {
    if (row.ok !== true) throw new Error('Row payload is malformed.');
    return { key: row.key as string, hour: row.hour as number };
  }, readAt);
  const head = { total: '2', offset: '0', rows: [{ key: 'a', hour: 100, ok: true }, { key: 'b', hour: 99, ok: false }], missing: [], counts: [] };
  const liveScene = {
    widget: 1,
    subs: [{ id: 7, channel: 'panel', route: null }],
    pictures: [{ h: 100, c: 120, parts: [part(2, { kind: 'panel', bytes: null, data: head, refused: null })] }],
    model: null,
  } as unknown as Scene;
  const liveView = sceneReadView(liveScene, { sources: { main: 7 }, validateTail }, 110);
  assert.deepEqual(liveView.rows('main')?.sliceAt(110, 0, 2), [
    { kind: 'data', ordinal: 0, readAt: 110, row: { key: 'a', hour: 100 } },
    { kind: 'refused', ordinal: 1, readAt: 110, hour: 99, reason: 'Row payload is malformed.' },
  ]);
  const staticView = sceneReadView({ widget: 1, subs: [], pictures: [], model: null } as unknown as Scene, { sources: {}, staticPanels: { main: head }, validateTail }, 110);
  assert.deepEqual(staticView.rows('main')?.sliceAt(110, 0, 2), [
    { kind: 'data', ordinal: 0, readAt: 110, row: { key: 'a', hour: 100 } },
    { kind: 'refused', ordinal: 1, readAt: 110, hour: 99, reason: 'Row payload is malformed.' },
  ]);
});
