import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement, isValidElement } from 'react';
import type { ReactElement, ReactNode } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import type { MenuItem } from '../../components/ContextMenu';
import type { IdeaHistoryReply, IdeaInfo } from '../../generated/dashboard';

type HookSlot = { value?: unknown; deps?: readonly unknown[]; cleanup?: () => void };
function hooks() {
  const slots: HookSlot[] = [], pending: (() => void)[] = [];
  let cursor = 0;
  return {
    begin() { cursor = 0; },
    state(initial: unknown) {
      const slot = slots[cursor++] ??= { value: initial };
      return [slot.value, (next: unknown) => { slot.value = typeof next === 'function' ? next(slot.value) : next; }];
    },
    ref(initial: unknown) { const slot = slots[cursor++] ??= { value: { current: initial } }; return slot.value; },
    effect(effect: () => unknown, deps: readonly unknown[]) {
      const slot = slots[cursor++] ??= {};
      if (slot.deps && deps.every((value, index) => Object.is(value, slot.deps?.[index]))) return;
      slot.deps = deps;
      pending.push(() => { slot.cleanup?.(); const cleanup = effect(); slot.cleanup = typeof cleanup === 'function' ? cleanup as () => void : undefined; });
    },
    flush() { for (const effect of pending.splice(0)) effect(); },
    dispose() { for (const slot of slots) slot.cleanup?.(); },
  };
}
const info = (token: string): IdeaInfo => ({ token, category: 'mod_spirits', slot: null, law: false, designer: false,
  effects: [], targeted: [], equipment: [], research: [], traits: [], tooltips: [], effectsKnown: true, unknown: null, nameMissing: null });
function reply(next = false): IdeaHistoryReply {
  const tokens = next ? ['alpha', 'beta', 'common'] : ['alpha', 'old_only', 'common'];
  return { branchId: '1', tag: 'mod_country', asked: 'mod_country', unobserved: [], identityStarts: [100], modifiers: [], texts: [], warnings: [],
    categories: [{ token: 'mod_spirits', name: 'Mod spirits', kind: 'national_spirit', hidden: false, politicsTab: true, slots: [] }],
    ideas: tokens.map(info), spans: tokens.map(token => ({ token, firstHour: 120,
      lastHour: token === 'common' || token === 'beta' ? 250 : next ? 190 : 160, slot: null, character: null })),
    artwork: tokens.map(token => ({ token, name: `${next ? 'New' : 'Old'} ${token}`, description: null, baseDurationDays: null, icon: null })), holders: [] };
}
function track(): BoundTrack {
  const branch = { id: '1', sessionId: '1', firstHour: 100, lastHour: 250, main: true, fork: null, joins: [],
    sampleBoundary: 'Sample origin unknown' as const, gaps: [] };
  return { id: 'track', roleId: 'track', label: 'Synthetic', branchIndex: 0, anchorOffset: null, axisOffset: 0, selectedTag: null, branch,
    source: { id: 'source', path: 'C:/recordings/synthetic.battleplan', revision: '1', metadata: {
      path: 'C:/recordings/synthetic.battleplan', revision: '1', name: 'Synthetic', format: 1, captureMode: 'full', campaignKey: 'c0ffee',
      branches: [branch], countries: [], playerCountryId: null, contentVerified: false, partial: null, warnings: [], buildWarnings: [] } } };
}
type TileProps = { children?: ReactNode; 'aria-label': string; className: string;
  onPointerMove(event: { clientX: number; clientY: number }): void;
  onContextMenu(event: { clientX: number; clientY: number; preventDefault(): void }): void };
function tiles(node: ReactNode): ReactElement<TileProps>[] {
  if (Array.isArray(node)) return node.flatMap(tiles);
  if (!isValidElement<{ children?: ReactNode }>(node)) return [];
  return node.type === 'li' ? [node as ReactElement<TileProps>] : tiles(node.props.children);
}
const host = globalThis as typeof globalThis & { __spiritHooks?: ReturnType<typeof hooks>;
  __spiritHistory?: { track: BoundTrack; tag: string; data: IdeaHistoryReply }; __spiritMenu?: readonly MenuItem[] };

