import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import type { VolunteersExpeditionariesReply } from '../../generated/dashboard';
import { BRANCH, KNOWN_HOUR, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';

test('members ever, known so far but not yet all found, leave a moment with no volunteers loading, not empty', async () => {
  // ENG sends volunteers at the prepared hour 150; whether ENG is ever a human player is known only once the whole branch is.
  const reply = { tallies: [], unobserved: [], warnings: [], links: [{ key: 'ef', sender: 'ENG', senderSlot: 2, senderCountryId: 102, host: 'FRA', hostSlot: 3,
    hostCountryId: 103, runs: [{ firstHour: 140, lastHour: 160, divisions: 2, startsSegment: false }] }] } as unknown as VolunteersExpeditionariesReply;
  const state = { groups: pendingGroups(), reply };
  const host = globalThis as typeof globalThis & { __volunteersPending?: typeof state }; host.__volunteersPending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/volunteers/VolunteersWidget.tsx', {
    '/dashboard/products.ts': 'export const useProduct = (_client, request) => ({ data: request ? globalThis.__volunteersPending.reply : undefined, retry() {} }); export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__volunteersPending.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour(); export const usePlaybackMotion = () => 0;',
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });
      export const flagReadSpec = () => ({}); export const flagReadMetadata = () => null;`,
    '/dashboard/countryNamesAt.ts': 'export const nameKey = (hour, tag) => JSON.stringify([hour, tag]); export const useCountryNamesRead = () => new Map();',
    '/dashboard/holderLooks.ts': 'export const nameReadText = () => "Country"; export const useHolderLooks = () => ({ color: () => "#123456" });',
    '/charts/useChartTokens.ts': `export const useChartTokens = () => ({ categories: ['#123'], muted: '#888', surface: '#000', text: '#fff' });`,
    '/charts/flow/FlowDiagram.tsx': `import { createElement } from 'react'; export default ({ message }) => createElement('p', null, message ?? 'links drawn');`,
    '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ menu: null, open() {} });',
  });
  try {
    const props = { widget: { id: 'volunteers', kind: 'volunteers', trackIds: ['main'], countries: humansBy('ever'), time: { mode: 'follow' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: { ...BRANCH, gaps: [] } }],
        client: {}, clock: { getHour: () => KNOWN_HOUR, getSnapshot: () => ({ playing: false, hoursPerSecond: 0 }) } }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // GER, a member already found, makes the request; ENG may still be found later.
    const pending = render();
    assert.match(pending, /This range is still being prepared\./);
    assert.doesNotMatch(pending, /No volunteers abroad/);
    // The final reply proves ENG never was a human player.
    state.groups = pendingGroups({ final: true });
    assert.match(render(), /No volunteers abroad at this date\./);
    // Found a member later, its volunteers at 150 show.
    state.groups = pendingGroups({ final: true, humans: [{ tag: 'GER', firstHour: 0, lastHour: 160 }, { tag: 'ENG', firstHour: 180, lastHour: 200 }] });
    assert.match(render(), /links drawn/);
  } finally { await close(); delete host.__volunteersPending; }
});
