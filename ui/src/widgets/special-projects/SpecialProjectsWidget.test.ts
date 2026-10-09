import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { CountryGroupReply, SpecialProjectsReply } from '../../generated/dashboard';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import type { TimelineProps } from '../../charts/timeline/Timeline';
import type { MenuItem } from '../../components/ContextMenu';

const groups: CountryGroupReply = { branchId: '1', humans: [], factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [], warnings: [],
  unobserved: { humans: [], factions: [], wars: [] }, countryIdentities: [
    { tag: 'X', identities: [{ firstHour: 100, lastHour: 109, slot: 1, countryId: 11 }, { firstHour: 120, lastHour: 300, slot: 2, countryId: 22 }] },
    { tag: 'Y', identities: [{ firstHour: 110, lastHour: 119, slot: 1, countryId: 11 }, { firstHour: 120, lastHour: 300, slot: 3, countryId: 33 }] },
  ] };

function reply(tag: 'X' | 'Y'): SpecialProjectsReply {
  const identities = groups.countryIdentities.find(history => history.tag === tag)!.identities;
  const episodes = identities.map((run, index) => ({ id: `${tag}-${index}`, start: { hour: run.firstHour, exact: false }, runs: [{
    firstHour: run.firstHour, lastHour: run.lastHour, progress: 10, presentation: index, state: 'running' as const, iterations: 0, completion: null, cancellation: null,
  }] }));
  return { branchId: '1', tag, identities, available: identities.map(({ firstHour, lastHour }) => ({ firstHour, lastHour })),
    readHours: identities.map(run => run.firstHour), projects: episodes, people: [{ assigned: false, id: null, name: null, portrait: null, missing: 'Unassigned' }],
    presentations: identities.map((_, index) => ({ name: tag === 'Y' && index === 1 ? 'Unrelated later project' : `${tag} project ${index}`,
      specialization: null, specializationName: null, scientist: 0, scientistAsOf: null, icon: null, iconMissing: 'Synthetic missing artwork' })),
    iterationTexts: { 0: '0 iterations' }, unavailable: null, warnings: [] };
}

type Request = { params: { tag: string; selectionTags: string[] } };
const host = globalThis as typeof globalThis & { __projectTimeline?: TimelineProps; __projectRequests?: Request[];
  __projectWanted?: { tag: string; hour: number }[]; __projectFlag?: ReturnType<CountryLabel>; __projectLabels?: [number, string][];
  __projectGroupReply?: CountryGroupReply; __projectReplies?: Record<string, SpecialProjectsReply>; __projectMenu?: readonly MenuItem[] };

