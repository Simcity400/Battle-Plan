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
import type { ReactElement } from 'react';

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
  __projectGroupReply?: CountryGroupReply; __projectReplies?: Record<string, SpecialProjectsReply>; __projectMenu?: readonly MenuItem[];
  __projectPending?: boolean; __projectGroupPending?: boolean; __projectErrors?: Record<string, string>; __projectGroupError?: string;
  __projectRetries?: string[]; __projectRetry?: () => void; __projectPortals?: ReactElement<{ onChange: (value: string) => void }>[];
  __projectKeys?: Record<string, (event: { key: string; shiftKey?: boolean; preventDefault: () => void; currentTarget: { getBoundingClientRect: () => { left: number; bottom: number } } }) => void> };

test('actual SpecialProjects preserves literal roots, selected dated flag failures and growing held History clocks', async () => {
  const replacements: Record<string, string> = {
    '/dashboard/useCountryGroups.ts': `export const useCountryGroups = () => ({ data: globalThis.__projectGroupPending ? undefined : globalThis.__projectGroupReply ?? ${JSON.stringify(groups)}, error: globalThis.__projectGroupError, retry() { globalThis.__projectRetries.push('groups'); } });`,
    '/dashboard/products.ts': `export const useProducts = (_client, requests, _active, validate) => {
      globalThis.__projectRequests = requests; const replies = globalThis.__projectReplies ?? ${JSON.stringify({ X: reply('X'), Y: reply('Y') })};
      const failed = requests.find(request => globalThis.__projectErrors?.[request.params.tag]);
      return { data: globalThis.__projectPending || failed ? undefined : requests.map(request => validate(replies[request.params.tag])),
        error: failed && globalThis.__projectErrors[failed.params.tag], retry() { globalThis.__projectRetries.push(failed?.params.tag ?? 'projects'); } }; };`,
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = (_client, _track, wanted) => { globalThis.__projectWanted = wanted; return { labelAt: hour => tag => { globalThis.__projectLabels.push([hour, tag]); return globalThis.__projectFlag; } }; };',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => {};',
    '/charts/useChartTokens.ts': 'export const useViewportHeight = () => 1000;',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open: (_x, _y, items) => { globalThis.__projectMenu = items; }, menu: null });',
    '/charts/timeline/Timeline.tsx': 'export default props => { globalThis.__projectTimeline = props; return null; };',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'special-project-holder-fixture', enforce: 'pre',
      transform(code, id) {
        const path = id.replaceAll('\\', '/');
        const replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix))?.[1];
        if (replacement) return replacement;
        if (path.endsWith('/special-projects/SpecialProjectsWidget.tsx')) return code
          .replace("import { createPortal } from 'react-dom';", 'const createPortal = (child) => { globalThis.__projectPortals.push(child); return child; };')
          .replace('onKeyDown={event =>', 'onKeyDown={globalThis.__projectKeys[project.id] = event =>');
        if (path.endsWith('/components/WidgetState.tsx')) return code.replace('onClick={props.onRetry}', 'onClick={globalThis.__projectRetry = props.onRetry}');
      } }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/special-projects/SpecialProjectsWidget.tsx');
    let hour = 150;
    const props = { widget: { id: 'projects', kind: 'special-projects', trackIds: ['main'], time: { mode: 'follow' },
      countries: { kind: 'tags', tags: ['X'] }, options: { view: 'history' } }, runtime: { tracks: [{ id: 'main', axisOffset: 0,
        source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: { id: '1', firstHour: 100, lastHour: 300 } }],
        clock: { getHour: () => hour, getSnapshot: () => ({ hoursPerSecond: 0 }) }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const seeks: [string, number][] = [];
    props.runtime.requestSeek = (track, at) => { seeks.push([track, at]); return true; };
    host.__projectRetries = [];
    const render = () => {
      delete host.__projectTimeline; delete host.__projectRetry;
      host.__projectLabels = []; host.__projectPortals = []; host.__projectKeys = {};
      return renderToStaticMarkup(createElement(Widget, props));
    };
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
    props.widget.options = { view: 'live' };
    const beforeCountries = props.widget.countries;
    props.widget.countries = { kind: 'tags', tags: ['X', 'Y'] };
    props.headerSlot = {} as HTMLElement;
    props.setOption = (key, value) => { props.widget.options = { ...props.widget.options, [key]: value }; };
    const current = render();
    assert.match(current, /X project 1/);
    assert.match(current, /Unrelated later project/);
    assert.doesNotMatch(current, /First country shown|Synthetic missing artwork|ProjectHistory/);
    assert.match(current, /art-fallback/);
    assert.equal(host.__projectPortals?.length, 1, 'one header selector serves every panel');
    assert.match(current, /Current/);
    assert.doesNotMatch(current, />Live</);
    for (const [id, expected] of [['X-1', 120], ['Y-1', 120]] as const) {
      let prevented = false;
      host.__projectKeys![id]({ key: 'F10', shiftKey: true, preventDefault() { prevented = true; }, currentTarget: { getBoundingClientRect: () => ({ left: 0, bottom: 0 }) } });
      assert.equal(prevented, true);
      assert.match(host.__projectMenu![0].label, /Jump to start · by /);
      assert.equal(host.__projectMenu?.length, 1, 'running cards have no completion action');
      host.__projectMenu![0].run!();
      assert.deepEqual(seeks.at(-1), ['main', expected]);
    }
    host.__projectPortals![0].props.onChange('history');
    assert.equal(props.widget.options?.view, 'history');
    render(); assert.equal(host.__projectPortals?.length, 1);
    host.__projectPortals![0].props.onChange('live');

    host.__projectPending = true;
    assert.match(render(), /Loading special projects/);
    assert.equal(host.__projectPortals?.length, 1);
    delete host.__projectPending;
    host.__projectErrors = { Y: 'Synthetic malformed backend reason' };
    const partlyFailed = render();
    assert.match(partlyFailed, /X project 1/);
    assert.match(partlyFailed, /Couldn’t load\./);
    assert.match(partlyFailed, />Retry</);
    assert.doesNotMatch(partlyFailed.replace(/<[^>]*>/g, ' '), /malformed backend/);
    host.__projectRetry!(); assert.equal(host.__projectRetries.at(-1), 'Y');
    delete host.__projectErrors;
    host.__projectGroupPending = true;
    assert.match(render(), /Loading countries/);
    assert.equal(host.__projectPortals?.length, 1);
    delete host.__projectGroupPending;
    host.__projectGroupError = 'Synthetic group failure';
    assert.match(render(), />Retry</);
    host.__projectRetry!(); assert.equal(host.__projectRetries.at(-1), 'groups');
    delete host.__projectGroupError;
    const unavailable = { ...reply('Y'), unavailable: 'ProjectPrograms were never captured', available: [], readHours: [], projects: [] };
    host.__projectReplies = { X: reply('X'), Y: unavailable };
    assert.match(render(), /Projects were not recorded\./);
    assert.match(render(), /X project 1/);
    host.__projectReplies = { X: reply('X'), Y: { ...reply('Y'), projects: [] } };
    assert.match(render(), /No projects running\./);
    host.__projectReplies = { X: reply('X'), Y: { ...reply('Y'), available: [], readHours: [], projects: [] } };
    assert.match(render(), /Projects were not recorded at this time\./);
    const completed = reply('Y');
    Object.assign(completed.projects[1].runs[0], { firstHour: 130, progress: 100, state: 'completed', completion: { hour: 130, exact: true } });
    host.__projectReplies = { X: reply('X'), Y: completed };
    assert.match(render(), /completed/);
    host.__projectKeys!['Y-1']({ key: 'ContextMenu', preventDefault() {}, currentTarget: { getBoundingClientRect: () => ({ left: 0, bottom: 0 }) } });
    assert.equal(host.__projectMenu?.length, 2);
    assert.match(host.__projectMenu![1].label, /Jump to completion · /);
    host.__projectMenu![1].run!(); assert.deepEqual(seeks.at(-1), ['main', 130]);
    delete host.__projectReplies;
    hour = 301;
    assert.match(render(), /This time is outside the recording/);
    assert.equal(host.__projectPortals?.length, 1);
    hour = 150;
    props.widget.countries = beforeCountries;
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
    delete host.__projectPending; delete host.__projectGroupPending; delete host.__projectGroupError; delete host.__projectErrors;
    delete host.__projectRetries; delete host.__projectRetry; delete host.__projectPortals; delete host.__projectKeys;
    await vite.close();
  }
});

