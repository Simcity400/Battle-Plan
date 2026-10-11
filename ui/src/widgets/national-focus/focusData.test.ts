import assert from 'node:assert/strict';
import test from 'node:test';
import { focusAt, focusCompletion, focusProgressAt, focusRequest, validateFocusReply, validateFocusArtworkReply } from './focusData.ts';
import { loadProduct } from '../../dashboard/products.ts';
import type { DashboardClient, ProductRequest } from '../../dashboard/client';
import type { FocusReply, FocusArtworkParams, FocusArtworkReply } from '../../generated/dashboard';
import type { WidgetProps } from '../../dashboard/definitions';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { HourStore, planRequests } from '../../dashboard/countryFacts.ts';

const request = focusRequest('source', 'v1', { branchId: '1', tag: 'GER' });
const first = { completed: [{ ordinal: 0, token: 'one', completedByHour: null }], current: 'two', continuous: null, paused: false };
const reply: FocusReply = { history: { segments: [
  { firstHour: 100, lastHour: 123, order: first },
  { firstHour: 125, lastHour: 200, order: { ...first, completed: [...first.completed, { ordinal: 1, token: 'two', completedByHour: 125 }], current: null } },
], progress: [
  { firstHour: 100, lastHour: 110, progressRaw: '6800000' },
  { firstHour: 111, lastHour: 123, progressRaw: '6900000' },
  { firstHour: 125, lastHour: 140, progressRaw: '100000' },
  { firstHour: 141, lastHour: 160, progressRaw: '200000' },
] }, artwork: [], warnings: [] };

test('focus histories reject overlap, future completion bounds and invalid artwork', () => {
  assert.equal(validateFocusReply(reply), reply);
  const invalid = (firstHour: number, lastHour: number, ordinal: number, completedByHour: number | null) => ({ ...reply, history: { progress: [], segments: [{ firstHour, lastHour, order: { ...first, completed: [{ ordinal, token: 'one', completedByHour }] } }] } });
  for (const bad of [invalid(100, 99, 0, null), invalid(100, 100, 3, null), invalid(100, 100, 0, 101), invalid(NaN, 100, 0, null)]) assert.throws(() => validateFocusReply(bad));
  assert.throws(() => validateFocusReply({ ...reply, history: { ...reply.history, segments: [...reply.history.segments, reply.history.segments[0]] } }));
  for (const progressRaw of ['1.5', '', '99999999999999999999']) assert.throws(() => validateFocusReply({ ...reply, history: { ...reply.history, progress: [{ firstHour: 1, lastHour: 2, progressRaw }] } }));
  assert.throws(() => validateFocusReply({ ...reply, history: { ...reply.history, progress: [...reply.history.progress, reply.history.progress[0]] } }));
  const art = { token: 'one', name: 'One', description: 'Game description', baseDurationDays: 70, icon: null };
  assert.equal(validateFocusReply({ ...reply, artwork: [art] }).artwork[0].baseDurationDays, 70);
  for (const baseDurationDays of [-1, NaN, Infinity]) assert.throws(() => validateFocusReply({ ...reply, artwork: [{ ...art, baseDurationDays }] }));
  // An icon that is not a bounded PNG data URL is never drawn: it shows as missing art,
  // and the rest of the reply (a mod's oversized icon included) still loads.
  for (const icon of ['https://example.com/remote.png', 'data:text/html;base64,YQ==', `data:image/png;base64,${'A'.repeat(200 * 1024)}`]) {
    assert.equal(validateFocusReply(structuredClone({ ...reply, artwork: [{ ...art, icon }] })).artwork[0].icon, null);
  }
  // A malformed entry is still refused rather than reaching the widget.
  for (const bad of [null, 'one', {}, { ...art, token: 7 }]) {
    assert.throws(() => validateFocusReply({ ...reply, artwork: [bad] } as unknown as FocusReply));
  }
});

