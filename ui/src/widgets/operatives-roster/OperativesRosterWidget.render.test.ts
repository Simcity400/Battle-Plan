import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';

test('an empty roster at an hour whose group membership is still being prepared is loading, not none', async () => {
  const state = { groups: pendingGroups() };
  const host = globalThis as typeof globalThis & { __operativesPending?: typeof state }; host.__operativesPending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/operatives-roster/OperativesRosterWidget.tsx', {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__operativesPending.groups, retry() {} });',
    '/dashboard/products.ts': `export const useProduct = () => ({ retry() {}, data: { countries: [{ tag: "GER", observed: [{ firstHour: 0, lastHour: 200 }], operatives: [] }], agencyAbsent: [], warnings: [] } });`,
    '/charts/table/useTableHour.ts': 'export const useTableHour = () => 200;',
    '/dashboard/playback.ts': 'export const usePlaybackSpeed = () => 0;',
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({ fontSize: { title: 12, small: 10 } }); export const useViewportHeight = () => 1000;',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/countryNamesAt.ts': 'export const nameKey = (hour, tag) => JSON.stringify([hour, tag]); export const useCountryNamesAt = () => new Map();',
  });
  try {
    const props = { widget: { id: 'operatives-roster', kind: 'operatives-roster', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [], campaignKey: 'campaign' } }, branch: { ...BRANCH, gaps: [] } }],
        client: {}, clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ hoursPerSecond: 0, playing: false }) } }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // The hour shown is still being prepared: its human players are not known yet.
    const pending = render();
    assert.match(pending, /This range is still being prepared\./);
    assert.doesNotMatch(pending, /No operatives in the selected roster\./);
    // The final reply proves there is no human player then.
    state.groups = pendingGroups({ final: true });
    assert.match(render(), /No operatives in the selected roster\./);
  } finally { delete host.__operativesPending; await close(); }
});

test('a roster of a group with no member found yet says what the hour was once read, and loads until then', async () => {
  const state = { groups: { ...pendingGroups({ humans: [] }), unobserved: { humans: [{ firstHour: 140, lastHour: 150 }], factions: [], wars: [] } }, hour: 0 };
  const host = globalThis as typeof globalThis & { __operativesGap?: typeof state }; host.__operativesGap = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/operatives-roster/OperativesRosterWidget.tsx', {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__operativesGap.groups, retry() {} });',
    // No request, no reply.
    '/dashboard/products.ts': `export const useProduct = (_client, request) => ({ retry() {}, data: request ? { countries: [], agencyAbsent: [], warnings: [] } : undefined });`,
    '/charts/table/useTableHour.ts': 'export const useTableHour = () => globalThis.__operativesGap.hour;',
    '/dashboard/playback.ts': 'export const usePlaybackSpeed = () => 0;',
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({ fontSize: { title: 12, small: 10 } }); export const useViewportHeight = () => 1000;',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/countryNamesAt.ts': 'export const nameKey = (hour, tag) => JSON.stringify([hour, tag]); export const useCountryNamesAt = () => new Map();',
  });
  try {
    const render = (hour: number) => { state.hour = hour; return renderToStaticMarkup(createElement(module.default, { widget: { id: 'operatives-roster', kind: 'operatives-roster',
      trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'follow' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [], campaignKey: 'campaign' } }, branch: { ...BRANCH, gaps: [] } }],
        client: {}, clock: { getHour: () => hour, getSnapshot: () => ({ hoursPerSecond: 0, playing: false }) } }, active: true, onDiagnostics() {} } as unknown as WidgetProps)); };
    assert.match(render(145), /Group membership is unavailable at this time\./);
    assert.match(render(100), /This group has no members at this moment\./);
    const later = render(170);
    assert.match(later, /aria-label="This range is still being prepared\."/);
    assert.doesNotMatch(later, /unavailable|no members/);
  } finally { delete host.__operativesGap; await close(); }
});
