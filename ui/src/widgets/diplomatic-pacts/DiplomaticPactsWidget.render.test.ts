import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import { BRANCH, mountModule } from '../../dashboard/pendingGroups.fixture.ts';
import type { DiplomacyPactsReply } from '../../generated/dashboard';

test('one country\'s agreements: still preparing is loading, none is said, and each agreement is a lane under its kind', async () => {
  const reply = (pacts: unknown[], pending: { firstHour: number; lastHour: number }[]) => ({ kinds: [], pacts, reads: [], unobserved: [], warnings: [], pending,
    coverage: { firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour, complete: !pending.length } }) as unknown as DiplomacyPactsReply;
  const state = { reply: reply([], [{ firstHour: 100, lastHour: BRANCH.lastHour }]), requests: [] as unknown[], lanes: [] as { identity?: { label: string } }[] };
  const host = globalThis as typeof globalThis & { __agreements?: typeof state }; host.__agreements = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/diplomatic-pacts/DiplomaticPactsWidget.tsx', {
    '/dashboard/products.ts': 'export const useProduct = (_client, request) => { if (request) globalThis.__agreements.requests.push(request); return { data: request ? globalThis.__agreements.reply : undefined, retry() {} }; }; export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({ categories: ["#112233"] });',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: `Country ${tag}`, known: true }) });',
    '/charts/timeline/Timeline.tsx': 'export default props => { globalThis.__agreements.lanes = props.lanes; return null; };',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
  });
  try {
    const props = { widget: { id: 'pacts', kind: 'diplomatic-pacts', trackIds: ['main'], countries: { kind: 'tags', tags: ['GER'] }, time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, selectedTag: null, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ hoursPerSecond: 0 }) }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    assert.match(render(), /aria-label="This range is still being prepared\."/);
    assert.deepEqual((state.requests[0] as { params: unknown }).params, { view: 'pacts', branchId: '1', tags: ['GER'] }, 'only the chosen country is read');
    state.reply = reply([], []);
    assert.match(render(), /Country GER has no recorded agreements at this time\./);
    const owner = { tag: 'GER', slot: 1, countryId: 1 };
    state.reply = reply([{ id: '1', kind: 'guarantee', owner: 'GER', target: 'TUR', first: 'GER', second: 'TUR', identities: [owner], rawCountries: [], ordinal: 0,
      startHour: 10, endHour: null, cancelled: null, retired: null, cancelRaw: 0, retiredRaw: 0, firstSeen: 10, lastSeen: 200, presentAtEnd: true, endedBy: false, readHour: 10 }], []);
    render();
    assert.deepEqual(state.lanes.map(lane => lane.identity?.label), ['Guarantees · 1 in force', 'Country GER guarantees', 'Country TUR']);
  } finally { delete host.__agreements; await close(); }
});
