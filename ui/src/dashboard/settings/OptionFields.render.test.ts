import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { parseStoredOptions, resolveOptions } from '../options.ts';
import type { OptionField, OptionValue } from '../options';

test('unavailable saved choices hide tokens, retain readable labels and remain independently removable', async () => {
  const state = { buttons: [] as { children: string; onClick: () => void }[] };
  const host = globalThis as typeof globalThis & { __optionFieldsRender?: typeof state };
  host.__optionFieldsRender = state;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'capture-option-buttons', enforce: 'pre',
      transform(source, id) {
        if (id.replaceAll('\\', '/').endsWith('/components/ChipSet.tsx')) {
          return source.replace('export function ChipSet(', 'function RenderChipSet(') + `
            export const ChipSet = props => {
              const tree = RenderChipSet(props);
              globalThis.__optionFieldsRender.buttons.push(...tree.props.children.map(button => button.props));
              return tree;
            };`;
        }
      },
    }] });
  try {
    const { OptionFields } = await vite.ssrLoadModule('/ui/src/dashboard/settings/OptionFields.tsx') as typeof import('./OptionFields');
    for (const kind of ['content-choice', 'content-set'] as const) {
      const field: OptionField = { key: 'saved', label: 'Saved', kind, section: 'options', automatic: 'All', default: '' };
      const fields = [field];
      const saved = kind === 'content-choice' ? 'opaque-token' : 'opaque-token another-token';
      const values = resolveOptions(fields, parseStoredOptions({ saved })!);
      const changes: Record<string, OptionValue>[] = [];
      const onChange = (key: string, value: OptionValue) => changes.push({ [key]: value });
      const html = renderToStaticMarkup(createElement(OptionFields, { fields, values, onChange }));
      const visible = (markup: string) => markup.replace(/<[^>]*>/g, ' ');
      assert.match(visible(html), /Saved choice unavailable/);
      assert.doesNotMatch(visible(html), /opaque-token|another-token/, 'the key appears only in hover details');
      assert.match(html, /Key: opaque-token/);
      assert.equal(values.saved, saved, 'opening settings preserves the saved view');
      assert.deepEqual(changes, []);
      if (kind === 'content-set') {
        assert.equal((html.match(/>Saved choice unavailable<\/button>/g) ?? []).length, 2);
        const buttons = state.buttons.splice(0);
        assert.equal(buttons.length, 2);
        buttons[0].onClick(); buttons[1].onClick();
        assert.deepEqual(changes, [{ saved: 'another-token' }, { saved: 'opaque-token' }]);
      }
      const named = renderToStaticMarkup(createElement(OptionFields, { fields, values,
        choices: { saved: [{ value: 'opaque-token', label: 'Readable name' }] }, onChange }));
      assert.match(named, /Readable name/);
      assert.doesNotMatch(visible(named), /opaque-token|another-token/);
      assert.equal((named.match(/>Saved choice unavailable<\/button>/g) ?? []).length, kind === 'content-set' ? 1 : 0);
      state.buttons = [];
    }
    const field: OptionField = { key: 'saved', label: 'Resources', kind: 'content-set', section: 'options', automatic: 'Largest resource', default: '' };
    const empty = renderToStaticMarkup(createElement(OptionFields, { fields: [field], values: { saved: '' }, onChange() {} }));
    assert.match(empty, /Largest resource/, 'the empty selection explains its non-obvious automatic choice');
    assert.match(empty, /setting-label-info/);
    const selected = renderToStaticMarkup(createElement(OptionFields, { fields: [field], values: { saved: 'opaque-token' }, onChange() {} }));
    assert.doesNotMatch(selected, /Largest resource|setting-label-info/);
  } finally { delete host.__optionFieldsRender; await vite.close(); }
});