test('a modded tree may complete any number of focuses', () => {
  const completed = Array.from({ length: 900 }, (_, ordinal) => ({ ordinal, token: `mod_focus_${ordinal}`, completedByHour: null }));
  const big = { ...reply, history: { progress: [], segments: [{ firstHour: 100, lastHour: 200, order: { ...first, completed } }] } };
  assert.equal(validateFocusReply(big).history.segments[0].order.completed.length, 900);
});

test('exact local playback preserves unknown dates and never bridges gaps or leaks future focuses', () => {
  assert.equal(focusAt(reply.history, 99), null);
  assert.equal(focusAt(reply.history, 100), first);
  assert.equal(focusAt(reply.history, 123)?.completed.length, 1);
  assert.equal(focusAt(reply.history, 124), null);
  assert.equal(focusAt(reply.history, 125)?.completed[1].completedByHour, 125);
  assert.equal(focusAt(reply.history, 201), null);
});

test('many focus widgets share one load while playback advances without requests', async () => {
  let calls = 0;
  const client = { product: async (input: ProductRequest) => { calls++; return { sourceId: input.sourceId, revision: input.revision, product: input.product, data: reply }; } } as unknown as DashboardClient;
  const results = await Promise.all(Array.from({ length: 24 }, () => loadProduct<FocusReply>(client, request, new AbortController().signal, validateFocusReply)));
  assert.equal(calls, 1);
  assert.ok(results.every(value => value === results[0]));
  for (let hour = 100; hour <= 200; hour++) for (const result of results) focusAt(result.history, hour);
  assert.equal(calls, 1);
  await loadProduct(client, focusRequest('source', 'v1', { branchId: '1', tag: 'ENG' }), new AbortController().signal, validateFocusReply);
  assert.equal(calls, 2);
});

test('focus progress reports exact days and days left only from a known daily rate', () => {
  // The second day of `two`: 69 of 70 days done, one day gained, one day left.
  const late = focusProgressAt(reply.history, 115)!;
  assert.deepEqual(late, { progress: 6_900_000n, dailyGain: 100_000n });
  assert.deepEqual(focusCompletion(late, 70), { fraction: 0.9857, elapsedDays: 69, totalDays: 70, daysLeft: 1 });
  // The first recorded day has no earlier day to measure a rate from.
  assert.equal(focusProgressAt(reply.history, 105)!.dailyGain, null);
  assert.equal(focusCompletion(focusProgressAt(reply.history, 105)!, 70)!.daysLeft, null);
  // Hour 124 is a gap: nothing is reported there, and the next run never measures across it.
  assert.equal(focusProgressAt(reply.history, 124), null);
  assert.equal(focusProgressAt(reply.history, 130)!.dailyGain, null);
  // No current focus after 125, so banked progress has no rate toward a focus.
  assert.equal(focusProgressAt(reply.history, 150)!.dailyGain, null);
  // A faster rate shortens the estimate; completion clamps to the base duration.
  assert.equal(focusCompletion({ progress: 3_500_000n, dailyGain: 125_000n }, 70)!.daysLeft, 28);
  assert.equal(focusCompletion({ progress: 9_000_000n, dailyGain: 100_000n }, 70)!.fraction, 1);
  assert.equal(focusCompletion({ progress: 0n, dailyGain: null }, 0), null);
});

test('dated focus artwork authenticates the read, retains empty text and refuses omitted tokens', () => {
  const params = { branchId: '1', hour: 10, tag: 'synthetic', tokens: ['focus'] };
  const art = { token: 'focus', name: '', description: '', baseDurationDays: 10, icon: null as string | null };
  const dated = { branchId: '1', hour: 10, tag: 'synthetic', items: [{ artwork: art, reason: 'focus-window mode is not recorded' as string | null }], warnings: ['Named content warning'] };
  assert.equal(validateFocusArtworkReply(dated, params).items[0].reason, 'focus-window mode is not recorded');
  assert.equal(dated.items[0].artwork.name, '');
  for (const bad of [{ ...dated, hour: 11 }, { ...dated, tag: 'other' }, { ...dated, branchId: '2' }, { ...dated, items: [] },
    { ...dated, items: [...dated.items, ...dated.items] }, { ...dated, warnings: [null] }]) {
    assert.throws(() => validateFocusArtworkReply(bad, params));
  }
  const unreadable = structuredClone(dated); unreadable.items[0].artwork.icon = 'https://invalid.example/icon';
  unreadable.items[0].reason = null;
  assert.equal(validateFocusArtworkReply(unreadable, params).items[0].artwork.icon, null);
  assert.equal(unreadable.items[0].reason, 'The returned icon could not be displayed.');
});

