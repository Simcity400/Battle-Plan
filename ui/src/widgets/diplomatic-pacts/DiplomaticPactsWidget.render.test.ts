import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';
import type { CountryGroupReply, DiplomacyPactsReply } from '../../generated/dashboard';

test('pacts of a group whose members are still being prepared are loading, not none matching', async () => {
  const reply: DiplomacyPactsReply = { kinds: [], pacts: [], reads: [], unobserved: [], warnings: [], pending: [],
    coverage: { firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour, complete: true } } as unknown as DiplomacyPactsReply;
  const state = { groups: pendingGroups() as CountryGroupReply };
  const host = globalThis as typeof globalThis & { __pactsPending?: typeof state }; host.__pactsPending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/diplomatic-pacts/DiplomaticPactsWidget.tsx', {
    '/dashboard/products.ts': `export const useProduct = (_client, request) => ({ data: request ? ${JSON.stringify(reply)} : undefined, retry() {} });`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__pactsPending.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackSpeed = () => 0;',
    '/charts/table/useTableHour.ts': `export const useTableHour = () => ${BRANCH.lastHour};`,
    '/charts/useChartTokens.ts': 'export const useViewportHeight = () => 1000;',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag, known: true }) });',
  });
  try {
    const props = { widget: { id: 'pacts', kind: 'diplomatic-pacts', trackIds: ['main'], countries: humansBy('ever'), time: { mode: 'campaign' }, options: { view: 'pacts' } },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => BRANCH.lastHour }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // Anyone ever a human player is known only with the whole branch: the later hours are still being prepared.
    const pending = render();
    assert.match(pending, /aria-label="This range is still being prepared\."/);
    assert.doesNotMatch(pending, /No pacts match/);
    // The final reply proves GER, the only human player ever, had no pact.
    state.groups = pendingGroups({ final: true });
    assert.match(render(), /No pacts match these settings\./);
  } finally { delete host.__pactsPending; await close(); }
});
