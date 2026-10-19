import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { createServer } from 'vite';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import type { PreparationReply, PreparationRequest } from '../../shell/preparation/contract';
import type { ScoreboardPreparedParams, ScoreboardPreparedReply } from '../../generated/dashboard';
import { preparedRefreshMs } from '../../dashboard/preparedSeries.ts';

test('the actual scoreboard hook bounds 5000-country requests and retains cell errors across index publications', { timeout: 15_000 }, async () => {
  const states: unknown[] = [];
  const dependencies: (readonly unknown[])[] = [];
  const cleanups: (() => void)[] = [];
  const effects = new Map<number, () => void | (() => void)>();
  let cursor = 0, effectCursor = 0, clock = 0;
  const harness = {
    now: () => clock,
    useState(initial: unknown) {
      const index = cursor++;
      if (!(index in states)) states[index] = typeof initial === 'function' ? initial() : initial;
      return [states[index], (next: unknown) => { states[index] = typeof next === 'function' ? next(states[index]) : next; }];
    },
    useEffect(callback: () => void | (() => void), deps: readonly unknown[]) {
      const index = effectCursor++;
      if (!dependencies[index] || deps.some((value, slot) => value !== dependencies[index][slot])) {
        effects.set(index, callback); dependencies[index] = deps;
      }
    },
  };
  const globals = globalThis as typeof globalThis & { __scoreboardHookFixture?: typeof harness };
  globals.__scoreboardHookFixture = harness;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'scoreboard-hook-react', enforce: 'pre',
      async load(id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/dashboard/contentRevision.ts')) return 'export const useContentRevision = () => 0;';
        if (!path.endsWith('/widgets/scoreboard/preparedScoreboard.ts')) return;
        return (await readFile(id, 'utf8'))
          .replace("import { useEffect, useState } from 'react';", 'const { useEffect, useState } = globalThis.__scoreboardHookFixture;')
          .replace("import { preparedSeriesQuery } from '../../dashboard/preparedSeries.ts';", `import { preparedSeriesQuery as query } from '../../dashboard/preparedSeries.ts';
const preparedSeriesQuery = (...args) => query(...args, globalThis.__scoreboardHookFixture.now);`);
      } }] });
  try {
    const { usePreparedScoreboard, preparedScoreboardRows } = await vite.ssrLoadModule('/ui/src/widgets/scoreboard/preparedScoreboard.ts');
    const source = { id: 'one', revision: '1' } as DashboardSource;
    const request: ScoreboardPreparedParams = { selection: { branchId: '1', tags: [], top: { count: 5000, sort: 'factories', descending: true }, columns: ['factories', 'nukes'] },
      hour: 10, sort: 'factories', descending: true, index: true, bars: true };
    const page = (request: ScoreboardPreparedParams): ScoreboardPreparedReply => {
      const rows = Array.from({ length: 5000 }, (_, slot) => ({ slot, tag: `country-${slot}` }));
      return { hour: request.hour, maxima: request.bars ? { factories: '5000', nukes: '9007199254740993' } : {},
        data: { branchId: '1', defaultColumns: ['factories'], defaultSort: 'factories', warnings: [],
          columns: ['factories', 'nukes'].map(key => ({ key: key as 'factories' | 'nukes', label: key, unit: 'count', decimals: 0, available: true, reason: null })),
          series: rows.filter(row => request.index || request.selection.tags.includes(row.tag)).map(({ slot, tag }) => ({ tag,
            segments: [{ firstHour: request.hour, lastHour: request.hour }], identities: [{ firstHour: request.hour, lastHour: request.hour, slot, countryId: slot + 1 }], membership: null,
            columns: (request.index ? ['factories'] as const : ['factories', 'nukes'] as const).map(key => ({ key,
              runs: [{ firstHour: request.hour, lastHour: request.hour, amount: key === 'factories' ? String(slot + 1) : '9007199254740993', startsSegment: false }],
              readRuns: request.index ? [] : [{ firstHour: request.hour, lastHour: request.hour, readHour: request.hour, hourly: true }] })) })) } };
    };
    let finish!: (value: ScoreboardPreparedReply) => void;
    const queries: ScoreboardPreparedParams[] = [];
    const client = { preparedQuery: async (owner: DashboardSource, _product: string, request: ScoreboardPreparedParams) => {
      queries.push(request);
      assert.ok(queries.length <= 16, 'the real hook must not keep requesting 5000-country replies');
      if (owner.id === 'one' && request.index && request.hour > 10) return await new Promise<ScoreboardPreparedReply>(resolve => { finish = resolve; });
      if (owner.id === 'incomplete' && !request.index) {
        const incomplete = page(request);
        incomplete.data.series = incomplete.data.series.slice(0, -1);
        return incomplete;
      }
      return page(request);
    }, product: () => { throw new Error('whole-branch scoreboard request'); } } as unknown as DashboardClient;
    let visible: ReturnType<typeof preparedScoreboardRows> = [];
    let lastRequest = request, lastSource = source, lastClient = client;
    const render = (next = request, current = source, transport = client) => {
      lastRequest = next; lastSource = current; lastClient = transport;
      cursor = effectCursor = 0;
      const result = usePreparedScoreboard(transport, current, next, visible, true);
      for (const [index, callback] of effects) { cleanups[index]?.(); cleanups[index] = callback() || (() => {}); }
      effects.clear(); return result;
    };
    const settle = async () => {
      await new Promise(resolve => setTimeout(resolve, 0));
      render(lastRequest, lastSource, lastClient);
      await new Promise(resolve => setTimeout(resolve, 0));
    };
    render(); await settle();
    let state = render();
    assert.equal(state.index.data.series.length, 5000);
    assert.equal(state.page.data.series.length, 1);
    assert.equal(queries.length, 2, 'one index and one initial cell request');
    visible = preparedScoreboardRows(state.index, state.page, 'factories', true).slice(200, 212);
    render(); await settle(); state = render();
    assert.equal(state.page.data.series.length, 12);
    assert.equal(queries.length, 3, 'scrolling requests one mounted page, not every country');
    for (let count = 0; count < 20; count++) { render(); await settle(); }
    assert.equal(queries.length, 3, 'settled 5000-country renders must not issue more requests');
    assert.ok(queries.every(query => query.index || query.selection.tags.length <= 12));
    const next = { ...request, hour: 11 };
    assert.equal(render(next).index.hour, 10);
    assert.equal(render(next).pending, true);
    finish(page(next)); await settle(); state = render(next);
    assert.equal(state.index.hour, 11); assert.equal(state.page.hour, 11);
    const departed = { ...request, hour: 12 };
    render(departed); const resolveDeparted = finish;
    const other = { id: 'other', revision: '2' } as DashboardSource;
    assert.equal(render(departed, other).index, undefined);
    await settle(); const owned = render(departed, other).index;
    resolveDeparted(page(departed)); await settle();
    assert.equal(render(departed, other).index, owned);
    const incomplete = { id: 'incomplete', revision: '3' } as DashboardSource;
    render(departed, incomplete); await settle(); state = render(departed, incomplete);
    assert.match(state.error, /did not supply/);
    assert.equal(state.pending, false);
    const stopped = queries.length;
    for (let count = 0; count < 20; count++) { render(departed, incomplete); await settle(); }
    assert.equal(queries.length, stopped, 'an incomplete successful reply must stop until an explicit retry');
    state.retry(); render(departed, incomplete); await settle(); state = render(departed, incomplete);
    assert.match(state.error, /did not supply/);
    assert.equal(queries.length, stopped + 2, 'an explicit retry issues one index and one cell request');

    let through = -1, cellReads = 0, refreshes = 0;
    let notifyStatus = () => {};
    const statuses: { request: PreparationRequest; resolve: (reply: PreparationReply) => void }[] = [];
    const preparingClient = {
      preparedQuery: async (_source: DashboardSource, _product: string, demand: ScoreboardPreparedParams) => {
        if (!demand.index) { cellReads++; throw new Error('Cells failed'); }
        return { ...page(demand), coverage: { firstHour: 0, lastHour: through, complete: false },
          pending: [{ firstHour: through + 1, lastHour: 99 }] };
      },
      preparationStatus: async (request: PreparationRequest) => new Promise<PreparationReply>(resolve => {
        statuses.push({ request, resolve }); notifyStatus();
      }),
    } as unknown as DashboardClient;
    const preparing = { id: 'preparing', revision: '1' } as DashboardSource;
    visible = [];
    const draw = () => render(request, preparing, preparingClient);
    draw(); await settle(); state = draw();
    assert.match(state.error, /Cells failed/);
    assert.equal(cellReads, 1);
    const advance = async (lastHour: number) => {
      if (!statuses.length) await new Promise<void>(resolve => { notifyStatus = resolve; });
      const next = statuses.shift()!;
      through = lastHour;
      // Advance the query's existing clock seam to the next permitted publication.
      clock += preparedRefreshMs(refreshes++);
      next.resolve({ ...next.request, state: 'preparing', sections: [], backgroundEnabled: true, bytes: '0', error: null,
        coverage: { '1': { firstHour: 0, lastHour, complete: false } } });
      await settle(); state = draw();
      assert.equal(state.index.coverage.lastHour, lastHour);
    };
    for (const lastHour of [3, 6, 9]) {
      await advance(lastHour);
      assert.match(state.error, /Cells failed/, 'index publications retain the failed cell request');
      assert.equal(cellReads, 1, 'preparation progress cannot retry failed cells');
    }
    state.retry(); draw(); await settle(); state = draw();
    assert.equal(cellReads, 2, 'Retry permits one new cell request');
    assert.match(state.error, /Cells failed/);
    visible = preparedScoreboardRows(state.index, undefined, 'factories', true).slice(10, 12);
    draw(); await settle(); state = draw();
    assert.equal(cellReads, 3, 'different wanted cells start a new request');
    assert.match(state.error, /Cells failed/);
    visible = [...visible].reverse();
    draw(); await settle(); state = draw();
    assert.equal(cellReads, 3, 'reordering the same wanted set does not retry');
    const changedHour = { ...request, hour: 11 };
    render(changedHour, preparing, preparingClient); await settle(); state = render(changedHour, preparing, preparingClient);
    assert.equal(cellReads, 4, 'a different hour starts a new cell request');
    assert.match(state.error, /Cells failed/);
    const changedSource = { ...preparing, revision: '2' };
    render(changedHour, changedSource, preparingClient); await settle(); state = render(changedHour, changedSource, preparingClient);
    assert.equal(cellReads, 5, 'a different source revision starts a new cell request');
    assert.match(state.error, /Cells failed/);
  } finally {
    for (const cleanup of cleanups) cleanup?.();
    await vite.close(); delete globals.__scoreboardHookFixture;
  }
});
