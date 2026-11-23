import assert from 'node:assert/strict';
import test from 'node:test';
import { focusArtAsks, focusAsksAhead, focusReadHours, focusAt, focusCompletion, focusMissingAt, focusProgressAt, focusProgressView, focusRequest, focusShownAt, validateFocusReply, validateFocusArtworkReply } from './focusData.ts';
import { loadProduct } from '../../dashboard/products.ts';
import type { DashboardClient, ProductRequest } from '../../dashboard/client';
import type { WidgetProps } from '../../dashboard/definitions';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { HourStore, planRequests } from '../../dashboard/countryFacts.ts';
import type { FocusOrder, FocusReply } from '../../generated/dashboard';
import type { SegmentHours } from './focusData';

const request = focusRequest('source', 'v1', { branchId: '1', country: 'GER' });
const first = { completed: [{ ordinal: 0, token: 'one', completedByHour: null }], current: 'two', continuous: null, paused: false };
const reply: FocusReply = { history: { segments: [
  { firstHour: 100, lastHour: 123, order: first },
  { firstHour: 125, lastHour: 200, order: { ...first, completed: [...first.completed, { ordinal: 1, token: 'two', completedByHour: 125 }], current: null } },
], progress: [
  { firstHour: 100, lastHour: 110, progressRaw: '6800000' },
  { firstHour: 111, lastHour: 123, progressRaw: '6900000' },
  { firstHour: 125, lastHour: 140, progressRaw: '100000' },
  { firstHour: 141, lastHour: 160, progressRaw: '200000' },
] }, missing: [{ firstHour: 124, lastHour: 124, reason: 'No focus row was recorded for this country' }], artwork: [], continuousStaticText: [], warnings: [] };

