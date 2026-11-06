import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, isValidElement } from 'react';
import type { ComponentProps, ReactElement, ReactNode } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import { datedCountryToken } from '../../dashboard/datedCountry.ts';
import { productionLineToken } from './productionData.ts';
import type ProductionDrill from './ProductionDrill';

function followButton(node: ReactNode): (() => void) | undefined {
  if (Array.isArray(node)) return node.map(followButton).find(Boolean);
  if (!isValidElement<{ children?: ReactNode; onClick?: () => void; onFollow?: () => void }>(node)) return undefined;
  return node.type === 'button' && node.props.children === 'Follow playback' ? node.props.onClick : typeof node.props.onFollow === 'function' ? node.props.onFollow : followButton(node.props.children);
}

function pauseButton(node: ReactNode): (() => void) | undefined {
  if (Array.isArray(node)) return node.map(pauseButton).find(Boolean);
  if (!isValidElement<{ children?: ReactNode; onPause?: () => void }>(node)) return undefined;
  return node.props.onPause ?? pauseButton(node.props.children);
}

test('registered production pins publish local body dates and native aliases, retaining the original request basis', async () => {
  const state = { overview: true, cursor: 0, hooks: [] as unknown[], hour: 112, requests: [] as unknown[],
    drill: undefined as ComponentProps<typeof ProductionDrill> | undefined,
    chip: undefined as ReactElement<{ hour: number | null; reason: string | null; label: { name: string; flag: string } | null }> | undefined };
  const harness = {
    useState(initial: unknown) {
      const index = state.cursor++;
      if (!(index in state.hooks)) state.hooks[index] = initial;
      return [state.hooks[index], (value: unknown) => { state.hooks[index] = typeof value === 'function' ? value(state.hooks[index]) : value; }];
    },
    useMemo(factory: () => unknown, deps: unknown[]) {
      const index = state.cursor++, previous = state.hooks[index] as { deps: unknown[]; value: unknown } | undefined;
      if (!previous || deps.some((value, i) => value !== previous.deps[i])) state.hooks[index] = { deps, value: factory() };
      return (state.hooks[index] as { value: unknown }).value;
    },
    useRef(initial: unknown) { return this.useMemo(() => ({ current: initial }), []); },
    useEffect() {},
  };
  const globals = globalThis as typeof globalThis & { __productionPin?: typeof state; __productionHooks?: typeof harness };
  globals.__productionPin = state;
  globals.__productionHooks = harness;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'production-native-header', enforce: 'pre', transform(source, id) {
      const path = id.replaceAll('\\', '/');
      if (path.endsWith('/production-lines/useProductionLines.ts')) return `
        import { datedCountrySpans } from '../../dashboard/datedCountry';
        export const useProductionLines = (widget, runtime) => ({ track: runtime.tracks[0], tag: widget.countries.tags[0],
          data: globalThis.__productionPin.overview ? { branchId: 'b', tag: 'X', lines: [], reads: [100], approximate: [], detailHours: 720, warnings: [] } : undefined });
        export const useLineDetail = () => ({ data: new Map(), errors: new Map(), retry() {} });
        export const useLineHour = () => ({ detail: undefined, hour: undefined, error: undefined, retry() {} });
        export const useProductionCountry = (selection, _runtime, track) => ({ spans: selection ? datedCountrySpans([
          { tag: 'X', identities: [{ firstHour: 100, lastHour: 109, slot: 1, countryId: 10 }, { firstHour: 110, lastHour: 120, slot: 2, countryId: 20 }] },
          { tag: 'Y', identities: [{ firstHour: 110, lastHour: 114, slot: 1, countryId: 10 }, { firstHour: 117, lastHour: 120, slot: 3, countryId: 30 }] }
        ], selection, track.branch.firstHour, track.branch.lastHour) : null, reason: null });
        export const useLineBreakdown = (_runtime, track, tag, key, group, _active, originHour) => {
          if (tag && key) globalThis.__productionPin.requests.push({ branchId: track.branch.id, tag, key, group, originHour });
          return { data: { runs: [], groups: [], warnings: [] } };
        };`;
      // The body is called outside React, so outside a widget frame: its errors stay its own.
      if (path.endsWith('/dashboard/heldFailures.ts')) return `import { createContext } from 'react'; const none = [];
        export class HeldFailures { subscribe = () => () => {}; read = () => none; retry() {} }
        export const HeldFailuresContext = createContext(null); export const useHeldFailure = error => error;
        export const useHeldFailures = () => false;`;
      if (path.endsWith('/production-lines/ProductionDrill.tsx')) return 'export default props => { globalThis.__productionPin.drill = props; return String(props.hour); };';
      if (path.endsWith('/dashboard/countryFlagsAt.ts')) return `export const useCountryLabelsAt = () => ({ labelAt: hour => tag => ({
        name: tag + ' at ' + hour, flag: 'data:flag-' + tag + '@' + hour, known: true }) });`;
      if (path.endsWith('/dashboard/playback.ts')) return 'export const usePlaybackHour = () => globalThis.__productionPin.hour; export const usePlaybackMotion = () => undefined;';
      if (path.endsWith('/charts/useChartTokens.ts')) return 'export const useChartTokens = () => ({}); export const useViewportHeight = () => 1000;';
      if (path.endsWith('/components/ContextMenu.tsx')) return 'export const useContextMenu = () => ({ menu: null, open() {} });';
      if (path.endsWith('/production-lines/ProductionLinesWidget.tsx')) return source
        .replace("import { useCallback, useEffect, useMemo, useState, useRef } from 'react';", `const { useEffect, useMemo, useState } = globalThis.__productionHooks;
          const useRef = initial => globalThis.__productionHooks.useRef(initial);`)
        .replace('createPortal(', '((chip) => { globalThis.__productionPin.chip = chip; return null; })(');
    } }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/production-lines/ProductionLinesWidget.tsx');
    const renderer = (await vite.ssrLoadModule('/ui/src/widgets/production-lines/index.ts')).default;
    assert.equal(renderer.render, Widget);
    assert.match(renderToStaticMarkup(createElement(renderer.countryChip, { widget: { options: { show: 'efficiency' } }, onSlot() {} })), /dash-widget-country-slot/);
    const props = { widget: { id: 'lines', kind: 'production-lines', trackIds: ['track'], countries: { kind: 'tags', tags: ['X'] },
      time: { mode: 'pin', windows: { track: { firstOffset: 5, lastOffset: 5 } } }, options: { show: 'efficiency',
        productionCountry: datedCountryToken({ campaign: 'c', branch: 'b', tag: 'X', originOffset: 5 }), line: productionLineToken('c', 'b', 'X', '5#1') } },
      runtime: { client: {}, tracks: [{ id: 'track', axisOffset: 0, branch: { id: 'b', firstHour: 100, lastHour: 120 }, source: { id: 'source', revision: '1', metadata: { campaignKey: 'c' } } }],
        clock: { getHour: () => state.hour, getSnapshot: () => ({ playing: false, hoursPerSecond: 0 }) } },
      active: true, countrySlot: {} as HTMLElement, onDiagnostics() {} } as unknown as WidgetProps;
    let node: ReactNode;
    const render = () => { state.cursor = 0; state.requests = []; node = Widget(props); return renderToStaticMarkup(node); };
    assert.match(render(), /105/);
    assert.equal(state.chip!.props.label!.name, 'X at 105');
    const savedTime = props.widget.time;
    props.widget.time = { mode: 'pin', windows: {} }; state.drill = undefined;
    assert.match(render(), /Choose another time in widget settings/);
    assert.equal(state.chip!.props.label, null);
    assert.equal(state.chip!.props.hour, null);
    assert.equal(state.drill, undefined, 'a missing saved date cannot display the branch end');
    props.widget.time = savedTime; render();
    state.drill!.onPick(112); render();
    assert.equal(state.chip!.props.label!.flag, 'data:flag-Y@112');
    assert.deepEqual(state.requests[0], { branchId: 'b', tag: 'X', key: '5#1', group: 'efficiency', originHour: 105 });
    state.hour = 113; followButton(node)!(); render();
    assert.equal(state.drill!.hour, 113);
    assert.equal(state.chip!.props.label!.name, 'Y at 113');
    pauseButton(node)!(); state.hour = 114; render();
    assert.equal(state.drill!.hour, 113, 'pausing shared Follow holds the displayed date');
    followButton(node)!(); state.hour = 113; render();
    state.overview = false; render();
    assert.equal(state.drill!.hour, 113, 'the saved native breakdown does not depend on the original tag overview');
    state.overview = true;
    state.hour = 116; render(); assert.equal(state.chip!.props.label, null);
    state.hour = 119; render(); assert.equal(state.chip!.props.label, null);
    const oldPick = state.drill!.onPick;
    const oldPause = pauseButton(node)!;
    props.runtime = { ...props.runtime, client: { ...props.runtime.client } }; render();
    assert.equal(state.drill!.hour, 105, 'a client handoff masks local state before cleanup');
    oldPause(); render(); assert.equal(state.drill!.hour, 105, 'a stale Follow callback cannot pick a date from the previous source');
    oldPick(112); render(); assert.equal(state.drill!.hour, 105);
    const hiddenPick = state.drill!.onPick;
    const hiddenPause = pauseButton(node)!;
    props.active = false; render(); hiddenPick(112); hiddenPause(); render();
    assert.equal(state.drill!.hour, 105, 'an inactive widget rejects callbacks from its visible render');
    props.active = true; render();
    props.widget = { ...props.widget, options: { ...props.widget.options, productionCountry: datedCountryToken({ campaign: 'other', branch: 'b', tag: 'X', originOffset: 5 }) } };
    assert.match(render(), /Saved country selection unavailable. Open the breakdown again/);
    assert.equal(state.chip!.props.label, null);
    assert.equal(state.requests.length, 0);
  } finally { delete globals.__productionPin; delete globals.__productionHooks; await vite.close(); }
});