test('actual SpecialProjects preserves literal roots, selected dated flag failures and growing held History clocks', async () => {
  const replacements: Record<string, string> = {
    '/dashboard/useCountryGroups.ts': `export const useCountryGroups = () => ({ data: globalThis.__projectGroupReply ?? ${JSON.stringify(groups)}, retry() {} });`,
    '/dashboard/products.ts': `export const useProducts = (_client, requests, _active, validate) => {
      globalThis.__projectRequests = requests; const replies = globalThis.__projectReplies ?? ${JSON.stringify({ X: reply('X'), Y: reply('Y') })};
      return { data: requests.map(request => validate(replies[request.params.tag])), retry() {} }; };`,
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = (_client, _track, wanted) => { globalThis.__projectWanted = wanted; return { labelAt: hour => tag => { globalThis.__projectLabels.push([hour, tag]); return globalThis.__projectFlag; } }; };',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => {};',
    '/charts/useChartTokens.ts': 'export const useViewportHeight = () => 1000;',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open: (_x, _y, items) => { globalThis.__projectMenu = items; }, menu: null });',
    '/charts/timeline/Timeline.tsx': 'export default props => { globalThis.__projectTimeline = props; return null; };',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'special-project-holder-fixture', enforce: 'pre',
      transform(_code, id) { return Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1]; } }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/special-projects/SpecialProjectsWidget.tsx');
    let hour = 150;
    const props = { widget: { id: 'projects', kind: 'special-projects', trackIds: ['main'], time: { mode: 'follow' },
      countries: { kind: 'tags', tags: ['X'] }, options: { view: 'history' } }, runtime: { tracks: [{ id: 'main', axisOffset: 0,
        source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: { id: '1', firstHour: 100, lastHour: 300 } }],
        clock: { getHour: () => hour, getSnapshot: () => ({ hoursPerSecond: 0 }) }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => { delete host.__projectTimeline; host.__projectLabels = []; return renderToStaticMarkup(createElement(Widget, props)); };
    const flags: ReturnType<CountryLabel>[] = [{ name: 'Selected actor', flagPending: true, flag: 'data:image/png;base64,synthetic' },
      { name: '', flagPending: false, flagReason: 'Native flag selection refused its supplying read.' }];
    for (const flag of flags) {
      host.__projectFlag = flag;
      render();
      assert.deepEqual(host.__projectRequests?.map(request => [request.params.tag, request.params.selectionTags]), [['X', ['X']], ['Y', ['X']]]);
      assert.deepEqual(host.__projectWanted, [{ tag: 'X', hour: 150 }, { tag: 'Y', hour: 119 }]);
      assert.deepEqual(host.__projectLabels, [[150, 'X'], [119, 'Y']], 'the earlier Y actor cannot borrow its later occupant');
      const chart = host.__projectTimeline!;
      assert.deepEqual(chart.extent, [100, 301]);
      assert.equal(chart.playhead, 150);
      assert.equal(chart.preserveZoomOnExtentChange, false);
      assert.ok(!chart.lanes.some(lane => lane.identity?.label === 'Unrelated later project'));
      const headings = chart.lanes.filter(lane => lane.heading);
      assert.equal(headings.length, 2);
      for (const heading of headings) assert.deepEqual(heading.identity?.countryFlag, { flag: flag.flag, flagPending: flag.flagPending, flagReason: flag.flagReason });
    }
    props.widget.options = { view: 'history', grow: true };
    render(); assert.deepEqual(host.__projectTimeline?.extent, [100, 150]);
    assert.equal(host.__projectTimeline?.preserveZoomOnExtentChange, true);
    hour = 160; render(); assert.deepEqual(host.__projectTimeline?.extent, [100, 160]);
    props.widget.time = { mode: 'pin', windows: { main: { firstOffset: 0, lastOffset: 50 } } };
    hour = 250; render(); assert.deepEqual(host.__projectTimeline?.extent, [100, 150]);
    assert.equal(host.__projectTimeline?.playhead, 150);
    props.widget.time = { mode: 'range', firstOffset: 20, lastOffset: 80 };
    render(); assert.deepEqual(host.__projectTimeline?.extent, [120, 181]);
    assert.equal(host.__projectTimeline?.playhead, null);
    props.widget.time = { mode: 'campaign' };
    render(); assert.deepEqual(host.__projectTimeline?.extent, [100, 301]);
    assert.equal(host.__projectTimeline?.playhead, null);

    // Native project observations exist throughout, while this group's roster
    // is unknown at 112..113. No future membership cut may expose a new lane.
    const identities = [{ firstHour: 100, lastHour: 300, slot: 1, countryId: 11 }];
    host.__projectGroupReply = { ...groups, humans: [{ tag: 'X', firstHour: 100, lastHour: 300 }],
      countryIdentities: [{ tag: 'X', identities }], unobserved: { ...groups.unobserved, humans: [{ firstHour: 112, lastHour: 113 }] } };
    const groupReply = reply('X');
    groupReply.identities = identities;
    groupReply.available = [{ firstHour: 100, lastHour: 300 }];
    groupReply.readHours = [100, 112];
    groupReply.projects = [
      { ...groupReply.projects[0], runs: [{ ...groupReply.projects[0].runs[0], lastHour: 300 }] },
      { ...groupReply.projects[1], id: 'fresh', start: { hour: 112, exact: false }, runs: [{ ...groupReply.projects[1].runs[0], firstHour: 112, lastHour: 300 }] },
    ];
    groupReply.presentations[1].name = 'First observed inside the membership gap';
    host.__projectReplies = { X: groupReply };
    props.widget.countries = { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false };
    props.widget.time = { mode: 'follow' };
    hour = 113;
    render();
    assert.deepEqual(host.__projectWanted, [{ tag: 'X', hour: 111 }], 'heading reads avoid the unknown membership date');
    const chart = host.__projectTimeline!;
    assert.equal(chart.lanes.length, 1, 'a new project seen only in unknown membership cannot become a future blueprint lane');
    assert.deepEqual(chart.lanes[0].spans.map(span => [span.firstHour, span.lastHour]), [[100, 111]]);
    assert.match(chart.tooltip?.(chart.lanes[0], null, 112)?.note ?? '', /not recorded at this hour/);
    delete host.__projectMenu;
    chart.onContextMenu?.(112, 0, 0, { ...chart.lanes[0], id: JSON.stringify([0, 'fresh']) });
    assert.equal(host.__projectMenu, undefined, 'unknown membership cannot create a new project menu');
    hour = 115;
    render();
    const fresh = host.__projectTimeline!.lanes.find(lane => lane.identity?.label === 'First observed inside the membership gap')!;
    assert.deepEqual(fresh.spans.map(span => [span.firstHour, span.lastHour]), [[114, 115]]);

    // An available empty project reply cannot prove no projects during an
    // unknown membership interval, even when other membership survives.
    const emptyIdentities = [{ firstHour: 100, lastHour: 150, slot: 1, countryId: 11 }];
    host.__projectGroupReply = { ...groups, countryIdentities: [{ tag: 'X', identities: emptyIdentities }], humans: [{ tag: 'X', firstHour: 100, lastHour: 150 }],
      unobserved: { ...groups.unobserved, humans: [{ firstHour: 112, lastHour: 113 }] } };
    host.__projectReplies = { X: { ...reply('X'), identities: emptyIdentities, available: [{ firstHour: 100, lastHour: 150 }], readHours: [100], projects: [], unavailable: null } };
    props.runtime.tracks[0].branch.lastHour = 150;
    const emptyMoments: { time: WidgetProps['widget']['time']; hour: number; unknown: boolean; headingHour: number }[] = [
      { time: { mode: 'follow' }, hour: 113, unknown: true, headingHour: 111 },
      { time: { mode: 'campaign' }, hour: 150, unknown: true, headingHour: 150 },
      { time: { mode: 'range', firstOffset: 20, lastOffset: 30 }, hour: 150, unknown: false, headingHour: 130 },
    ];
    for (const moment of emptyMoments) {
      props.widget.time = moment.time; props.widget.options = { view: 'history', grow: true }; hour = moment.hour;
      delete host.__projectMenu;
      const markup = render();
      if (moment.unknown) {
        assert.match(markup, /This group’s members were not recorded for part of this history/);
        assert.doesNotMatch(markup, /No selected countries|No special projects yet/);
      } else {
        assert.match(markup, /No special projects yet/);
        assert.doesNotMatch(markup, /This group’s members were not recorded/);
      }
      assert.deepEqual(host.__projectRequests?.map(request => [request.params.tag, request.params.selectionTags]), [['X', ['X']]], 'known membership retains only its selected native root');
      assert.deepEqual(host.__projectWanted, [{ tag: 'X', hour: moment.headingHour }], 'empty replies never request a masked membership date');
      assert.equal(host.__projectTimeline, undefined, 'empty project replies expose no History lanes');
      assert.equal(host.__projectMenu, undefined, 'empty project replies expose no project menus');
    }
    props.runtime.tracks[0].branch.lastHour = 300;
    host.__projectReplies = { X: groupReply };

    // A valid partial roster can have every observed member run inside its
    // missing coverage. Empty known membership must not prove an empty group.
    host.__projectGroupReply = { ...groups, countryIdentities: [{ tag: 'X', identities }], humans: [{ tag: 'X', firstHour: 100, lastHour: 150 }],
      unobserved: { ...groups.unobserved, humans: [{ firstHour: 100, lastHour: 150 }] } };
    const moments: { time: WidgetProps['widget']['time']; hour: number; unknownLive: boolean; unknownHistory: boolean }[] = [
      { time: { mode: 'follow' }, hour: 120, unknownLive: true, unknownHistory: true },
      { time: { mode: 'pin', windows: { main: { firstOffset: 0, lastOffset: 20 } } }, hour: 250, unknownLive: true, unknownHistory: true },
      { time: { mode: 'range', firstOffset: 10, lastOffset: 30 }, hour: 250, unknownLive: true, unknownHistory: true },
      { time: { mode: 'campaign' }, hour: 250, unknownLive: false, unknownHistory: true },
      { time: { mode: 'range', firstOffset: 60, lastOffset: 70 }, hour: 250, unknownLive: false, unknownHistory: false },
    ];
    for (const moment of moments) for (const view of ['live', 'history']) {
      props.widget.time = moment.time; props.widget.options = { view, grow: true }; hour = moment.hour;
      const markup = render();
      const unknown = view === 'live' ? moment.unknownLive : moment.unknownHistory;
      if (unknown) {
        assert.match(markup, /This group’s members were not recorded/);
        assert.doesNotMatch(markup, /No selected countries|No projects running|No special projects yet/);
      } else assert.match(markup, /No selected countries were recorded/);
      assert.deepEqual(host.__projectRequests, [], 'unknown members never become unrestricted product requests');
      assert.deepEqual(host.__projectWanted, [], 'no country heading borrows a masked membership date');
      assert.equal(host.__projectTimeline, undefined, 'masked membership exposes no History lanes or menus');
    }
  } finally {
    delete host.__projectTimeline; delete host.__projectRequests; delete host.__projectWanted; delete host.__projectFlag; delete host.__projectLabels;
    delete host.__projectGroupReply; delete host.__projectReplies; delete host.__projectMenu;
    await vite.close();
  }
});
