import assert from 'node:assert/strict';
import { test } from 'node:test';
import { sinkingWindowRows } from './shipsSunkWindows.ts';
import type { ShipsSunkWindowReply } from './useShipsSunk';
import { validateSinkingManifest, validateSinkingWindow } from './useShipsSunk.ts';
import type { ShipsSunkReply, ShipsSunkRow } from '../../generated/dashboard';
import { deepFreeze } from '../../testFrozen.ts';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { ComponentType } from 'react';
import type { WidgetProps } from '../../dashboard/definitions';
import type { LogListProps } from '../../charts/log/LogList';
import type { SinkingItem } from './shipsSunkWindows';
const row = (sinkHour: number, knownAt: number): ShipsSunkRow => ({ sinkHour, knownAt, name: 'recorded ship', classIndex: 0,
  recordedNameIndex: 0, victim: 'mod victim', attacker: null, victimAsOf: null, attackerAsOf: null, placeIndex: 0,
  cause: 'hostile', attributionName: null, assistedKill: false });
function page(window: number, indices: number[], rows: ShipsSunkRow[], token: number): ShipsSunkWindowReply {
  const reply: ShipsSunkReply = { branchId: '1', available: true, firstHour: 100, lastHour: 130, firstRead: 100, unread: [],
    classes: [{ token, key: `mod class ${token}`, name: null, icon: null }], places: [`mod sea ${token}`],
    recordedNames: [`recorded class ${token}`], rows, warnings: [] };
  return { window, indices, reply };
}
test('window dictionaries and stable sinking indices survive a late native-date repair', () => {
  const first = page(0, [0, 3, 4], [row(99, 100), row(105, 130), row(105, 130)], 70);
  const second = page(1, [1, 2], [row(115, 120), row(115, 120)], 4);
  const pages = new Map([[0, first], [1, second]]);
  const read = (hour: number, from: number | null = null) => sinkingWindowRows(pages, hour, [], 'victim', () => true, from, 'mod campaign');
  assert.deepEqual(read(120).map(row => row.key), ['1', '2', '0']);
  const visible = read(130);
  assert.deepEqual(visible.map(row => row.key), ['1', '2', '3', '4', '0']);
  assert.equal(visible[0].item.page.places[visible[0].item.placeIndex!], 'mod sea 4');
  assert.equal(visible[2].item.page.recordedNames[visible[2].item.recordedNameIndex!], 'recorded class 70');
  assert.deepEqual(read(130, 105).map(row => row.key), ['1', '2', '3', '4']);
  assert.deepEqual(sinkingWindowRows(pages, 130, ['mod class 70'], 'both', () => true, null, 'mod campaign').map(row => row.key), ['3', '4', '0']);
});

test('sinking content is prepared once and retained until a visibility boundary or replacement', () => {
  let reads = 0;
  const old = row(100, 100), future = row(110, 130);
  Object.defineProperty(future, 'knownAt', { get() { reads++; return 130; } });
  const original = deepFreeze(page(0, [0, 1], [old, future], 70));
  const pages: ReadonlyMap<number, ShipsSunkWindowReply> = deepFreeze(new Map([[0, original]]));
  const country = () => true;
  const read = (hour: number) => sinkingWindowRows(pages, hour, [], 'both', country, null, 'campaign');
  const first = read(101);
  reads = 0;
  for (let hour = 102; hour < 130; hour++) assert.equal(read(hour), first);
  assert.equal(reads, 0);
  const crossed = read(130);
  assert.notEqual(crossed, first);
  assert.equal(crossed[1], first[0]);
  assert.equal(crossed.length, 2);
  const repaired = deepFreeze({ ...original, reply: { ...original.reply,
    rows: original.reply.rows.map(row => ({ ...row, name: 'repaired' })) } });
  const replacement = deepFreeze(new Map([[0, repaired]]));
  assert.equal(sinkingWindowRows(replacement, 130, [], 'both', country, null, 'campaign')[0].item.name, 'repaired');
});