test('duration metadata and dated requests retain 5000 modded tokens without a chosen cap', () => {
  const artwork = Array.from({ length: 5000 }, (_, index) => ({ token: `synthetic_focus_${index}`, name: null, description: null, baseDurationDays: 10, icon: null }));
  assert.equal(validateFocusReply({ ...reply, artwork }).artwork.length, artwork.length);
  const params = { branchId: '1', hour: 10, tag: 'synthetic', tokens: artwork.map(art => art.token) };
  assert.equal(validateFocusArtworkReply({ branchId: params.branchId, hour: params.hour, tag: params.tag,
    items: artwork.map(art => ({ artwork: art, reason: null })), warnings: [] }, params).items.length, 5000);
});

type HookSlot = { value?: unknown; deps?: readonly unknown[]; cleanup?: () => void };
function focusHooks() {
  const slots: HookSlot[] = [], pending: (() => void)[] = [];
  let cursor = 0;
  return {
    begin() { cursor = 0; },
    state(initial: unknown) {
      const slot = slots[cursor++] ??= { value: initial };
      return [slot.value, (next: unknown) => { slot.value = typeof next === 'function' ? next(slot.value) : next; }];
    },
    ref(initial: unknown) { return (slots[cursor++] ??= { value: { current: initial } }).value; },
    effect(effect: () => unknown, deps: readonly unknown[]) {
      const slot = slots[cursor++] ??= {};
      if (slot.deps?.length === deps.length && deps.every((value, index) => Object.is(value, slot.deps?.[index]))) return;
      slot.deps = deps;
      pending.push(() => { slot.cleanup?.(); const cleanup = effect(); slot.cleanup = typeof cleanup === 'function' ? cleanup as () => void : undefined; });
    },
    flush() { for (const effect of pending.splice(0)) effect(); },
    dispose() { for (const slot of slots) slot.cleanup?.(); },
  };
}

