import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import { gameScoreModel, gameScoreRace, gameScoreRows, SCORE_PARTS, FALLBACK_LABELS } from './gameScoreData.ts';
import type { GameScoreReply, GameScoreSeries } from '../../generated/dashboard';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';

const series = (): GameScoreSeries => Object.fromEntries(SCORE_PARTS.map(part => [part,
  [{ firstHour: 104, lastHour: 104, raw: part === 'total' ? '9007199254740993' : '-100001', readEachHour: true }]])) as GameScoreSeries;
const reply = (count: number): GameScoreReply => ({ branchId: '1', scale: 100000, reads: [104], reasons: [], warnings: [], scoreWarnings: [],
  labels: { ...FALLBACK_LABELS, faction: 'Faction' }, hints: {}, multipliers: series(), countries: Array.from({ length: count }, (_, index) => ({
    tag: `tail only ${index}`, identity: [{ firstHour: 104, lastHour: 104, value: { canonicalSlot: index, nativeCountry: index + 10000,
      name: '', nameMissing: null, color: [17, 29, 0], colorMissing: null, faction: null, factionMissing: null } }], scores: series() })) });

// Execute the real hook and shared preparation loop; only React scheduling and the transport are controlled.
test('score draws 5000 tail countries immediately and refreshes once accepted coverage advances', async () => {
  const states: unknown[] = [];
  let cursor = 0;
  let effect: (() => void | (() => void)) | undefined;
  let cleanup: (() => void) | undefined;
  let deps: readonly unknown[] | undefined;
  const harness = {
    useState(initial: unknown) {
      const index = cursor++;
      if (!(index in states)) states[index] = typeof initial === 'function' ? initial() : initial;
      return [states[index], (next: unknown) => { states[index] = typeof next === 'function' ? next(states[index]) : next; }];
    },
    useEffect(callback: () => void | (() => void), next: readonly unknown[]) {
      if (!deps || next.some((value, index) => value !== deps![index])) { effect = callback; deps = next; }
    },
  };
  const globals = globalThis as typeof globalThis & { __scoreHookFixture?: typeof harness };
  globals.__scoreHookFixture = harness;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'score-hook-react', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/dashboard/contentRevision.ts')) return 'export const useContentRevision = () => 0;';
        if (path.endsWith('/widgets/game-score/usePreparedScore.ts')) return source.replace("import { useEffect, useState } from 'react';",
          'const {useEffect,useState} = globalThis.__scoreHookFixture;');
      } }] });
  try {
    const { usePreparedScore, validatePreparedScore } = await vite.ssrLoadModule('/ui/src/widgets/game-score/usePreparedScore.ts') as typeof import('./usePreparedScore');
    const track = { source: { id: 'score', revision: '1' }, branch: { id: '1' } } as BoundTrack;
    const data = reply(5000);
    const partial = { reply: data, stretches: {}, approximate: true, coverage: { firstHour: 100, lastHour: 100, complete: false } };
    const exact = { ...partial, approximate: false, coverage: { firstHour: 100, lastHour: 104, complete: true },
      stretches: Object.fromEntries(data.countries.map(country => [country.tag, `prepared ${country.tag}`])) };
    let queries = 0, statuses = 0;
    let refreshed!: () => void;
    const completion = new Promise<void>(resolve => { refreshed = resolve; });
    const client = {
      preparedQuery: async (_source: unknown, product: string, params: unknown) => {
        assert.ok(++queries <= 2, 'coverage advancement permits only one refresh');
        assert.equal(product, 'series-state');
        assert.deepEqual(params, { branchId: '1', family: { kind: 'game-score' }, reads: [{ hour: 104 }] });
        if (queries === 2) refreshed();
        return queries === 1 ? partial : exact;
      },
      preparationStatus: async (request: object) => {
        assert.ok(++statuses <= 2, 'an unchanged prefix does not reread scores');
        return { ...request, state: statuses === 1 ? 'preparing' : 'ready', sections: [], bytes: '0', error: null,
          backgroundEnabled: true, coverage: { '1': statuses === 1 ? partial.coverage : exact.coverage } };
      },
    } as unknown as DashboardClient;
    const render = (currentTrack = track, currentClient = client) => {
      cursor = 0;
      const result = usePreparedScore({ client: currentClient }, currentTrack, 104, true);
      if (effect) { cleanup?.(); const run = effect; effect = undefined; cleanup = run() || undefined; }
      return result;
    };
    render();
    await new Promise(resolve => setTimeout(resolve, 0));
    const immediate = render();
    assert.equal(immediate.data?.approximate, true);
    assert.equal(immediate.pending, false);
    const model = gameScoreModel(immediate.data!.reply);
    const rows = gameScoreRows(model, model.tags, immediate.hour, { column: 'total', direction: 'descending' });
    assert.equal(rows.length, 5000);
    assert.equal(gameScoreRace(rows, false).length, 5000);
    assert.equal(rows[0].cells.total.raw, '9007199254740993');
    assert.deepEqual(immediate.data!.stretches, {}, 'a tail frame proves no earlier continuity');
    assert.equal(queries, 1);
    await completion;
    await new Promise(resolve => setTimeout(resolve, 0));
    assert.equal(render().data?.approximate, false);
    assert.equal(Object.keys(render().data!.stretches).length, 5000);
    assert.deepEqual({ queries, statuses }, { queries: 2, statuses: 2 });

    // An idle preparation job cannot erase the values already drawn for this hour.
    let idleQueries = 0, idleStatuses = 0;
    const idleClient = { preparedQuery: async () => { assert.ok(++idleQueries <= 1); return { ...partial, coverage: { firstHour: 0, lastHour: -1, complete: false } }; },
      preparationStatus: async (request: object) => { assert.ok(++idleStatuses <= 1); return { ...request, state: 'idle', sections: [], bytes: '0',
        error: null, backgroundEnabled: false, coverage: {} }; } } as unknown as DashboardClient;
    render(track, idleClient);
    await new Promise(resolve => setTimeout(resolve, 0));
    const idle = render(track, idleClient);
    assert.equal(idle.data?.reply.countries.length, 5000);
    assert.equal(idle.data?.approximate, true);
    assert.match(idle.error!, /Background preparation is off/);
    assert.deepEqual({ idleQueries, idleStatuses }, { idleQueries: 1, idleStatuses: 1 });
    assert.throws(() => validatePreparedScore({ ...partial, approximate: 'true' }), /malformed/);
    const cold = { ...partial, coverage: { ...partial.coverage, lastHour: 99 } };
    assert.deepEqual(validatePreparedScore(cold), cold);
    for (const coverage of [null, { ...cold.coverage, lastHour: 98 }, { ...cold.coverage, complete: true }])
      assert.throws(() => validatePreparedScore({ ...partial, coverage }), /malformed/);

    const changed = { ...track, source: { ...track.source, revision: '2' } };
    let changedQueries = 0;
    const pendingClient = { preparedQuery: async () => { assert.ok(++changedQueries <= 1); return new Promise(() => {}); } } as unknown as DashboardClient;
    assert.equal(render(changed, pendingClient).data, undefined, 'a new source revision cannot inherit old rows');
    assert.equal(changedQueries, 1);
  } finally { cleanup?.(); await vite.close(); delete globals.__scoreHookFixture; }
});
