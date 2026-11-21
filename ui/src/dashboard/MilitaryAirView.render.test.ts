import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ComponentType } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { tokensFromValues } from '../charts/tokens.ts';
import type { WidgetProps } from './definitions';
import type { CountryGroupReply } from '../generated/dashboard';
import type { MilitaryAirLedger } from './militaryAirLedger';
import { validateMilitaryAirReply } from './militaryAirLedger.ts';
import { OPTIONS } from '../widgets/bombing-damage/definition.ts';
import { OPTIONS as AIR_OPTIONS } from '../widgets/air-kills-losses/definition.ts';
import { BRANCH, humansBy, mountModule, pendingGroups } from './pendingGroups.fixture.ts';
import { partOf } from './militaryAirParts.fixture.ts';
import type { Leaf } from './militaryAirParts.fixture';

test('a group whose members are read is a moment no chosen country was recorded, in both widgets', async () => {
  const span = { firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour };
  const data: MilitaryAirLedger = { family: 'bombing', ...span, available: true, observed: [span], incomplete: [],
    countries: [{ tag: 'ENG', identity: '1:102', present: [span], membership: null }],
    parts: ([{ tag: 'ENG', identity: '1:102', dealt: false, opponent: null, region: null, buildingId: 7, buildingToken: 'mod works', state: 1, cause: 0,
      points: [[10, 11, '50000000']] }] as Leaf[]).map(partOf), regions: [], buildings: [], warnings: [] };
  validateMilitaryAirReply(data);
  const track = { id: 'track', axisOffset: 0, source: { id: 'source', revision: 1, metadata: { countries: [], campaignKey: 'campaign' } }, branch: { ...BRANCH, gaps: [] } };
  const state = { data, track, groups: pendingGroups() as CountryGroupReply };
  const host = globalThis as typeof globalThis & { __airPending?: typeof state }; host.__airPending = state;
  const { module, close } = await mountModule<{ default: ComponentType<WidgetProps & { family: 'bombing'; fields: typeof OPTIONS; title: string }> }>('/ui/src/dashboard/MilitaryAirView.tsx', {
    '/dashboard/useMilitaryAirLedger.ts': 'export const useMilitaryAirLedger = () => ({ data: globalThis.__airPending.data, track: globalThis.__airPending.track, retry() {} });',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__airPending.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackMotion = () => 0;',
    '/dashboard/countryLooks.ts': 'export const useCountryLooks = () => ({ name: tag => tag, color: () => null, flag() {}, flagReason() {}, flagPending: () => false, revision: 0 });',
    '/charts/race/countryColours.ts': 'export const useCountryColours = () => () => "#777777";',
    '/charts/race/CountryRaceView.tsx': `import { createElement } from 'react'; export default props => props.state ?? (props.ranked.length ? null : createElement('p', null, props.empty));`,
    '/dashboard/BombingDrill.tsx': 'export default () => null;',
    '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
    '/charts/useChartTokens.ts': `export const useChartTokens = () => (${JSON.stringify(tokensFromValues(() => '#777', 'sans-serif'))});`,
    '/charts/butterfly/index.ts': 'export const ButterflyChart = () => null;',
  });
  try {
    const props = { widget: { id: 'widget', kind: 'bombing-damage', trackIds: ['track'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [track], client: {}, clock: { getHour: () => BRANCH.lastHour } }, active: true, onDiagnostics() {},
      family: 'bombing', fields: OPTIONS, title: 'Bombing damage' } as unknown as WidgetProps & { family: 'bombing'; fields: typeof OPTIONS; title: string };
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    state.groups = pendingGroups({ final: true, humans: [{ tag: 'GER', firstHour: 0, lastHour: 200 }] });
    assert.match(render(), /No selected country was recorded at this moment\./);
    state.data = { ...data, family: 'airLosses' };
    Object.assign(props, { family: 'airLosses', fields: AIR_OPTIONS, title: 'Air kills & losses' });
    assert.match(render(), /No selected country was recorded at this moment\./);
  } finally { delete host.__airPending; await close(); }
});