test('registered focus renderer owns dated art, pending/refused hover text, Pin and client A-B-A reads', async () => {
  const host = globalThis as typeof globalThis & { __focusHooks?: ReturnType<typeof focusHooks> };
  const state = focusHooks(); host.__focusHooks = state;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'focus-renderer-state', enforce: 'pre',
      transform(code, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/national-focus/NationalFocusWidget.tsx')) return code.replace(
          "import { useLayoutEffect, useMemo, useRef, useState } from 'react';",
          "import { useMemo } from 'react'; import { useLayoutEffect, useRef, useState } from 'focus-state';").replace(
          "import { useEffect, useLayoutEffect, useMemo, useRef } from 'react';",
          "import { useMemo } from 'react'; import { useEffect, useLayoutEffect, useRef } from 'focus-state';");
        if (path.endsWith('/dashboard/useDiagnostics.ts')) return code.replace(
          "import { useEffect, useMemo } from 'react';",
          "import { useMemo } from 'react'; import { useEffect } from 'focus-state';");
        if (path.endsWith('/dashboard/countryNamesAt.ts')) return code.replace(
          "import { useEffect, useMemo, useRef, useState } from 'react';",
          "import { useMemo } from 'react'; import { useEffect, useRef, useState } from 'focus-state';");
        if (path.endsWith('/dashboard/products.ts')) return code.replace("from 'react'", "from 'focus-state'");
        if (path.endsWith('/dashboard/playback.ts')) return 'export const usePlaybackHour = () => {};';
        if (path.endsWith('/dashboard/contentRevision.ts')) return 'export const useContentRevision = () => 0; export const contentRevision = () => ({ snapshot: () => 0 });';
        if (path.endsWith('/components/ContextMenu.tsx')) return `import { createElement } from 'react'; export const useContextMenu = label => ({
          menu: createElement('span', { 'data-row-menu': label }), open: () => {} });`;
      },
      resolveId(id) { if (id === 'focus-state') return '\0focus-state'; },
      load(id) { if (id === '\0focus-state') return `export const useState = initial => globalThis.__focusHooks.state(initial);
        export const useRef = initial => globalThis.__focusHooks.ref(initial);
        export const useEffect = (effect, deps) => globalThis.__focusHooks.effect(effect, deps);
        export const useLayoutEffect = useEffect;`; },
    }] });
  const { default: renderer } = await vite.ssrLoadModule('/ui/src/widgets/national-focus/index.ts');
  const history: FocusReply = { history: { segments: [{ firstHour: 100, lastHour: 150, order: {
    completed: [{ ordinal: 0, token: 'done', completedByHour: null }], current: 'switch', continuous: null, paused: false,
  } }], progress: [{ firstHour: 100, lastHour: 150, progressRaw: '100000' }] },
  artwork: ['done', 'switch'].map(token => ({ token, name: 'Undated static name', description: null, baseDurationDays: 10, icon: null })), warnings: [] };
  type Pending = { owner: string; params: FocusArtworkParams; resolve: (reply: FocusArtworkReply) => void; reject: (error: Error) => void };
  const reads: Pending[] = [];
  const client = (owner: string) => ({ product: (request: ProductRequest) => request.product === 'national-focus'
    ? Promise.resolve({ sourceId: request.sourceId, revision: request.revision, product: request.product, data: history })
    : new Promise<FocusArtworkReply>((resolve, reject) => { reads.push({ owner, params: request.params as FocusArtworkParams, resolve, reject }); })
      .then(data => ({ sourceId: request.sourceId, revision: request.revision, product: request.product, data })) }) as unknown as DashboardClient;
  const clientA = client('A'), clientB = client('B');
  let hour = 100, diagnostics: string[] = [];
  const reports: string[][] = [];
  const props = { widget: { id: 'focus', kind: 'national-focus', trackIds: ['track'], countries: { kind: 'tags', tags: ['synthetic'] }, time: { mode: 'follow' } },
    runtime: { client: clientA, tracks: [{ id: 'track', axisOffset: 0, branch: { id: '1', firstHour: 100, lastHour: 150 }, source: { id: 'source', revision: '1' } }], clock: { getHour: () => hour } },
    active: true, headerSlot: null, onDiagnostics: (warnings: string[]) => { diagnostics = warnings; reports.push(warnings); } } as unknown as WidgetProps;
  const render = () => { state.begin(); return renderToStaticMarkup(createElement(renderer.render, props)); };
  const settle = () => new Promise<void>(resolve => setImmediate(resolve));
  const complete = (read: Pending, name: string | null, reason: string | null = null, warnings: string[] = [], icon = 'data:image/png;base64,YQ==') => read.resolve({
    branchId: read.params.branchId, hour: read.params.hour, tag: read.params.tag, warnings,
    items: read.params.tokens.map(token => ({ artwork: { token, name: token === 'switch' ? name : 'Completed at 100', description: null,
      baseDurationDays: 10, icon: reason || read.params.textOnly ? null : icon }, reason: token === 'switch' ? reason : null })),
  });
  try {
    render(); state.flush(); await settle(); render(); state.flush();
    assert.match(render(), /data-row-menu="National focus actions"/);
    assert.doesNotMatch(render(), /Undated static name/);
    const firstRead = reads.find(read => read.params.hour === 100)!;
    assert.deepEqual([...firstRead.params.tokens].sort(), ['done', 'switch']);
    complete(firstRead, 'Early choice'); await settle();
    assert.match(render(), /Early choice/);
    assert.equal(firstRead.params.textOnly, true);
    assert.doesNotMatch(render(), /src="data:image/); // Text does not wait for the unresolved image request.
    hour = 101; const pending = render(); state.flush();
    assert.doesNotMatch(pending, /Early choice/);
    assert.doesNotMatch(pending, /Loading game artwork/);
    assert.match(pending, /Loading name…/);
    assert.match(pending, /Completed at 100/);
    const next = reads.find(read => read.params.hour === 101)!;
    assert.deepEqual(next.params.tokens, ['switch']);
    complete(next, 'Later choice', null, [], 'data:image/png;base64,Yg=='); await settle();
    complete(reads.find(read => read.params.hour === 101 && !read.params.textOnly)!, 'Image-side title', null, [], 'data:image/png;base64,Yg=='); await settle();
    const changed = render();
    assert.match(changed, /Later choice/);
    assert.doesNotMatch(changed, /Image-side title/);
    assert.match(changed, /src="data:image\/png;base64,Yg=="/);
    assert.doesNotMatch(changed, /Early choice/);
    hour = 102; render(); state.flush();
    complete(reads.find(read => read.params.hour === 102)!, null, 'focus-window mode is not recorded', ['Named content warning']); await settle();
    const refused = render(); state.flush();
    assert.doesNotMatch(refused, /data-tooltip="focus-window mode is not recorded"/);
    assert.doesNotMatch(refused, /Artwork unavailable \(1\)/);
    assert.match(refused, /art-fallback/);
    assert.match(refused, /data-tooltip="[^"]*focus-window mode is not recorded/);
    assert.ok(diagnostics.includes('Named content warning'));
    const reportCount = reports.length;
    render(); state.flush(); render(); state.flush();
    assert.equal(reports.length, reportCount, 'equal diagnostics do not clear and re-report on each parent render');
    hour = 103; render(); state.flush();
    complete(reads.find(read => read.params.hour === 103)!, ''); await settle();
    assert.match(render(), /focus-now-text"><strong[^>]*><\/strong>/, 'defined empty name is not a failed name or token fallback');
    props.widget.time = { mode: 'pin', windows: { track: { firstOffset: 10, lastOffset: 10 } } };
    hour = 140; render(); state.flush();
    const pinned = reads.find(read => read.params.hour === 110)!;
    assert.ok(pinned, 'Pin requests its own supplying hour rather than playback');
    complete(pinned, 'Pinned choice'); await settle(); assert.match(render(), /Pinned choice/);
    props.widget.time = { mode: 'follow' }; hour = 120; render(); state.flush();
    const oldA = reads.find(read => read.owner === 'A' && read.params.hour === 120 && read.params.textOnly)!;
    props.runtime.client = clientB;
    assert.doesNotMatch(render(), /Pinned choice/); state.flush(); await settle(); render(); state.flush();
    complete(reads.find(read => read.owner === 'B' && read.params.hour === 120)!, 'B choice'); await settle(); assert.match(render(), /B choice/);
    props.runtime.client = clientA;
    assert.doesNotMatch(render(), /B choice/); state.flush(); await settle(); render(); state.flush();
    const newA = reads.filter(read => read.owner === 'A' && read.params.hour === 120 && read.params.textOnly).at(-1)!;
    assert.notEqual(newA, oldA);
    complete(oldA, 'Expired A choice'); await settle();
    assert.doesNotMatch(render(), /Expired A choice/);
    assert.doesNotMatch(render(), /Loading game artwork/);
    assert.match(render(), /data-tooltip="Key: switch[^"]*Loading focus name/);
    newA.reject(new Error('Synthetic native refusal'));
    reads.filter(read => read.owner === 'A' && read.params.hour === 120 && !read.params.textOnly).at(-1)!.reject(new Error('Synthetic image refusal')); await settle();
    assert.doesNotMatch(render(), /Game artwork request failed/);
    assert.match(render(), /data-tooltip="Key: switch[^"]*Focus name request failed/);
    assert.doesNotMatch(render(), /Artwork unavailable \(/);
    assert.match(render(), /art-fallback/);
  } finally { state.dispose(); delete host.__focusHooks; await vite.close(); }
});

