import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { tokensFromValues } from '../tokens.ts';
import type { SunburstProps } from './Sunburst';

test('sunburst centre qualifies a known subtotal without an omitted-count inventory', async () => {
  const state = { effects: [] as (() => unknown)[], refs: [] as {current: unknown}[], texts: [] as string[],
    tokens: {...tokensFromValues(() => '#777777', 'sans-serif'), reducedMotion: true},
    frame: undefined as ((info: {now: number; dt: number}) => boolean) | undefined };
  const host = globalThis as typeof globalThis & {__sunburstCentre?: typeof state}; host.__sunburstCentre = state;
  const globals = ['ResizeObserver', 'window'].map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const);
  Object.defineProperty(globalThis, 'ResizeObserver', {configurable: true, value: class {observe() {} disconnect() {}}});
  Object.defineProperty(globalThis, 'window', {configurable: true, value: {devicePixelRatio: 1}});
  const vite = await createServer({configFile: false, optimizeDeps: {noDiscovery: true},
    server: {middlewareMode: true, watch: null, hmr: false, ws: false}, plugins: [{name: 'sunburst-centre-frame', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/charts/useChartTokens.ts')) return 'export const useChartTokens = () => globalThis.__sunburstCentre.tokens;';
        if (path.endsWith('/sunburst/Sunburst.tsx')) return source
          .replace("import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';", `import {useCallback, useId, useMemo, useRef as ref, useState} from 'react';
            const useRef = initial => {const value = ref(initial); globalThis.__sunburstCentre.refs.push(value); return value;};
            const useEffect = effect => globalThis.__sunburstCentre.effects.push(effect);`)
          .replace("import { scheduler } from '../scheduler';", `const scheduler = {add: task => {globalThis.__sunburstCentre.frame = task.frame; return () => {};}, wake() {}, setVisible() {}};`)
          .replace("import { readTokens, subscribeTokens } from '../tokens';", 'const readTokens = () => globalThis.__sunburstCentre.tokens; const subscribeTokens = () => () => {};');
      },
    }]});
  let cleanup: unknown;
  try {
    const {default: Sunburst} = await vite.ssrLoadModule('/ui/src/charts/sunburst/Sunburst.tsx') as typeof import('./Sunburst.tsx');
    const context = new Proxy({measureText: (text: string) => ({width: text.length * 6}), fillText: (text: string) => state.texts.push(text)},
      {get: (target, key) => key in target ? target[key as keyof typeof target] : () => {}});
    const canvas = {clientWidth: 600, clientHeight: 600, width: 600, height: 600, style: {}, getContext: () => context,
      getBoundingClientRect: () => ({width: 600, height: 600}), addEventListener() {}, removeEventListener() {}};
    const props: SunburstProps = {label: 'Synthetic subtotal', root: {id: 'all', label: 'All'}, active: true, fold: false,
      items: [{path: [{id: 'a', label: 'Known'}], value: 30, exact: 30n, unknown: {unavailable: 2}}],
      text: {format: (_value, exact) => String(exact), unit: 'units', valueLabel: 'Amount', shares: false},
      settings: {growth: 'share', labels: true, legend: false, breadcrumb: true, dimming: true}};
    const html = renderToStaticMarkup(createElement(Sunburst, props));
    assert.match(html, /All: 30 units, partial/);
    state.refs[0].current = canvas;
    cleanup = state.effects[0](); state.frame!({now: 0, dt: 500});
    assert.ok(state.texts.includes('30')); assert.ok(state.texts.includes('partial'));
    assert.equal(state.texts.some(text => /not counted|\+\?/.test(text)), false);
  } finally {
    if (typeof cleanup === 'function') cleanup();
    await vite.close(); delete host.__sunburstCentre;
    for (const [key, descriptor] of globals) {if (descriptor) Object.defineProperty(globalThis, key, descriptor); else Reflect.deleteProperty(globalThis, key);}
  }
});
