import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { MEMBERSHIP_PREPARING } from '../../dashboard/groups.ts';
import { BRANCH, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';
import type { CountryGroupReply } from '../../generated/dashboard';
import type { useResourceCountry as UseResourceCountry } from './useResourceBalance';

test('a resource breakdown chosen at an hour still being prepared is loading, not an unknown country', async () => {
  const state = { groups: pendingGroups() as CountryGroupReply };
  const host = globalThis as typeof globalThis & { __resourcePending?: typeof state }; host.__resourcePending = state;
  const { module, close } = await mountModule<{ useResourceCountry: typeof UseResourceCountry }>('/ui/src/widgets/resource-balance/useResourceBalance.ts', {
    '/dashboard/products.ts': 'export const useProduct = () => ({ retry() {} }); export const useProducts = () => ({ retry() {} });',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__resourcePending.groups, retry() {} });',
  });
  try {
    const track = { id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { campaignKey: 'campaign', countries: [] } }, branch: BRANCH } as unknown as BoundTrack;
    // GER was chosen at hour 170, which the groups reply has not read yet.
    const selection = { campaign: 'campaign', branch: BRANCH.id, tag: 'GER', originOffset: 170 };
    const Probe = () => { const actor = module.useResourceCountry(selection, { client: {} } as unknown as WidgetRuntime, track, true);
      return `${actor.preparing}|${actor.reason}`; };
    const render = () => renderToStaticMarkup(createElement(Probe));
    assert.equal(render(), `true|${MEMBERSHIP_PREPARING}`);
    // The final reply proves no country held GER then.
    state.groups = pendingGroups({ final: true, tags: ['ENG'] });
    assert.equal(render(), 'false|The selected country identity is unknown or conflicting at its original choice.');
  } finally { delete host.__resourcePending; await close(); }
});
