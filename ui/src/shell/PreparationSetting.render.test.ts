import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

test('an unread background-preparation preference is unknown, and a failed first read can be retried', async () => {
  const slots: unknown[] = []; let cursor = 0; let effects: (() => void)[] = [];
  const hooks = {
    useState: (initial: unknown) => { const at = cursor++; if (!(at in slots)) slots[at] = initial;
      return [slots[at], (next: unknown) => { slots[at] = typeof next === 'function' ? (next as (value: unknown) => unknown)(slots[at]) : next; }]; },
    useRef: (initial: unknown) => { const at = cursor++; return (slots[at] ??= { current: initial }); },
    useEffect: (effect: () => void, deps: unknown[]) => { const at = cursor++; const before = slots[at] as unknown[] | undefined;
      if (!before || deps.some((value, index) => !Object.is(value, before[index]))) { slots[at] = deps; effects.push(effect); } },
  };
  const host = globalThis as typeof globalThis & { __preparationSetting?: typeof hooks };
  host.__preparationSetting = hooks;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'preparation-setting-hooks', enforce: 'pre', transform(source, id) {
      if (id.replaceAll('\\', '/').endsWith('/shell/PreparationSetting.tsx'))
        return source.replace("import { useEffect, useRef, useState } from 'react';", 'const { useEffect, useRef, useState } = globalThis.__preparationSetting;');
    } }] });
  try {
    const { default: Setting } = await vite.ssrLoadModule('/ui/src/shell/PreparationSetting.tsx');
    let fail = true; const asked: (boolean | null)[] = [];
    const client = { preparationPreferences: async (enabled: boolean | null) => {
      asked.push(enabled);
      if (fail) throw new Error('service unavailable');
      return { backgroundEnabled: false };
    } };
    let markup = '';
    const render = async () => { cursor = 0; effects = []; markup = renderToStaticMarkup(createElement(Setting, { client }));
      for (const effect of effects.splice(0)) effect(); for (let i = 0; i < 4; i++) await new Promise(resolve => setImmediate(resolve)); };
    await render(); await render();
    assert.match(markup, /service unavailable/);
    assert.match(markup, /Unknown/);
    assert.doesNotMatch(markup, /type="checkbox"/, 'an unread preference is not shown as on');
    assert.match(markup, /<button[^>]*>Retry<\/button>/, 'the failed read offers a retry');
    fail = false;
    const find = (node: any, match: (node: any) => boolean): any => {
      if (node == null || typeof node !== 'object') return undefined;
      if (Array.isArray(node)) { for (const child of node) { const found = find(child, match); if (found) return found; } return undefined; }
      return match(node) ? node : find(node.props?.children, match);
    };
    cursor = 0;
    find(Setting({ client }), node => node.type === 'button').props.onClick();
    await render(); await render();
    assert.deepEqual(asked, [null, null]);
    assert.match(markup, /type="checkbox"/);
    assert.doesNotMatch(markup, /checked=""|Unknown|service unavailable/, 'the preference read on retry is shown, off');
  } finally { await vite.close(); delete host.__preparationSetting; }
});
