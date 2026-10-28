import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';
import type { CountryGroupReply } from '../../generated/dashboard';

test('opinions of a group at an hour still being prepared are loading, not a choice of too few countries', async () => {
  const state = { groups: pendingGroups() as CountryGroupReply };
  const host = globalThis as typeof globalThis & { __relationsPending?: typeof state }; host.__relationsPending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/diplomatic-relations/DiplomaticRelationsWidget.tsx', {
    '/dashboard/products.ts': 'export const useProduct = () => ({ retry() {} }); export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__relationsPending.groups, retry() {} });',
    '/charts/table/useTableHour.ts': `export const useTableHour = () => ${BRANCH.lastHour};`,
    '/charts/useChartTokens.ts': "export const useChartTokens = () => ({ surface: '#000', text: '#fff', error: '#f00', positive: '#0f0' });",
  });
  try {
    const props = { widget: { id: 'relations', kind: 'diplomatic-relations', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => BRANCH.lastHour }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // The branch end is still being prepared: its human players are not known yet.
    const pending = render();
    assert.match(pending, /aria-label="This range is still being prepared\."/);
    assert.doesNotMatch(pending, /Choose at least two countries/);
    // The final reply proves no human player at the end.
    state.groups = pendingGroups({ final: true });
    assert.match(render(), /Choose at least two countries\./);
  } finally { delete host.__relationsPending; await close(); }
});

test('opinions of a group with no member found yet say what the hour was once read, and load until then', async () => {
  const state = { groups: { ...pendingGroups({ humans: [] }), unobserved: { humans: [{ firstHour: 140, lastHour: 150 }], factions: [], wars: [] } } as CountryGroupReply, hour: 0 };
  const host = globalThis as typeof globalThis & { __relationsGap?: typeof state }; host.__relationsGap = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/diplomatic-relations/DiplomaticRelationsWidget.tsx', {
    // No request, no reply.
    '/dashboard/products.ts': 'export const useProduct = (_client, request) => ({ data: request ? { owners: [], opinions: [], warnings: [], unresolved: 0, hour: 0 } : undefined, retry() {} }); export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__relationsGap.groups, retry() {} });',
    '/charts/table/useTableHour.ts': 'export const useTableHour = () => globalThis.__relationsGap.hour;',
    '/charts/useChartTokens.ts': "export const useChartTokens = () => ({ surface: '#000', text: '#fff', error: '#f00', positive: '#0f0' });",
  });
  try {
    const render = (hour: number) => { state.hour = hour; return renderToStaticMarkup(createElement(module.default, { widget: { id: 'relations', kind: 'diplomatic-relations',
      trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'follow' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => hour }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps)); };
    assert.match(render(145), /Group membership is unavailable at this time\./);
    assert.match(render(100), /This group has no members at this moment\./);
    const later = render(170);
    assert.match(later, /aria-label="This range is still being prepared\."/);
    assert.doesNotMatch(later, /unavailable|no members|Choose at least/);
  } finally { delete host.__relationsGap; await close(); }
});
