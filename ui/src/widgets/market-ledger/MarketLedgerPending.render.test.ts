import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import type { CountrySelection } from '../../dashboard/model';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';

test('a market ledger whose group membership is still being prepared is loading, not a group with no members or no activity', async () => {
  const state = { groups: pendingGroups(), asked: null as unknown };
  const host = globalThis as typeof globalThis & { __ledgerPending?: typeof state }; host.__ledgerPending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/market-ledger/MarketLedgerWidget.tsx', {
    // The market was read through the branch and lists nothing for the members asked.
    '/widgets/market-ledger/useMarketLedger.ts': `export const useMarketLedger = (_runtime, _track, _active, _from, _hour, countries) => {
      globalThis.__ledgerPending.asked = countries;
      return { data: countries ? { lastHour: ${BRANCH.lastHour}, known: [[${BRANCH.firstHour}, ${BRANCH.lastHour}]] } : undefined,
        pages: [], errors: new Map(), warnings: [], identity: 'ledger', prefix: null, onWantedPages() {}, retry() {} }; };`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__ledgerPending.groups, retry() {} });',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => {};',
    '/charts/log/LogList.tsx': `import { createElement } from 'react'; export default ({ empty }) => createElement('p', null, empty);`,
    '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
    '/dashboard/MarketDrill.tsx': 'export default () => null;',
    '/charts/useChartTokens.ts': `export const useChartTokens = () => ({ categories: ['#123'] }); export const useViewportHeight = () => 800;`,
  });
  try {
    const props = (countries: CountrySelection) => ({ widget: { id: 'ledger', kind: 'market-ledger', trackIds: ['main'], countries, time: { mode: 'follow' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        client: {}, clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ playing: false }) } }, active: true, onDiagnostics() {} }) as unknown as WidgetProps;
    const render = (countries: CountrySelection) => renderToStaticMarkup(createElement(module.default, props(countries)));
    // Members at the end, with the end still being prepared: no member there yet is not none.
    state.groups = pendingGroups({ humans: [] });
    const end = render(humansBy('end'));
    assert.match(end, /aria-label="Loading market activity…"/);
    assert.doesNotMatch(end, /no members at the end/);
    state.groups = pendingGroups({ final: true, humans: [] });
    assert.match(render(humansBy('end')), /This group has no members at the end of this campaign\./);
    // A faction the prepared prefix does not name yet may still be founded.
    const later: CountrySelection = { kind: 'group', group: { type: 'faction', id: 'later' }, membership: 'hour', aggregate: false };
    state.groups = pendingGroups();
    assert.doesNotMatch(render(later), /unavailable in this campaign/);
    state.groups = pendingGroups({ final: true });
    assert.match(render(later), /This group is unavailable in this campaign\./);
    // GER is a member through 160, so the ledger is asked for it; activity after 160 is not known to be left out.
    state.groups = pendingGroups();
    const hourly = render(humansBy('hour'));
    assert.ok(state.asked, 'the request stays restricted to the members known so far');
    assert.match(hourly, /This range is still being prepared\./);
    assert.doesNotMatch(hourly, /No market activity matches/);
    state.groups = pendingGroups({ final: true });
    assert.match(render(humansBy('hour')), /No market activity matches these settings\./);
  } finally { await close(); delete host.__ledgerPending; }
});