test('Follow schedules the current and newest focus first while every time mode preserves rendered completion order and dates', async () => {
  const completed = Array.from({ length: 20 }, (_, ordinal) => ({ ordinal, token: `focus_${ordinal}`,
    completedByHour: ordinal === 0 ? null : 100 + ordinal }));
  const data: FocusReply = { history: { segments: [{ firstHour: 100, lastHour: 200, order: {
    completed, current: 'current_focus', continuous: null, paused: false,
  } }], progress: [] }, artwork: [], warnings: [] };
  const fixture = { data, wanted: [] as { tag: string; hour: number }[] };
  const host = globalThis as typeof globalThis & { __focusPriorities?: typeof fixture };
  host.__focusPriorities = fixture;
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': 'export const useProduct = () => ({ data: globalThis.__focusPriorities.data });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => {};',
    '/dashboard/useDiagnostics.ts': 'export const useDiagnostics = () => {};',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/national-focus/useFocusArtwork.ts': `export const useFocusArtwork = (_client, _track, _country, wanted) => {
      globalThis.__focusPriorities.wanted = wanted;
      return { warnings: [], read: new Map(wanted.map(({ tag, hour }) => [JSON.stringify([hour, tag]), {
        artwork: { token: tag, name: 'Localized ' + tag, description: 'Recorded description', baseDurationDays: 10, icon: null }, reason: null,
      }])) }; };`,
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'focus-priority-renderer', enforce: 'pre',
      transform(_code, id) { const path = id.replaceAll('\\', '/'); return Object.entries(replacements).find(([suffix]) => path.endsWith(suffix))?.[1]; },
    }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/national-focus/NationalFocusWidget.tsx');
    const props = { widget: { id: 'priority', kind: 'national-focus', trackIds: ['track'], countries: { kind: 'tags', tags: ['synthetic'] }, time: { mode: 'follow' } },
      runtime: { client: {}, tracks: [{ id: 'track', axisOffset: 0, branch: { id: '1', firstHour: 100, lastHour: 200 },
        source: { id: 'source', revision: '1' } }], clock: { getHour: () => 200 } }, active: true, headerSlot: null, onDiagnostics() {} } as unknown as WidgetProps;
    const completionPairs = completed.map(entry => ({ tag: entry.token, hour: entry.completedByHour ?? 100 }));
    const render = () => renderToStaticMarkup(createElement(Widget, props));
    for (const mode of ['follow', 'pin', 'campaign'] as const) {
      props.widget.time = mode === 'pin' ? { mode, windows: { track: { firstOffset: 100, lastOffset: 100 } } } : { mode };
      const markup = render();
      const current = { tag: 'current_focus', hour: 200 };
      const expected = mode === 'follow' ? [current, ...completionPairs.slice().reverse()]
        : mode === 'pin' ? [current, ...completionPairs] : [...completionPairs, current];
      assert.deepEqual(fixture.wanted, expected);
      const images = planRequests(new HourStore(64), fixture.wanted, 21, 8, 'wanted');
      assert.deepEqual(images.slice(0, 2).map(([hour, tags]) => ({ hour, tag: tags[0] })), expected.slice(0, 2));
      const rendered = [...markup.matchAll(/Key: (focus_\d+)/g)].map(match => match[1]);
      assert.deepEqual(rendered, completed.map(entry => entry.token), 'scheduling never reverses, hides or drops displayed rows');
      assert.match(markup, /Date not recorded/);
      assert.match(markup, /Recorded description/);
      assert.match(markup, /Localized current_focus/);
      assert.equal(completed[0].completedByHour, null);
      assert.deepEqual(data.history.segments[0].order.completed, completed);
    }
  } finally { delete host.__focusPriorities; await vite.close(); }
});
