import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';
import type { CountryGroupReply, ScoreboardColumn } from '../../generated/dashboard';

test('a scoreboard of a group with no member known at an hour still being prepared is loading, and empty once read', async () => {
  const factories: ScoreboardColumn = { key: 'factories', label: 'Factories', unit: 'count', decimals: 0, available: true, reason: null };
  const registry = { hour: BRANCH.lastHour, maxima: {}, data: { branchId: BRANCH.id, defaultColumns: ['factories'], defaultSort: 'factories', columns: [factories], series: [], warnings: [] } };
  const state = { groups: pendingGroups() as CountryGroupReply };
  const host = globalThis as typeof globalThis & { __scoreboardPending?: typeof state }; host.__scoreboardPending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/scoreboard/ScoreboardWidget.tsx', {
    '/dashboard/products.ts': `export const useProduct = () => ({ data: ${JSON.stringify(registry)}, retry() {} });`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__scoreboardPending.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackSpeed = () => 0;',
    '/charts/table/useTableHour.ts': `export const useTableHour = () => ${BRANCH.lastHour};`,
    '/charts/useChartTokens.ts': 'export const useViewportHeight = () => 800;',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/countryNamesAt.ts': 'export const nameKey = (hour, tag) => `${hour}:${tag}`; export const useCountryNamesAt = () => new Map();',
    '/widgets/scoreboard/preparedScoreboard.ts': `export const scoreboardRegistryRequest = branchId => ({ branchId }); export const validatePreparedScoreboard = value => value;
      export const preparedScoreboardRows = () => []; export const usePreparedScoreboard = (_client, _source, request) => ({ index: request ? { data: globalThis.__scoreboardPending.index } : undefined, retry() {} });`,
  });
  try {
    const props = { widget: { id: 'scoreboard', kind: 'scoreboard', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => BRANCH.lastHour }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // The branch end is still being prepared: its human players are not known yet, so nothing is asked.
    assert.match(render(), /aria-label="This range is still being prepared\."/);
    // The final reply proves no human player at the end: no countries, not a reply that never comes.
    state.groups = pendingGroups({ final: true });
    assert.match(render(), /No countries available for this selection\./);
  } finally { delete host.__scoreboardPending; await close(); }
});

test('a scoreboard of a group with no member found yet says what the hour was once read, and loads until then', async () => {
  const factories: ScoreboardColumn = { key: 'factories', label: 'Factories', unit: 'count', decimals: 0, available: true, reason: null };
  const registry = { hour: BRANCH.lastHour, maxima: {}, data: { branchId: BRANCH.id, defaultColumns: ['factories'], defaultSort: 'factories', columns: [factories], series: [], warnings: [] } };
  const state = { groups: { ...pendingGroups({ humans: [] }), unobserved: { humans: [{ firstHour: 140, lastHour: 150 }], factions: [], wars: [] } } as CountryGroupReply, hour: 0 };
  const host = globalThis as typeof globalThis & { __scoreboardGap?: typeof state }; host.__scoreboardGap = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/scoreboard/ScoreboardWidget.tsx', {
    '/dashboard/products.ts': `export const useProduct = (_client, request) => ({ data: request ? ${JSON.stringify(registry)} : undefined, retry() {} });`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__scoreboardGap.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackSpeed = () => 0;',
    '/charts/table/useTableHour.ts': 'export const useTableHour = () => globalThis.__scoreboardGap.hour;',
    '/charts/useChartTokens.ts': 'export const useViewportHeight = () => 800;',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/countryNamesAt.ts': 'export const nameKey = (hour, tag) => `${hour}:${tag}`; export const useCountryNamesAt = () => new Map();',
    // No request, no reply.
    '/widgets/scoreboard/preparedScoreboard.ts': `export const scoreboardRegistryRequest = branchId => ({ branchId }); export const validatePreparedScoreboard = value => value;
      export const preparedScoreboardRows = () => []; export const usePreparedScoreboard = (_client, _source, request) => ({ index: request ? { data: { rows: [] } } : undefined, retry() {} });`,
  });
  try {
    const render = (hour: number) => { state.hour = hour; return renderToStaticMarkup(createElement(module.default, { widget: { id: 'scoreboard', kind: 'scoreboard',
      trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'follow' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => hour }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps)); };
    assert.match(render(145), /Group membership is unavailable at this time\./);
    assert.match(render(100), /This group has no members at this moment\./);
    const later = render(170);
    assert.match(later, /aria-label="This range is still being prepared\."/);
    assert.doesNotMatch(later, /unavailable|no members/);
  } finally { delete host.__scoreboardGap; await close(); }
});
