import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';

test('an empty event feed whose group hours are still being prepared is loading, not a log with no matches', async () => {
  const state: { groups: ReturnType<typeof pendingGroups>; prefix: unknown } = { groups: pendingGroups(), prefix: null };
  const host = globalThis as typeof globalThis & { __feedPending?: typeof state }; host.__feedPending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/event-feed/EventFeedWidget.tsx', {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__feedPending.groups });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({ categories: ["#123456"] });',
    '/dashboard/useStateNamesAt.ts': 'export const useStateNamesAt = () => () => ({ name: "State" });',
    '/event-feed/useEventFeed.ts': `export const useEventFeed = () => ({ data: { unavailable: [] }, pages: [], warnings: [], replies: [],
      errors: new Map(), total: null, identity: 'event-feed', prefix: globalThis.__feedPending.prefix, onWantedPages() {}, retryPages() {} });`,
    '/charts/log/LogList.tsx': `import { createElement } from "react"; import { preparedRangeText } from "../prepared.ts";
      export default props => createElement("p", {}, preparedRangeText(props.preparedCoverage), props.empty);`,
  });
  try {
    const props = { widget: { id: 'event-feed', kind: 'event-feed', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: { ...BRANCH, gaps: [] } }],
        client: {}, clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ hoursPerSecond: 0 }) } }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    const pending = render();
    assert.match(pending, /This range is still being prepared\./);
    assert.doesNotMatch(pending, /No recorded events/);
    state.groups = pendingGroups({ final: true });
    assert.match(render(), /No recorded events match these settings\./);
    // The feed itself not prepared through the hour shown: still being prepared, said once, never no matches.
    state.prefix = { coverage: { firstHour: BRANCH.firstHour, lastHour: BRANCH.firstHour - 1, complete: false }, pending: [{ firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour }] };
    const unprepared = render();
    assert.equal(unprepared.match(/still being prepared/g)?.length, 1);
    assert.doesNotMatch(unprepared, /No recorded events/);
  } finally { delete host.__feedPending; await close(); }
});
