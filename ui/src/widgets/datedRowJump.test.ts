import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactNode } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { MenuItem } from '../components/ContextMenu';
import type { WidgetProps } from '../dashboard/definitions';
import type { FocusReply, ResearchReply } from '../generated/dashboard';
import { formatDate, gameDate, HOUR_EPOCH } from '../dashboard/calendar.ts';

type Node = { type: unknown; props: Record<string, unknown> };
const nodes = (tree: unknown): Node[] => tree && typeof tree === 'object' && 'type' in tree && typeof (tree as Node).type === 'function' && ((tree as Node).type as Function).name === 'VirtualList'
  ? nodes(((tree as Node).type as Function)((tree as Node).props)) : Array.isArray(tree) ? tree.flatMap(nodes)
  : tree && typeof tree === 'object' && 'props' in tree ? [tree as Node, ...nodes((tree as Node).props.children)] : [];

test('registered Focus and Research rows Jump to their recorded bounds through pointer and menu keys', async () => {
  const first = HOUR_EPOCH + 100, focusHour = first + 25, ended = first + 48, confirmed = first + 60, last = first + 100;
  const focus: FocusReply = { history: { segments: [{ firstHour: first, lastHour: focusHour - 1, order: {
    completed: [{ ordinal: 0, token: 'already_done', completedByHour: null }], current: 'new_done', continuous: null, paused: false,
  } }, { firstHour: focusHour, lastHour: last, order: {
    completed: [{ ordinal: 0, token: 'already_done', completedByHour: null }, { ordinal: 1, token: 'new_done', completedByHour: focusHour }],
    current: 'unfinished', continuous: null, paused: false,
  } }], progress: [] }, artwork: [], continuousStaticText: [], warnings: [] };
  const research: ResearchReply = { history: { coverage: [{ firstHour: first, lastHour: last }], slots: [{ index: 0,
    availability: [{ firstHour: first, lastHour: last }], states: [], progress: [], entries: [{ token: 'technology', technologyHandle: 7, databaseIndex: 7,
      continuityStartHour: first, level: 0, firstObservedHour: first + 10, startedAfterHour: first + 9,
      lastObservedHour: ended - 1, endedByHour: ended, outcomeObservedHour: confirmed, outcome: 'completed' }],
  }] }, artwork: [], folders: [], technologyFolders: [], warnings: [] };
  const host = globalThis as typeof globalThis & { __datedRowData?: FocusReply | ResearchReply;
    __datedRowMenu?: { x: number; y: number; items: readonly MenuItem[] }; __datedRowKey?: string };
  const shownMenu = () => host.__datedRowMenu;
  const replacements: Record<string, string> = {
    '/components/VirtualList.tsx': `import { createElement } from 'react'; export const VirtualList = ({ items, itemKey, rowProps, children }) => {
      globalThis.__datedRowKey = items.length ? itemKey(items[0]) : undefined;
      return items.map((item, index) => createElement('li', { ...rowProps?.(item, index), key: index }, children(item, index))); };`,
    '/research/preparedResearch.ts': 'export const usePreparedResearch = () => ({ data: globalThis.__datedRowData });',
    '/dashboard/products.ts': 'export const useProduct = () => ({ data: globalThis.__datedRowData }); export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    // Server rendering has no document to read theme colours from.
    '/charts/useChartTokens.ts': "export const useChartTokens = () => ({ categories: ['#3987e5'] }); export const useViewportHeight = () => 800;",
    '/national-focus/useFocusArtwork.ts': 'export const useFocusArtwork = () => ({ read: new Map(), warnings: [] }); export const useContinuousFocusArtwork = () => ({ read: new Map(), warnings: [], details: new Map() });',
    '/components/ContextMenu.tsx': `import { createElement } from 'react'; export const useContextMenu = label => ({
      menu: createElement('span', { 'data-row-menu': label }),
      open: (x, y, items) => { globalThis.__datedRowMenu = { x, y, items }; } });`,
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'dated-row-menu-fixture', enforce: 'pre',
      transform(_source, id) { return Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1]; } }] });
  try {
    type Renderer = { render: (props: WidgetProps) => ReactNode };
    const focusRenderer = (await vite.ssrLoadModule('/ui/src/widgets/national-focus/index.ts')).default as Renderer;
    const { LegacyNationalFocusWidget: focusOracle } = await vite.ssrLoadModule('/ui/src/widgets/national-focus/NationalFocusWidget.tsx');
    const researchRenderer = (await vite.ssrLoadModule('/ui/src/widgets/research/index.ts')).default as Renderer;
    const seeks: [string, number][] = [];
    const track = { id: 'track', axisOffset: 12, branch: { id: 'branch', firstHour: first, lastHour: last, gaps: [] },
      source: { id: 'source', revision: '1' } };
    let hour = last, tree: ReactNode = null;
    const props = { widget: { kind: 'national-focus', trackIds: ['track'], countries: { kind: 'tags', tags: ['synthetic'] }, time: { mode: 'follow' } },
      runtime: { client: {}, tracks: [track], clock: { getHour: () => hour - track.axisOffset }, requestSeek: (id: string, at: number) => seeks.push([id, at]) },
      active: true, onDiagnostics() {} } as unknown as WidgetProps;
    let renderer = focusRenderer;
    function Probe() {
      if (renderer === focusRenderer) tree = focusOracle(props);
      else {
        // The registered research view composes its body with the doctrine
        // panel. Probe row behavior inside that body; panel composition has
        // its own render test.
        const view = renderer.render(props) as Node;
        assert.equal(typeof view.props.body, 'function');
        tree = (view.props.body as (props: WidgetProps) => ReactNode)(props);
      }
      return tree;
    }
    const render = () => renderToStaticMarkup(createElement(Probe));
    const activate = (row: Node, expected: number) => {
      const pointer = row.props.onContextMenu as (event: unknown) => void;
      const keyboard = row.props.onKeyDown as (event: unknown) => void;
      assert.equal(typeof pointer, 'function'); assert.equal(typeof keyboard, 'function');
      let prevented = 0, stopped = 0;
      const event = { clientX: 31, clientY: 42, currentTarget: { getBoundingClientRect: () => ({ left: 10, bottom: 20 }) },
        preventDefault() { prevented++; }, stopPropagation() { stopped++; } };
      pointer(event);
      assert.deepEqual([shownMenu()?.x, shownMenu()?.y], [31, 42]);
      assert.equal(shownMenu()?.items[0].disabled, false);
      assert.equal(shownMenu()?.items[0].label, `Jump to ${formatDate(gameDate(expected)!)}`);
      shownMenu()!.items[0].run(); assert.deepEqual(seeks.at(-1), ['track', expected]);
      for (const [key, shiftKey] of [['ContextMenu', false], ['F10', true]] as const) {
        delete host.__datedRowMenu; keyboard({ ...event, key, shiftKey });
        assert.deepEqual([shownMenu()?.x, shownMenu()?.y], [10, 20]);
        shownMenu()!.items[0].run(); assert.deepEqual(seeks.at(-1), ['track', expected]);
      }
      assert.equal(prevented, 3); assert.equal(stopped, 3);
      for (const key of ['Enter', ' ', 'F10', 'ArrowDown']) {
        delete host.__datedRowMenu; keyboard({ ...event, key, shiftKey: false });
        assert.equal(shownMenu(), undefined, `${key} retains its native action`);
      }
      assert.equal(prevented, 3);
    };
    host.__datedRowData = focus;
    hour = focusHour - 1; render();
    const undatedKey = (globalThis as typeof globalThis & { __datedRowKey?: string }).__datedRowKey;
    assert.ok(nodes(tree).filter(node => node.type === 'li').every(node => node.props.onContextMenu === undefined), 'a completion is not dated before its first observation');
    hour = last;
    const html = render();
    assert.notEqual((globalThis as typeof globalThis & { __datedRowKey?: string }).__datedRowKey, undatedKey, 'undated row expansion belongs to its supplying artwork read');
    assert.match(html, /data-row-menu="National focus actions"/);
    assert.ok(html.includes('data-tooltip="Completed by this date. Focuses are read once a game day, so it may have finished a day or so earlier.">'
      + `${formatDate(gameDate(focusHour)!)}</span>`), 'a dated completion shows its plain date with the daily-read caveat in its hint');
    assert.ok(!html.includes(`By ${formatDate(gameDate(focusHour)!)}`));
    const focusRows = nodes(tree).filter(node => node.type === 'li');
    assert.equal(focusRows.length, 2);
    assert.equal(focusRows[0].props.onContextMenu, undefined, 'an artwork snapshot does not date an earlier completion');
    assert.equal(focusRows[0].props.onKeyDown, undefined);
    activate(focusRows[1], focusHour);
    props.widget.time = { mode: 'campaign' }; render();
    assert.equal(nodes(tree).find(node => node.props.className === 'focus-entry is-unfinished')?.props.onContextMenu, undefined);

    renderer = researchRenderer; props.widget.kind = 'research'; props.widget.time = { mode: 'follow' };
    host.__datedRowData = research;
    type Jump = (at: number | (() => number | null)) => { onContextMenu: (event: unknown) => void };
    const columnsTree = () => {
      const view = nodes(tree).find(node => Array.isArray(node.props.lanes) && 'moment' in node.props);
      if (!view) return [];
      let columns: ReactNode = null;
      function ColumnsProbe() { columns = (view!.type as (props: Record<string, unknown>) => ReactNode)(view!.props); return columns; }
      renderToStaticMarkup(createElement(ColumnsProbe));
      return nodes(columns);
    };
    const completedCell = () => columnsTree().find(node => (node.props.item as { kind?: string } | undefined)?.kind === 'completed');
    hour = confirmed - 1; render();
    assert.equal(completedCell(), undefined, 'a later completion classification cannot appear early');
    hour = confirmed;
    assert.match(render(), /data-row-menu="Research actions"/);
    const cell = completedCell()!;
    let cellTree: ReactNode = null;
    function CellProbe() { cellTree = (cell.type as (props: Record<string, unknown>) => ReactNode)(cell.props); return cellTree; }
    renderToStaticMarkup(createElement(CellProbe));
    activate(nodes(cellTree).find(node => node.type === 'button')!, ended);
    const beforeOutside = seeks.length;
    track.branch.firstHour = ended + 1; render();
    const outside = completedCell()!;
    (outside.props.jump as Jump)((outside.props.item as { hour: number }).hour)
      .onContextMenu({ clientX: 0, clientY: 0, preventDefault() {}, stopPropagation() {} });
    assert.equal(shownMenu()?.items[0].disabled, true);
    assert.equal(seeks.length, beforeOutside, 'opening a disabled Jump never seeks');

    track.branch.firstHour = first; hour = last;
    const { default: Explanation } = await vite.ssrLoadModule('/ui/src/widgets/research/ResearchExplanation.tsx');
    for (const startedAfterHour of [null, first + 9]) {
      const earlier = { ...research.history.slots[0].entries[0], startedAfterHour, lastObservedHour: first + 29,
        endedByHour: first + 30, outcomeObservedHour: null, outcome: 'switched' as const };
      const ongoing: ResearchReply = { ...research, history: { ...research.history, slots: [
        { ...research.history.slots[0], entries: [earlier] },
        { index: 1, availability: [{ firstHour: first + 30, lastHour: last }], progress: [], entries: [{ ...earlier,
          firstObservedHour: first + 30, startedAfterHour: first + 29, lastObservedHour: last, endedByHour: null, outcome: 'active' }],
        states: [{ firstHour: first + 30, lastHour: last, token: earlier.token, entryFirstObservedHour: first + 30,
          speedMultiplierRaw: '100000', bankedDaysRaw: '0', appliedDaysRaw: '0', aheadReductionYearsRaw: '0', details: null }] },
      ] } };
      host.__datedRowData = ongoing; render();
      const current = columnsTree().find(node => (node.props.lane as { slot?: { index: number } } | undefined)?.slot?.index === 1 && node.props.hour === last)!;
      function CurrentProbe() { cellTree = (current.type as (props: Record<string, unknown>) => ReactNode)(current.props); return cellTree; }
      renderToStaticMarkup(createElement(CurrentProbe));
      activate(nodes(cellTree).find(node => node.type === 'button')!, earlier.firstObservedHour);
      const slots = current.props.slots as ResearchReply['history']['slots'];
      const explanation = renderToStaticMarkup(createElement(Explanation, { history: ongoing.history, slots, slot: slots[1], entry: slots[1].entries[0] }));
      // No opening progress is recorded, so even a witnessed assignment only bounds the start.
      assert.ok(explanation.includes(`<small>Started by</small>${formatDate(gameDate(earlier.firstObservedHour)!)}`),
        'the current menu and explanation use the earlier joined assignment, not the current slot start');
      assert.notEqual(earlier.firstObservedHour, ongoing.history.slots[1].entries[0].firstObservedHour);
    }
    // A witnessed assignment is the start only when its opening progress is explained; parked progress only bounds it.
    for (const [progressRaw, label] of [['0', 'Started'], ['9000000', 'Started by']] as const) {
      const entry = { ...research.history.slots[0].entries[0], lastObservedHour: last, endedByHour: null, outcomeObservedHour: null, outcome: 'active' as const };
      const slot = { index: 0, availability: [{ firstHour: first, lastHour: last }], entries: [entry],
        states: [{ firstHour: entry.firstObservedHour, lastHour: last, token: entry.token, entryFirstObservedHour: entry.firstObservedHour,
          speedMultiplierRaw: '100000', bankedDaysRaw: '0', appliedDaysRaw: '0', aheadReductionYearsRaw: '0', details: null }],
        progress: [{ firstHour: entry.firstObservedHour, lastHour: last, entryFirstObservedHour: entry.firstObservedHour, baseDaysRaw: '18000000', progressRaw }] };
      const explanation = renderToStaticMarkup(createElement(Explanation, { history: { coverage: [{ firstHour: first, lastHour: last }], slots: [slot] },
        slots: [slot], slot, entry }));
      assert.ok(explanation.includes(`<small>${label}</small>${formatDate(gameDate(entry.firstObservedHour)!)}`));
    }
    const { rowJumpProps } = await vite.ssrLoadModule('/ui/src/charts/jump.ts') as typeof import('../charts/jump.ts');
    let lookedUp = 0, prevented = 0;
    const unknown = rowJumpProps(props.runtime, track.id, () => { lookedUp++; return null; }, () => assert.fail('an undated lazy row has no menu'));
    const event = { clientX: 0, clientY: 0, currentTarget: { getBoundingClientRect: () => ({ left: 0, bottom: 0 }) },
      preventDefault() { prevented++; }, stopPropagation() { prevented++; } };
    for (const key of ['Enter', ' ', 'F10']) (unknown.onKeyDown as (event: unknown) => void)({ ...event, key, shiftKey: false });
    assert.equal(lookedUp, 0, 'ordinary activation does not resolve the journey');
    (unknown.onContextMenu as (event: unknown) => void)(event);
    (unknown.onKeyDown as (event: unknown) => void)({ ...event, key: 'ContextMenu', shiftKey: false });
    assert.equal(lookedUp, 2); assert.equal(prevented, 0, 'a missing lazy date preserves the native context event');
  } finally { delete host.__datedRowData; delete host.__datedRowMenu; delete host.__datedRowKey; await vite.close(); }
});
