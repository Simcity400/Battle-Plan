import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { LogListProps } from '../../charts/log/LogList';
import type { WidgetProps } from '../../dashboard/definitions';
import type { EspionageLogReply } from '../../generated/dashboard';
import type { OperationAt } from './espionageData';

test('parity espionage prepares every real history window behind an empty newest view without a name or detail sweep', async () => {
  const windows = [{ firstHour: 10, lastHour: 20 }, { firstHour: 21, lastHour: 30 }, { firstHour: 31, lastHour: 40 }];
  const manifest: EspionageLogReply = { branchId: '1', window: null, windows, firstHour: 10, lastHour: 40,
    available: [{ firstHour: 10, lastHour: 40 }], completionsAvailable: [], completionsUnrecorded: null,
    readHours: [], unavailable: null, warnings: [], operations: [],
    types: [{ key: 'mod operation', name: 'Recorded operation', icon: 'data:decoration', iconMissing: null }] };
  const replies = windows.map((interval, window): EspionageLogReply => ({ ...manifest, ...interval, window, windows: [], readHours: [interval.firstHour] }));
  replies[0].operations = [{ definitionToken: 42, id: 'first-episode', firstRead: 10, startHour: 12, startKnownAt: 12,
    runs: [{ firstHour: 10, lastHour: 17, readHour: 10, ownerTag: 'mod owner', targetTag: 'mod target', typeKey: 'mod operation',
      phaseCount: 4, autoRepeat: false, durationDays: 8, assignedSlots: 3, preparationRatio: 35001, elapsedDays: 2,
      currentPhase: 0, terminal: false, autoCommence: true, startHour: 12, operatives: ['123', '456'], operativeNames: null }],
    end: { byHour: 18, lastSeen: 17, reason: 'completed', completionHour: 17, completionSequence: null, typeKey: 'mod operation',
      outcome: null, outcomeUnrecorded: 'Outcome not recorded', ownerTag: 'mod owner', targetTag: 'mod target' } }];
  const state = { manifest, requested: false, wanted: [] as readonly number[], chunks: new Map(replies.map((reply, index) => [index, reply])),
    errors: new Map<number, Error>(), wantedCountries: [] as unknown[], log: undefined as LogListProps<OperationAt> | undefined };
  const host = globalThis as typeof globalThis & { __parityEspionage?: typeof state };
  host.__parityEspionage = state;
  const replacements: Record<string, string> = {
    '/widgets/espionage-log/useEspionageLog.ts': `export const useEspionageLog = (_widget, runtime) => ({track: runtime.tracks[0], data: globalThis.__parityEspionage.manifest, retry() {}});
      export const useEspionageWindows = (_track, _runtime, wanted) => { const state = globalThis.__parityEspionage; state.wanted = wanted;
        return {data: new Map(wanted.flatMap(index => state.chunks.has(index) ? [[index, state.chunks.get(index)]] : [])), errors: state.errors, retry() {}}; };`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({});',
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = (_client, _track, wanted) => {
      globalThis.__parityEspionage.wantedCountries = wanted; return {labelAt: () => tag => ({name: tag})}; };`,
    '/dashboard/operationDescription.ts': 'export const loadOperationDescription = () => { throw new Error("Offscreen description demand"); };',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    '/charts/log/LogList.tsx': 'export const PreparedLogCoverage = () => null; export default props => { globalThis.__parityEspionage.log = props; return null; };',
  };
  const vite = await createServer({ configFile: false, mode: 'parity', optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'parity-espionage', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        const replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
        if (replacement) return replacement[1];
        if (path.endsWith('/dashboard/parityPreparation.ts')) return source.replace(
          'const [requested, setRequested] = useState(false);',
          'const requested = globalThis.__parityEspionage.requested; const setRequested = value => { globalThis.__parityEspionage.requested = value; };');
      },
    }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/espionage-log/EspionageLogWidget.tsx');
    const props = { widget: { id: 'operations', kind: 'espionage-log', trackIds: ['main'], countries: { kind: 'all' },
      time: { mode: 'campaign' }, options: { chips: false } }, runtime: { client: {}, clock: { getHour: () => 40 }, tracks: [{
        id: 'main', axisOffset: 0, branch: { id: '1', firstHour: 10, lastHour: 40 }, source: { id: 'synthetic', revision: '1' },
      }] }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(Widget, props));
    const preparation = () => { const value = state.log!.paritySemantic?.preparation; assert.ok(value); return value; };
    render();
    assert.deepEqual(state.wanted, [1, 2], 'ordinary history demand stays bounded');
    assert.equal(state.log!.rows!.length, 0);
    assert.equal(state.log!.parityHistoryComplete, false, 'the empty newest view cannot certify the older history');
    const id = preparation().id;
    preparation().prepare();
    render();
    assert.deepEqual(state.wanted, [0, 1, 2]);
    assert.equal(state.log!.paritySemantic!.complete, true);
    assert.equal(preparation().id, id, 'empty and populated captures retain their preparation identity');
    assert.equal(state.log!.rows![0].item.operation.id, 'first-episode');
    assert.equal(state.log!.rows![0].hour, 17);
    const content = state.log!.paritySemantic!.content!(state.log!.rows![0]);
    assert.deepEqual(content, { key: 'mod operation', name: 'Recorded operation' });
    assert.match(JSON.stringify(content), /Recorded operation/);
    assert.doesNotMatch(JSON.stringify(content), /decoration/);
    assert.deepEqual(state.wantedCountries, [], 'complete model demand does not admit offscreen dated facts');
    state.chunks.delete(0); render();
    assert.equal(state.log!.parityHistoryComplete, false);
    assert.equal(preparation().ready(), false, 'missing real windows cannot certify completion');
    state.errors.set(0, new Error('Synthetic history failure'));
    assert.match(render(), /data-detail="Synthetic history failure"/, 'the original history reader exposes its named failure');
    state.errors.clear(); state.chunks.set(0, replies[0]);
    props.widget.time = { mode: 'range', firstOffset: 11, lastOffset: 30 };
    render();
    assert.deepEqual(state.wanted, [1, 2], 'range preparation selects exactly the real intersecting history');
    assert.equal(state.log!.rows!.length, 0);
    assert.equal(state.log!.paritySemantic!.complete, true);
  } finally { await vite.close(); delete host.__parityEspionage; }
});