test('Current comparison props remain stable during playback and wrapper hooks keep their order across views', async () => {
  const slots: { deps: readonly unknown[]; value: unknown }[] = [];
  let cursor = 0;
  const state = { groups, memo: (factory: () => unknown, deps: readonly unknown[]) => {
    const at = cursor++, held = slots[at];
    if (!held || deps.length !== held.deps.length || deps.some((value, index) => !Object.is(value, held.deps[index])))
      slots[at] = { deps, value: factory() };
    return slots[at].value;
  } };
  const global = globalThis as typeof globalThis & { __projectStable?: typeof state };
  global.__projectStable = state;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'project-comparison-lifetime', enforce: 'pre',
      transform(code, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/dashboard/useCountryGroups.ts')) return 'export const useCountryGroups = () => ({ data: globalThis.__projectStable.groups });';
        if (path.endsWith('/dashboard/playback.ts')) return 'export const usePlaybackHour = () => {};';
        if (path.endsWith('/special-projects/SpecialProjectsWidget.tsx')) return code.replaceAll('useMemo(', 'globalThis.__projectStable.memo(');
      } }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/special-projects/SpecialProjectsWidget.tsx') as { default: (props: WidgetProps) => ReactElement };
    let hour = 150;
    const props = { widget: { id: 'projects', kind: 'special-projects', trackIds: ['main'], countries: { kind: 'tags', tags: ['X', 'Y'] },
      time: { mode: 'follow' }, options: { view: 'live' } }, runtime: { tracks: [{ id: 'main', axisOffset: 0,
        source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: { id: '1', firstHour: 100, lastHour: 300 } }],
        clock: { getHour: () => hour }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => { cursor = 0; const tree = Widget(props); assert.equal(cursor, 2); return tree.props as { children: ReactElement[] }; };
    const first = render().children[1].props as { widget: WidgetProps['widget']; headerSlot: unknown };
    assert.equal(first.headerSlot, null);
    assert.deepEqual(first.widget.countries, { kind: 'tags', tags: ['X', 'Y'] });
    hour = 151;
    assert.equal((render().children[1].props as typeof first).widget, first.widget, 'clock changes cannot reset CountryComparisons scroll or panel state');
    props.widget.options = { view: 'history' }; render();
    props.widget.options = { view: 'live' }; render();
    const heldGroups = state.groups;
    state.groups = undefined as unknown as CountryGroupReply; render();
    state.groups = heldGroups; render();
  } finally { delete global.__projectStable; await vite.close(); }
});
