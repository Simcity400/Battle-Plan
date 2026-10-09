import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { ProductRequest } from './client';
import type { WidgetProps } from './definitions';
import { syntheticRaids } from './raids.fixtures.ts';

test('actual Raid and windowed Espionage logs refuse aligned Follow and Pin hours outside their branch', async () => {
  const common = { branchId: '1', windows: [], completionsAvailable: [],
    completionsUnrecorded: 'Synthetic missing completion events', warnings: [], unavailable: null,
    types: [{ key: 'synthetic-operation', name: 'Synthetic operation', icon: null, iconMissing: null }] };
  const early = { ...common, window: 0, firstHour: 100, lastHour: 149,
    available: [{ firstHour: 100, lastHour: 149 }], readHours: [100, 110, 115, 116, 149],
    operations: (['completed', 'disappeared'] as const).map((reason, index) => ({ id: `operation-${index}`, definitionToken: index,
      firstRead: 100, startHour: 100, startKnownAt: 100, runs: [{ firstHour: 100, lastHour: 110, readHour: 100,
        ownerTag: 'MOD A', targetTag: 'MOD B', typeKey: 'synthetic-operation', phaseCount: 1, autoRepeat: false,
        durationDays: 8, assignedSlots: 0, preparationRatio: 35000, elapsedDays: 2, currentPhase: 0,
        terminal: false, autoCommence: true, startHour: 100, operatives: [], operativeNames: [] }],
      end: { byHour: 115 + index, lastSeen: 110, reason, completionHour: null, completionSequence: null,
        typeKey: 'synthetic-operation', outcome: null, outcomeUnrecorded: 'Synthetic unknown outcome', ownerTag: 'MOD A', targetTag: 'MOD B' } })),
  };
  const late = { ...common, window: 1, firstHour: 150, lastHour: 200,
    available: [{ firstHour: 150, lastHour: 200 }], readHours: [150, 200], operations: [] };
  const manifest = { ...common, window: null, firstHour: 100, lastHour: 200,
    available: [{ firstHour: 100, lastHour: 200 }], readHours: [], operations: [],
    windows: [{ firstHour: 100, lastHour: 149 }, { firstHour: 150, lastHour: 200 }] };
  const host = globalThis as typeof globalThis & { __pinLogCalls?: number[][]; __pinLogWindowRequests?: ProductRequest[] };
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': `export const useProduct = (_client, request, _active, validate) => {
      if (!request) return { retry() {} };
      const data = request.product === 'raids' ? ${JSON.stringify(syntheticRaids())}
        : request.product === 'espionage-log' && request.params.window === null ? ${JSON.stringify(manifest)} : undefined;
      if (data === undefined) throw new Error('Unexpected branch-product request');
      return { data: validate(data), retry() {} };
    }; export const loadTransientProduct = () => { throw new Error('Unexpected detail tooltip request'); };`,
    '/dashboard/productWindows.ts': `export const useProductWindows = (_client, identity, request, wanted, active, validate) => {
      const windows = ${JSON.stringify([early, late])};
      return { data: new Map(active && identity !== null ? wanted.map(index => {
        const asked = request(index); globalThis.__pinLogWindowRequests.push(asked);
        return [index, validate(windows[asked.params.window])];
      }) : []), errors: new Map(), retry() {} };
    };`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ retry() {} });',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/charts/log/LogList.tsx': 'export default props => { globalThis.__pinLogCalls.push(props.rows.map(row => row.hour)); return null; };',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'windowed-log-pin-boundaries', enforce: 'pre',
      transform: (_source, id) => Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1],
    }] });
  try {
    const { default: Raid } = await vite.ssrLoadModule('/ui/src/widgets/raid-log/RaidLogWidget.tsx') as typeof import('../widgets/raid-log/RaidLogWidget');
    const { default: Espionage } = await vite.ssrLoadModule('/ui/src/widgets/espionage-log/EspionageLogWidget.tsx') as typeof import('../widgets/espionage-log/EspionageLogWidget');
    const { pinTime, resolveWindow } = await vite.ssrLoadModule('/ui/src/dashboard/model.ts') as typeof import('./model');
    let position = -850;
    const props = { widget: { id: 'log', kind: 'raid-log', trackIds: ['short'], countries: { kind: 'all' }, time: { mode: 'follow' }, options: {} },
      runtime: { tracks: [{ id: 'short', axisOffset: 1000, source: { id: 'synthetic', revision: '1' }, branch: { id: '1', firstHour: 100, lastHour: 200 } }],
        clock: { getHour: () => position, subscribeHour: () => () => {} }, client: {} }, active: true, onDiagnostics() {},
    } as unknown as WidgetProps;
    const track = props.runtime.tracks[0];
    for (const [kind, Widget, firstDates, lastDates] of [
      ['raid-log', Raid, [100], [103]], ['espionage-log', Espionage, [100, 100], [116, 115]],
    ] as const) {
      props.widget = { ...props.widget, kind };
      for (const hour of [99, 201]) {
        position = hour - track.axisOffset;
        const follow = { ...props.widget, time: { mode: 'follow' as const } };
        for (const time of [follow.time, { mode: 'pin' as const, windows: { short: { firstOffset: hour - 100, lastOffset: hour - 100 } } },
          pinTime(follow, [track], position)]) {
          props.widget = { ...props.widget, time };
          assert.equal(resolveWindow(props.widget, track, [track], position)?.lastHour, hour);
          host.__pinLogCalls = []; host.__pinLogWindowRequests = [];
          assert.match(renderToStaticMarkup(createElement(Widget, props)), /outside.*branch/);
          assert.deepEqual(host.__pinLogCalls, []);
        }
      }
      // Ended rows at the later endpoint come from the preceding requested detail window.
      position = 5000;
      for (const [hour, dates] of [[100, firstDates], [200, lastDates]] as const) {
        props.widget = { ...props.widget, time: { mode: 'pin', windows: { short: { firstOffset: hour - 100, lastOffset: hour - 100 } } } };
        host.__pinLogCalls = []; host.__pinLogWindowRequests = [];
        assert.doesNotMatch(renderToStaticMarkup(createElement(Widget, props)), /outside.*branch/);
        assert.deepEqual(host.__pinLogCalls, [dates]);
        assert.deepEqual(host.__pinLogWindowRequests, kind === 'raid-log' ? [] : (hour === 100 ? [0] : [0, 1]).map(window => ({
          sourceId: 'synthetic', revision: '1', product: 'espionage-log', params: { branchId: '1', window },
        })));
      }
    }
  } finally {
    delete host.__pinLogCalls;
    delete host.__pinLogWindowRequests;
    await vite.close();
  }
});
