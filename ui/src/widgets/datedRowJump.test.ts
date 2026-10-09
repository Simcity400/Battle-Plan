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
const nodes = (tree: unknown): Node[] => Array.isArray(tree) ? tree.flatMap(nodes)
  : tree && typeof tree === 'object' && 'props' in tree ? [tree as Node, ...nodes((tree as Node).props.children)] : [];

test('registered Focus and Research rows Jump to their recorded bounds through pointer and menu keys', async () => {
  const first = HOUR_EPOCH + 100, focusHour = first + 25, ended = first + 48, confirmed = first + 60, last = first + 100;
  const focus: FocusReply = { history: { segments: [{ firstHour: first, lastHour: focusHour - 1, order: {
    completed: [{ ordinal: 0, token: 'already_done', completedByHour: null }], current: 'new_done', continuous: null, paused: false,
  } }, { firstHour: focusHour, lastHour: last, order: {
    completed: [{ ordinal: 0, token: 'already_done', completedByHour: null }, { ordinal: 1, token: 'new_done', completedByHour: focusHour }],
    current: 'unfinished', continuous: null, paused: false,
  } }], progress: [] }, artwork: [], warnings: [] };
  const research: ResearchReply = { history: { coverage: [{ firstHour: first, lastHour: last }], slots: [{ index: 0,
    availability: [{ firstHour: first, lastHour: last }], states: [], progress: [], entries: [{ token: 'technology', technologyHandle: 7,
      continuityStartHour: first, level: 0, firstObservedHour: first + 10, startedAfterHour: first + 9,
      lastObservedHour: ended - 1, endedByHour: ended, outcomeObservedHour: confirmed, outcome: 'completed' }],
  }] }, artwork: [], warnings: [] };
  const host = globalThis as typeof globalThis & { __datedRowData?: FocusReply | ResearchReply;
    __datedRowMenu?: { x: number; y: number; items: readonly MenuItem[] } };
  const shownMenu = () => host.__datedRowMenu;
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': 'export const useProduct = () => ({ data: globalThis.__datedRowData });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => {};',
    '/national-focus/useFocusArtwork.ts': 'export const useFocusArtwork = () => ({ read: new Map(), warnings: [] });',
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
    const researchRenderer = (await vite.ssrLoadModule('/ui/src/widgets/research/index.ts')).default as Renderer;
    const seeks: [string, number][] = [];
    const track = { id: 'track', axisOffset: 12, branch: { id: 'branch', firstHour: first, lastHour: last, gaps: [] },
      source: { id: 'source', revision: '1' } };
    let hour = last, tree: ReactNode = null;
    const props = { widget: { kind: 'national-focus', trackIds: ['track'], countries: { kind: 'tags', tags: ['synthetic'] }, time: { mode: 'follow' } },
      runtime: { client: {}, tracks: [track], clock: { getHour: () => hour - track.axisOffset }, requestSeek: (id: string, at: number) => seeks.push([id, at]) },
      active: true, onDiagnostics() {} } as unknown as WidgetProps;
    let renderer = focusRenderer;
    function Probe() { tree = renderer.render(props); return tree; }
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
    assert.ok(nodes(tree).filter(node => node.type === 'li').every(node => node.props.onContextMenu === undefined), 'a completion is not dated before its first observation');
    hour = last;
    const html = render();
    assert.match(html, /data-row-menu="National focus actions"/);
    assert.ok(html.includes(`By ${formatDate(gameDate(focusHour)!)}`));
    const focusRows = nodes(tree).filter(node => node.type === 'li');
    assert.equal(focusRows.length, 2);
    assert.equal(focusRows[0].props.onContextMenu, undefined, 'an artwork snapshot does not date an earlier completion');
    assert.equal(focusRows[0].props.onKeyDown, undefined);
    activate(focusRows[1], focusHour);
    props.widget.time = { mode: 'campaign' }; render();
    assert.equal(nodes(tree).find(node => node.props.className === 'focus-entry is-unfinished')?.props.onContextMenu, undefined);

    renderer = researchRenderer; props.widget.kind = 'research'; props.widget.time = { mode: 'follow' };
    host.__datedRowData = research;
    hour = confirmed - 1; render();
    assert.equal(nodes(tree).filter(node => node.props.entry).length, 0, 'a later completion classification cannot appear early');
    hour = confirmed;
    assert.match(render(), /data-row-menu="Research actions"/);
    const card = nodes(tree).find(node => node.props.entry)!;
    let cardTree: ReactNode = null;
    function CardProbe() { cardTree = (card.type as (props: Record<string, unknown>) => ReactNode)(card.props); return cardTree; }
    renderToStaticMarkup(createElement(CardProbe));
    activate(nodes(cardTree).find(node => node.type === 'li')!, ended);
    const beforeOutside = seeks.length;
    track.branch.firstHour = ended + 1; render();
    const outside = nodes(tree).find(node => node.props.entry)!;
    const jump = outside.props.jump as { onContextMenu: (event: unknown) => void };
    jump.onContextMenu({ clientX: 0, clientY: 0, preventDefault() {}, stopPropagation() {} });
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
      const current = nodes(tree).find(node => node.props.current)!;
      function CurrentProbe() { cardTree = (current.type as (props: Record<string, unknown>) => ReactNode)(current.props); return cardTree; }
      renderToStaticMarkup(createElement(CurrentProbe));
      activate(nodes(cardTree).find(node => node.props.className === 'research-now-details')!, earlier.firstObservedHour);
      const explanation = renderToStaticMarkup(createElement(Explanation, current.props));
      assert.ok(explanation.includes(`<small>${startedAfterHour === null ? 'Started by' : 'Started'}</small>${formatDate(gameDate(earlier.firstObservedHour)!)}`),
        'the current menu and explanation use the earlier joined assignment, not the current slot start');
      assert.notEqual(earlier.firstObservedHour, ongoing.history.slots[1].entries[0].firstObservedHour);
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
  } finally { delete host.__datedRowData; delete host.__datedRowMenu; await vite.close(); }
});