test('focus histories reject overlap, future completion bounds and invalid artwork', () => {
  assert.equal(validateFocusReply(reply), reply);
  const invalid = (firstHour: number, lastHour: number, ordinal: number, completedByHour: number | null) => ({ ...reply, history: { progress: [], segments: [{ firstHour, lastHour, order: { ...first, completed: [{ ordinal, token: 'one', completedByHour }] } }] } });
  for (const bad of [invalid(100, 99, 0, null), invalid(100, 100, 3, null), invalid(100, 100, 0, 101), invalid(NaN, 100, 0, null)]) assert.throws(() => validateFocusReply(bad));
  for (const missing of [undefined, [{ firstHour: 5, lastHour: 4, reason: 'x' }], [{ firstHour: 1, lastHour: 2, reason: 3 }], [{ firstHour: 1, lastHour: 4, reason: 'x' }, { firstHour: 3, lastHour: 5, reason: 'y' }]])
    assert.throws(() => validateFocusReply({ ...reply, missing } as unknown as FocusReply));
  assert.throws(() => validateFocusReply({ ...reply, history: { ...reply.history, segments: [...reply.history.segments, reply.history.segments[0]] } }));
  for (const progressRaw of ['1.5', '', '99999999999999999999']) assert.throws(() => validateFocusReply({ ...reply, history: { ...reply.history, progress: [{ firstHour: 1, lastHour: 2, progressRaw }] } }));
  assert.throws(() => validateFocusReply({ ...reply, history: { ...reply.history, progress: [...reply.history.progress, reply.history.progress[0]] } }));
  const art = { token: 'one', name: 'One', description: 'Game description', baseDurationDays: 70, icon: null };
  assert.equal(validateFocusReply({ ...reply, artwork: [art] }).artwork[0].baseDurationDays, 70);
  for (const baseDurationDays of [-1, NaN, Infinity]) assert.throws(() => validateFocusReply({ ...reply, artwork: [{ ...art, baseDurationDays }] }));
  for (const icon of ['https://example.com/remote.png', 'data:text/html;base64,YQ==', 'data:image/png;base64,YQ==', `/api/artwork/../${'a'.repeat(32)}.png`]) {
    assert.equal(validateFocusReply(structuredClone({ ...reply, artwork: [{ ...art, icon }] })).artwork[0].icon, null);
  }
  for (const bad of [null, 'one', {}, { ...art, token: 7 }]) {
    assert.throws(() => validateFocusReply({ ...reply, artwork: [bad] } as unknown as FocusReply));
  }
  assert.equal(validateFocusReply({ ...reply, continuousStaticText: [art] }).continuousStaticText[0].name, 'One');
  for (const continuousStaticText of [undefined, [{ ...art, icon: 'http://artwork.localhost/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png' }], [null]]) {
    assert.throws(() => validateFocusReply({ ...reply, continuousStaticText } as unknown as FocusReply));
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

test('a view keys what it shows by segment or missing span, so following the head renders only where the order changes', () => {
  assert.equal(focusShownAt(reply, null), 'out');
  assert.equal(focusShownAt(reply, 100), 's0');
  assert.equal(focusShownAt(reply, 123), 's0', 'progress moving inside one order changes nothing shown');
  assert.equal(focusShownAt(reply, 124), 'm0');
  assert.equal(focusMissingAt(reply, 124)?.reason, 'No focus row was recorded for this country');
  assert.equal(focusShownAt(reply, 125), 's1');
  assert.equal(focusShownAt(reply, 99), 'none');
  assert.equal(focusMissingAt(reply, 99), null);
});

test('dated art is asked at the hours the order holds, never at the moving head', () => {
  const order = { completed: [{ ordinal: 0, token: 'one', completedByHour: null }, { ordinal: 1, token: 'two', completedByHour: 125 }], current: 'three', continuous: null, paused: false };
  const hours = { completed: [125, 125], active: 125 };
  assert.deepEqual(focusArtAsks(order, hours, true, true), [{ tag: 'three', hour: 125 }, { tag: 'two', hour: 125 }, { tag: 'one', hour: 125 }]);
  assert.deepEqual(focusArtAsks(order, hours, false, true), [{ tag: 'three', hour: 125 }, { tag: 'one', hour: 125 }, { tag: 'two', hour: 125 }]);
  assert.deepEqual(focusArtAsks(order, hours, false, false), [{ tag: 'one', hour: 125 }, { tag: 'two', hour: 125 }, { tag: 'three', hour: 125 }]);
  assert.deepEqual(focusArtAsks({ ...order, current: null, continuous: 'endless' }, hours, true, true), [{ tag: 'two', hour: 125 }, { tag: 'one', hour: 125 }], 'a continuous focus is not a national ask');
  assert.deepEqual(focusArtAsks(null, hours, true, true), []);
});

test('a new segment asks nothing again for the focuses it shares with the unbroken one before it', () => {
  const undated = { ordinal: 0, token: 'one', completedByHour: null };
  const history = { progress: [], segments: [
    { firstHour: 100, lastHour: 109, order: { completed: [undated], current: 'two', continuous: null, paused: false } },
    { firstHour: 110, lastHour: 119, order: { completed: [undated], current: 'two', continuous: null, paused: true } },
    { firstHour: 120, lastHour: 129, order: { completed: [undated, { ordinal: 1, token: 'two', completedByHour: 120 }], current: 'three', continuous: null, paused: false } },
    { firstHour: 140, lastHour: 149, order: { completed: [undated, { ordinal: 1, token: 'two', completedByHour: 120 }], current: 'three', continuous: null, paused: false } },
  ] };
  assert.deepEqual(focusReadHours(history), [
    { completed: [100], active: 100 },
    { completed: [100], active: 100 },
    { completed: [100, 120], active: 120 },
    { completed: [140, 120], active: 140 },
  ], 'a pause or a completion keeps the hours of what stays; a gap starts them again');
  const hours = focusReadHours(history);
  const now = focusArtAsks(history.segments[0].order, hours[0], true, true);
  const asks = (order: FocusOrder, held: SegmentHours) => focusArtAsks(order, held, true, true);
  assert.deepEqual(focusAsksAhead(now, history.segments, hours, 0, 120, asks), now, 'nothing begins before the window ends: the asks shown, the same');
  assert.deepEqual(focusAsksAhead(now, history.segments, hours, 0, 130, asks), [...now, { tag: 'three', hour: 120 }, { tag: 'two', hour: 120 }],
    'the focus begun and the completion within the window are asked ahead, once each, after what is shown');
  assert.deepEqual(focusAsksAhead(now, history.segments, hours, 0, Infinity, asks).slice(now.length + 2), [{ tag: 'three', hour: 140 }, { tag: 'one', hour: 140 }],
    'past a gap the same focuses are asked again at the hours that segment reads them');
  assert.deepEqual(focusAsksAhead(now, history.segments, hours, 3, Infinity, asks), now, 'the last segment has nothing ahead');
});

test('the painted progress line names the day reached, the days left only from a known rate, and banked days while idle', () => {
  assert.deepEqual(focusProgressView({ progress: 6_900_000n, dailyGain: 100_000n }, 70, true), { days: '· Day 70 of 70', left: { value: '1', unit: 'day left', estimated: true }, fraction: 0.9857, banked: null });
  assert.deepEqual(focusProgressView({ progress: 3_500_000n, dailyGain: null }, 70, true).left, { value: '50%', unit: 'complete', estimated: false });
  assert.deepEqual(focusProgressView(null, 70, true), { days: null, left: null, fraction: null, banked: null }, 'unknown progress is shown as nothing, never zero');
  assert.deepEqual(focusProgressView({ progress: 200_000n, dailyGain: null }, null, false).banked, '2 days of progress saved for the next focus');
  assert.equal(focusProgressView({ progress: 0n, dailyGain: null }, null, false).banked, null);
});

test('many focus widgets share one load while playback advances without requests', async () => {
  let calls = 0;
  const client = { product: async (input: ProductRequest) => { calls++; return { sourceId: input.sourceId, revision: input.revision, product: input.product, data: reply }; } } as unknown as DashboardClient;
  const results = await Promise.all(Array.from({ length: 24 }, () => loadProduct<FocusReply>(client, request, new AbortController().signal, validateFocusReply)));
  assert.equal(calls, 1);
  assert.ok(results.every(value => value === results[0]));
  for (let hour = 100; hour <= 200; hour++) for (const result of results) focusAt(result.history, hour);
  assert.equal(calls, 1);
  await loadProduct(client, focusRequest('source', 'v1', { branchId: '1', country: 'ENG' }), new AbortController().signal, validateFocusReply);
  assert.equal(calls, 2);
});

test('focus progress reports exact days and days left only from a known daily rate', () => {
  const late = focusProgressAt(reply.history, 115)!;
  assert.deepEqual(late, { progress: 6_900_000n, dailyGain: 100_000n });
  assert.deepEqual(focusCompletion(late, 70), { fraction: 0.9857, elapsedDays: 69, totalDays: 70, daysLeft: 1 });
  assert.equal(focusProgressAt(reply.history, 105)!.dailyGain, null);
  assert.equal(focusCompletion(focusProgressAt(reply.history, 105)!, 70)!.daysLeft, null);
  assert.equal(focusProgressAt(reply.history, 124), null);
  assert.equal(focusProgressAt(reply.history, 130)!.dailyGain, null);
  assert.equal(focusProgressAt(reply.history, 150)!.dailyGain, null);
  assert.equal(focusCompletion({ progress: 3_500_000n, dailyGain: 125_000n }, 70)!.daysLeft, 28);
  assert.equal(focusCompletion({ progress: 9_000_000n, dailyGain: 100_000n }, 70)!.fraction, 1);
  assert.equal(focusCompletion({ progress: 0n, dailyGain: null }, 0), null);
});

test('dated focus artwork authenticates the read, retains empty text and refuses omitted tokens', () => {
  const params = { branchId: '1', hour: 10, country: 'synthetic', database: 'national' as const, tokens: ['focus'] };
  const art = { token: 'focus', name: '', description: '', baseDurationDays: 10, icon: null as string | null };
  const dated = { branchId: '1', hour: 10, tag: 'synthetic', database: 'national', items: [{ artwork: art, reason: 'focus-window mode is not recorded' as string | null }], warnings: ['Named content warning'] };
  assert.equal(validateFocusArtworkReply(dated, params).items[0].reason, 'focus-window mode is not recorded');
  assert.equal(dated.items[0].artwork.name, '');
  for (const bad of [{ ...dated, hour: 11 }, { ...dated, tag: 'other' }, { ...dated, branchId: '2' }, { ...dated, database: 'continuous' }, { ...dated, items: [] },
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
  const params = { branchId: '1', hour: 10, country: 'synthetic', database: 'continuous' as const, tokens: artwork.map(art => art.token) };
  assert.equal(validateFocusArtworkReply({ branchId: params.branchId, hour: params.hour, tag: params.country, database: params.database,
    items: artwork.map(art => ({ artwork: art, reason: null })), warnings: [] }, params).items.length, 5000);
});

test('Follow schedules the current and newest focus first while every time mode preserves rendered completion order and dates', async () => {
  const completed = Array.from({ length: 20 }, (_, ordinal) => ({ ordinal, token: `focus_${ordinal}`,
    completedByHour: ordinal === 0 ? null : 100 + ordinal }));
  const data: FocusReply = { history: { segments: [{ firstHour: 100, lastHour: 200, order: {
    completed, current: 'current_focus', continuous: null, paused: false,
  } }], progress: [] }, missing: [], artwork: [], continuousStaticText: [], warnings: [] };
  const fixture = { data, wanted: [] as { tag: string; hour: number }[] };
  const host = globalThis as typeof globalThis & { __focusPriorities?: typeof fixture };
  host.__focusPriorities = fixture;
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': 'export const useProduct = () => ({ data: globalThis.__focusPriorities.data }); export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });',
    '/dashboard/useDiagnostics.ts': 'export const useDiagnostics = () => {};',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/national-focus/useFocusArtwork.ts': `export const useFocusArtwork = (_client, _track, _country, wanted) => {
      globalThis.__focusPriorities.wanted = wanted;
      return { warnings: [], read: new Map(wanted.map(({ tag, hour }) => [JSON.stringify([hour, tag]), {
        artwork: { token: tag, name: 'Localized ' + tag, description: 'Recorded description', baseDurationDays: 10, icon: null }, reason: null,
      }])) }; };
      export const useContinuousFocusArtwork = () => ({ warnings: [], read: new Map(), details: new Map() });`,
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'focus-priority-renderer', enforce: 'pre',
      transform(_code, id) { const path = id.replaceAll('\\', '/'); return Object.entries(replacements).find(([suffix]) => path.endsWith(suffix))?.[1]; },
    }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/national-focus/NationalFocusWidget.tsx');
    const props = { widget: { id: 'priority', kind: 'national-focus', trackIds: ['track'], countries: { kind: 'tags', tags: ['synthetic'] }, time: { mode: 'follow' } },
      runtime: { client: {}, tracks: [{ id: 'track', axisOffset: 0, branch: { id: '1', firstHour: 100, lastHour: 200 },
        source: { id: 'source', revision: '1', metadata: { campaignKey: 'campaign', playerCountryId: null, countries: [] } } }], clock: { getHour: () => 200, getSnapshot: () => ({ playing: false, hoursPerSecond: 0 }), subscribe: () => () => {}, subscribeFrame: () => () => {} } }, active: true, headerSlot: null, onDiagnostics() {} } as unknown as WidgetProps;
    const completionPairs = completed.map(entry => ({ tag: entry.token, hour: entry.completedByHour ?? 100 }));
    const render = () => renderToStaticMarkup(createElement(Widget, props));
    for (const mode of ['follow', 'pin', 'campaign'] as const) {
      props.widget.time = mode === 'pin' ? { mode, windows: { track: { firstOffset: 100, lastOffset: 100 } } } : { mode };
      const markup = render();
      const current = { tag: 'current_focus', hour: 100 };
      const expected = mode === 'follow' ? [current, ...completionPairs.slice().reverse()]
        : mode === 'pin' ? [current, ...completionPairs] : [...completionPairs, current];
      assert.deepEqual(fixture.wanted, expected);
      const images = planRequests(new HourStore(64), fixture.wanted, 21, 8, 'wanted');
      assert.deepEqual({ hour: images[0][0], tag: images[0][1][0] }, expected[0], 'the most wanted art is asked first');
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
