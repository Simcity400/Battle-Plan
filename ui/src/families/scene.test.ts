import assert from 'node:assert/strict';
import test from 'node:test';
import type { Scene } from '../draw/lane.ts';
import { projectScene, sceneReadView } from './scene.ts';

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