test('the real sinking widget passes unchanged rows to LogList while only playback advances', async () => {
  const window = deepFreeze(page(0, [0, 1], [row(100, 100), row(110, 130)], 70));
  const state = { hour: 101, log: undefined as LogListProps<SinkingItem> | undefined,
    manifest: { classes: window.reply, windows: [{ firstHour: 100, lastHour: 160 }] },
    pages: deepFreeze(new Map([[0, window]])), errors: deepFreeze(new Map<number, string>()),
    track: { id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { campaignKey: 'campaign', countries: [] } },
      branch: { id: '1', firstHour: 100, lastHour: 160, gaps: [] } } };
  const memos: { deps: readonly unknown[]; value: unknown }[] = [];
  let cursor = 0;
  const memo = (build: () => unknown, deps: readonly unknown[]) => {
    const index = cursor++, before = memos[index];
    if (!before || before.deps.length !== deps.length || deps.some((value, index) => !Object.is(value, before.deps[index])))
      memos[index] = { deps: [...deps], value: build() };
    return memos[index].value;
  };
  const hooks = { memo, callback: (callback: unknown, deps: readonly unknown[]) => memo(() => callback, deps) };
  const host = globalThis as typeof globalThis & { __sinkingPlayback?: typeof state; __sinkingHooks?: typeof hooks };
  host.__sinkingPlayback = state; host.__sinkingHooks = hooks;
  const replacements: Record<string, string> = {
    '/widgets/ships-sunk/useShipsSunk.ts': `export const useShipsSunk = () => ({ track: globalThis.__sinkingPlayback.track, data: globalThis.__sinkingPlayback.manifest, retry() {} });
      export const useShipsSunkBattleTitles = () => ({}); export const validateSinkingWindow = value => value;
      export const useShipClassChoices = () => ({});`,
    '/dashboard/productWindows.ts': `export const useProductWindows = () => ({ data: globalThis.__sinkingPlayback.pages, errors: globalThis.__sinkingPlayback.errors, retry() {} });`,
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => globalThis.__sinkingPlayback.hour;',
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({});',
    '/dashboard/countryFlagsAt.ts': `const label = () => ({ name: 'synthetic' }), labelAt = () => label;
      export const useCountryLabelsAt = () => ({ labelAt }); export const flagReadMetadata = () => ({});`,
    '/charts/log/LogList.tsx': 'export const PreparedLogCoverage = () => null; export default props => { globalThis.__sinkingPlayback.log = props; return null; };',
    '/components/ChipSet.tsx': 'export const ChipSet = () => null;',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'sinking-playback-caller', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        const replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
        if (replacement) return replacement[1];
        if (path.endsWith('/widgets/ships-sunk/ShipsSunkWidget.tsx')) return source
          .replace(/\buseMemo(?:<[^;\n]*>)?\(/g, 'globalThis.__sinkingHooks.memo(')
          .replace(/\buseCallback\(/g, 'globalThis.__sinkingHooks.callback(');
      },
    }] });
  try {
    await vite.ssrLoadModule('/ui/src/widgets/definitions.ts');
    const { createWidget } = await vite.ssrLoadModule('/ui/src/dashboard/model.ts');
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/ships-sunk/ShipsSunkWidget.tsx') as { default: ComponentType<WidgetProps> };
    const props = { widget: { ...createWidget('ships-sunk', 'main'), countries: { kind: 'all' }, time: { mode: 'follow' } },
      runtime: { tracks: [state.track], client: {}, clock: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => { cursor = 0; renderToStaticMarkup(createElement(Widget, props)); return state.log!.rows!; };
    const first = render();
    assert.equal(first.length, 1);
    for (state.hour = 102; state.hour < 130; state.hour++) assert.equal(render(), first);
    const crossed = render();
    assert.notEqual(crossed, first);
    assert.equal(crossed.length, 2);
    assert.equal(crossed[1], first[0]);
  } finally { await vite.close(); delete host.__sinkingPlayback; delete host.__sinkingHooks; }
});

test('pending sinking pages preserve their coverage so held windows can follow repairs', () => {
  const pending = [{ firstHour: 111, lastHour: 130 }];
  const value = { ...page(0, [0], [row(105, 110)], 70),
    coverage: { schema: 1, firstHour: 100, lastHour: 110, complete: false }, pending };
  assert.deepEqual(validateSinkingWindow(value).coverage, value.coverage);
  assert.deepEqual(validateSinkingWindow(value).pending, pending);
  assert.deepEqual(validateSinkingManifest({ windows: [{ firstHour: 100, lastHour: 130 }], classes: value.reply,
    coverage: { firstHour: 100, lastHour: 99, complete: false }, pending }).pending, pending);
  assert.throws(() => validateSinkingWindow({ ...value, pending: [{ firstHour: 130, lastHour: 111 }] }));
});