test('spirit fades and tooltips belong to their recorded context; dated tiles use the shared Jump action', async () => {
  const replacements: Record<string, string> = {
    '/dashboard/ideaHistory.ts': `import { resolveWindow } from '/ui/src/dashboard/model.ts'; export const useIdeaHour = (widget, runtime, track) => { const hour = track ? resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour())?.lastHour : undefined; return hour === undefined || hour < track.branch.firstHour || hour > track.branch.lastHour ? null : Math.floor(hour); }; export const useIdeaHistory = () => globalThis.__spiritHistory;`,
    '/dashboard/useIdeaTooltips.ts': 'export const useIdeaTooltips = () => ({ infos: new Map(), sources: [], art: new Map() });',
    '/dashboard/ideaTooltip.ts': 'export const ideaTooltip = head => ({ ...head, rows: [] }); export const tooltipText = tip => tip.title;',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open: (_x, _y, items) => { globalThis.__spiritMenu = items; }, menu: null });',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'spirit-context-fixture', enforce: 'pre',
      transform(code, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/national-spirits/NationalSpiritsWidget.tsx')) return code.replace(
          "import { useEffect, useId, useMemo, useRef, useState } from 'react';",
          "import { useId, useMemo } from 'react'; import { useEffect, useRef, useState } from 'spirit-state';");
        return Object.entries(replacements).find(([suffix]) => path.endsWith(suffix))?.[1];
      },
      resolveId(id) { if (id === 'spirit-state') return '\0spirit-state'; },
      load(id) { if (id === '\0spirit-state') return `export const useState = initial => globalThis.__spiritHooks.state(initial);
        export const useRef = initial => globalThis.__spiritHooks.ref(initial);
        export const useEffect = (effect, deps) => globalThis.__spiritHooks.effect(effect, deps);`; },
    }] });
  const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/national-spirits/NationalSpiritsWidget.tsx');
  const { tooltipStore } = await vite.ssrLoadModule('/ui/src/charts/tooltip.ts');
  const timers = new Map<ReturnType<typeof setTimeout>, { callback: () => void; delay?: number }>();
  const originalSet = globalThis.setTimeout, originalClear = globalThis.clearTimeout;
  globalThis.setTimeout = ((callback: () => void, delay?: number) => {
    const handle = {} as ReturnType<typeof setTimeout>;
    timers.set(handle, { callback, delay }); return handle;
  }) as typeof setTimeout;
  globalThis.clearTimeout = ((handle: ReturnType<typeof setTimeout>) => { timers.delete(handle); }) as typeof clearTimeout;
  try {
    for (const change of ['country', 'source', 'revision', 'campaign', 'branch', 'holder']) {
      const state = hooks(); host.__spiritHooks = state;
      const oldTrack = track(), oldData = reply();
      host.__spiritHistory = { track: oldTrack, tag: oldData.tag, data: oldData };
      let hour = 150, tree: ReactNode = null;
      const seeks: [string, number][] = [];
      const runtime = { tracks: [oldTrack], clock: { getHour: () => hour }, client: {},
        requestSeek: (id: string, at: number) => seeks.push([id, at]) };
      const props = { widget: { id: 'spirits', kind: 'national-spirits', trackIds: ['track'], time: { mode: 'follow' },
        countries: { kind: 'tags', tags: [oldData.tag] } }, runtime, active: true, onDiagnostics() {} } as unknown as WidgetProps;
      function Probe() { tree = Widget(props); return tree; }
      const render = () => { state.begin(); return renderToStaticMarkup(createElement(Probe)); };
      try {
        render(); state.flush();
        hour = 170; render(); state.flush();
        assert.match(render(), /Old old_only/, 'same-context departures retain their fade'); state.flush();
        assert.equal(timers.size, 2); assert.ok([...timers.values()].every(timer => timer.delay === 450));
        const expiredOldAlpha = [...timers.values()][0].callback;
        tiles(tree).find(tile => tile.props['aria-label'].startsWith('Old common'))!.props.onPointerMove({ clientX: 10, clientY: 20 });
        assert.equal(tooltipStore.get()?.content.title, 'Old common');
        const newTrack = track(), newData = reply(true);
        if (change === 'country') newData.tag = 'another_mod_country';
        if (change === 'source') newTrack.source.id = 'other-source';
        if (change === 'revision') newTrack.source.revision = '2';
        if (change === 'campaign') newTrack.source.metadata.campaignKey = 'decade';
        if (change === 'branch') { newTrack.branch.id = '2'; newData.branchId = '2'; }
        if (change === 'holder') { newData.identityStarts.push(165); newData.spans.forEach(span => { span.firstHour = 165; }); }
        host.__spiritHistory = { track: newTrack, tag: newData.tag, data: newData }; runtime.tracks = [newTrack];
        assert.doesNotMatch(render(), /Old /, `${change}: previous-context tiles disappear before effects run`);
        state.flush();
        assert.equal(timers.size, 0); assert.equal(tooltipStore.get(), null, 'previous-context tooltip is cleared');
        assert.doesNotMatch(render(), /Old |is-leaving/); state.flush();
        const alpha = tiles(tree).find(tile => tile.props['aria-label'].startsWith('New alpha'))!;
        let prevented = false;
        alpha.props.onContextMenu({ clientX: 10, clientY: 20, preventDefault() { prevented = true; } });
        assert.equal(prevented, true); assert.equal(host.__spiritMenu?.[0].disabled, false);
        host.__spiritMenu?.[0].run();
        assert.deepEqual(seeks, [['track', change === 'holder' ? 165 : 120]]);
        hour = 195; render(); state.flush();
        assert.match(render(), /is-leaving/); state.flush();
        assert.equal(timers.size, 1);
        expiredOldAlpha();
        assert.match(render(), /New alpha/); state.flush();
        assert.equal(timers.size, 1, "an old callback cannot remove a new owner's same-token timer");
        const [handle, timer] = [...timers][0]; timers.delete(handle); timer.callback();
        assert.doesNotMatch(render(), /New alpha/); state.flush();
        host.__spiritHistory = { track: oldTrack, tag: oldData.tag, data: oldData }; runtime.tracks = [oldTrack];
        hour = 150; render(); state.flush();
        hour = 170; render(); state.flush();
        assert.equal(timers.size, 2);
        expiredOldAlpha();
        assert.match(render(), /Old alpha/); state.flush();
        assert.equal(timers.size, 2, 'the returned context owns fresh same-token timers');
      } finally { state.dispose(); timers.clear(); }
    }
  } finally {
    globalThis.setTimeout = originalSet; globalThis.clearTimeout = originalClear;
    delete host.__spiritHooks; delete host.__spiritHistory; delete host.__spiritMenu;
    await vite.close();
  }
});
