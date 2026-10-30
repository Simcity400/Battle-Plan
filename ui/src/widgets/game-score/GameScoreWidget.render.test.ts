import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import { assertNoMemberStates, BRANCH, gapGroups, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';
import type { CountryGroupReply, GameScoreReply, GameScoreSeries } from '../../generated/dashboard';
import { FALLBACK_LABELS, SCORE_PARTS } from './gameScoreData.ts';

test('game scores of a group at an hour still being prepared are loading, not missing', async () => {
  const span = { firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour };
  const scores = Object.fromEntries(SCORE_PARTS.map(part => [part, [{ ...span, raw: '100000', readEachHour: true }]])) as GameScoreSeries;
  const reply: GameScoreReply = { branchId: BRANCH.id, scale: 100000, reads: [0], reasons: [], multipliers: scores, warnings: [], scoreWarnings: [],
    labels: FALLBACK_LABELS, hints: {}, countries: [{ tag: 'GER', scores, identity: [{ ...span, value: { canonicalSlot: 1, nativeCountry: 101, name: 'GER',
      nameMissing: null, color: null, colorMissing: null, faction: null, factionMissing: null } }] }] };
  const state = { groups: pendingGroups() as CountryGroupReply };
  const host = globalThis as typeof globalThis & { __scorePending?: typeof state }; host.__scorePending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/game-score/GameScoreWidget.tsx', {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__scorePending.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackMotion = () => 0;',
    '/charts/table/useTableHour.ts': `export const useTableHour = () => ${BRANCH.lastHour};`,
    '/charts/useChartTokens.ts': "export const useChartTokens = () => ({ categories: [], muted: '#888' }); export const useViewportHeight = () => 800;",
    '/dashboard/countryNamesAt.ts': 'export const nameKey = (hour, tag) => `${hour}:${tag}`; export const useCountryFactsRead = () => new Map();',
    '/dashboard/countryFlags.ts': 'export const useCountryFlagsRead = () => new Map();',
    '/widgets/game-score/usePreparedScore.ts': `export const usePreparedScore = (_runtime, _track, hour) => ({ data: { reply: ${JSON.stringify(reply)}, stretches: {} }, hour, pending: false, retry() {} });`,
    '/charts/table/Table.tsx': 'export default () => "score table"; export const CountryCell = () => null;',
  });
  try {
    const props = { widget: { id: 'score', kind: 'game-score', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: { view: 'table' } },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => BRANCH.lastHour }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // The branch end is still being prepared: its human players are not known yet.
    const pending = render();
    assert.match(pending, /aria-label="This range is still being prepared\."/);
    assert.doesNotMatch(pending, /No country data/);
    // The final reply proves no human player at the end.
    state.groups = pendingGroups({ final: true });
    assert.match(render(), /No country data at this time\./);
  } finally { delete host.__scorePending; await close(); }
});

test('game scores of a group with no member found yet say what the hour was once read, and load until then', async () => {
  const empty: GameScoreReply = { branchId: BRANCH.id, scale: 100000, reads: [0], reasons: [], multipliers: {} as GameScoreSeries, warnings: [], scoreWarnings: [],
    labels: FALLBACK_LABELS, hints: {}, countries: [] };
  const state = { groups: gapGroups(), hour: 0, reply: empty };
  const host = globalThis as typeof globalThis & { __scoreGap?: typeof state }; host.__scoreGap = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/game-score/GameScoreWidget.tsx', {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__scoreGap.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackMotion = () => 0;',
    '/charts/table/useTableHour.ts': 'export const useTableHour = () => globalThis.__scoreGap.hour;',
    '/charts/useChartTokens.ts': "export const useChartTokens = () => ({ categories: [], muted: '#888' }); export const useViewportHeight = () => 800;",
    '/dashboard/countryNamesAt.ts': 'export const nameKey = (hour, tag) => `${hour}:${tag}`; export const useCountryFactsRead = () => new Map();',
    '/dashboard/countryFlags.ts': 'export const useCountryFlagsRead = () => new Map();',
    '/widgets/game-score/usePreparedScore.ts': `export const usePreparedScore = (_runtime, track, hour) => ({ data: track ? { reply: globalThis.__scoreGap.reply, stretches: {} } : undefined,
      hour, pending: false, retry() {} });`,
    '/charts/table/Table.tsx': 'export default () => "score table"; export const CountryCell = () => null;',
  });
  try {
    const render = (hour: number) => { state.hour = hour; return renderToStaticMarkup(createElement(module.default, { widget: { id: 'score', kind: 'game-score',
      trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'follow' }, options: { view: 'table' } },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => hour }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps)); };
    assertNoMemberStates(render);
  } finally { delete host.__scoreGap; await close(); }
});
