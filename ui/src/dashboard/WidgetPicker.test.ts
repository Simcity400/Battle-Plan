import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { DashboardClient, ProductReply, ProductRequest } from './client';
import type { WidgetMeta, WidgetPickerItemsProps, WidgetRuntime } from './definitions';
import type { BoundTrack, DashboardTemplate, WidgetSpec } from './model';
import type { CategoryInfo, IdeaHistoryReply } from '../generated/dashboard';

const category = (token: string, name: string | null, changes: Partial<CategoryInfo> = {}): CategoryInfo => ({
  token, name, kind: null, hidden: false, politicsTab: true,
  slots: [{ token: `${token}_slot`, name: null, character: false, seats: 1, laws: [] }], ...changes,
});
const reply = (categories: CategoryInfo[]): IdeaHistoryReply => ({ branchId: 'branch', tag: 'recorded_country',
  categories, spans: [], holders: [], ideas: [], artwork: [], unobserved: [], identityStarts: [], modifiers: [], texts: [], warnings: [] });
type Button = { definition: WidgetMeta; initial?: Pick<WidgetSpec, 'title' | 'options'>; onAdd: WidgetPickerItemsProps['onAdd'] };
type Effect = { deps: readonly unknown[]; run: () => void | (() => void); cleanup?: () => void };
const settle = () => new Promise<void>(resolve => setImmediate(resolve));
const MOD_CATEGORY = 'mod council,elite:20:seat';

test('actual picker offers content categories, persists their choices and excludes stale campaign callbacks', async () => {
  const cells = new Map<number, unknown>(), effects = new Map<number, Effect>(), pending = new Map<number, Effect>();
  let cursor = 0;
  const cell = (initial: () => unknown) => { const id = cursor++; if (!cells.has(id)) cells.set(id, initial()); return cells.get(id); };
  const changed = (before: readonly unknown[], next: readonly unknown[]) => before.length !== next.length || before.some((value, i) => !Object.is(value, next[i]));
  const fixture = {
    open: false, buttons: [] as Button[],
    useRef: (initial: unknown) => cell(() => ({ current: initial })),
    useMemo(run: () => unknown, deps: readonly unknown[]) {
      const memo = cell(() => ({ value: run(), deps })) as { value: unknown; deps: readonly unknown[] };
      if (changed(memo.deps, deps)) { memo.value = run(); memo.deps = deps; } return memo.value;
    },
    useState(initial: unknown) {
      const state = cell(() => ({ value: initial })) as { value: unknown };
      return [state.value, (next: unknown) => { state.value = typeof next === 'function' ? next(state.value) : next; }];
    },
    useEffect(run: Effect['run'], deps: readonly unknown[]) {
      const id = cursor++, before = effects.get(id);
      if (!before || changed(before.deps, deps)) pending.set(id, { run, deps });
    },
    commit() {
      for (const [id, effect] of pending) { effects.get(id)?.cleanup?.(); const cleanup = effect.run();
        effect.cleanup = typeof cleanup === 'function' ? cleanup : undefined; effects.set(id, effect); }
      pending.clear();
    },
    replayEffects() {
      for (const effect of effects.values()) {
        effect.cleanup?.(); const cleanup = effect.run();
        effect.cleanup = typeof cleanup === 'function' ? cleanup : undefined;
      }
    },
    unmount() {
      for (const effect of effects.values()) effect.cleanup?.();
      effects.clear(); pending.clear(); cells.clear();
    },
  };
  const globals = globalThis as typeof globalThis & { __ideaPicker?: typeof fixture };
  globals.__ideaPicker = fixture;
  // Keep the product hook, cache, request identity, reply validator and picker code real.
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'idea-picker-fixture', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/components/Popover.tsx')) return `export const usePopover = () => ({ trigger: null, open: globalThis.__ideaPicker.open,
          close() { globalThis.__ideaPicker.open = false; } }); export const Popover = ({ children }) => children;`;
        if (path.endsWith('/dashboard/products.ts')) return source.replace("import { useEffect, useState } from 'react';",
          'const { useEffect, useState } = globalThis.__ideaPicker;');
        if (path.endsWith('/idea-timeline/IdeaTimelinePickerItems.tsx')) return source.replace("import { useEffect, useMemo, useRef } from 'react';",
          'const { useEffect, useMemo, useRef } = globalThis.__ideaPicker;');
        if (path.endsWith('/dashboard/contentRevision.ts')) return source.replace(
          'return useSyncExternalStore(revision.subscribe, revision.snapshot, revision.snapshot);', 'return revision.snapshot();');
        if (path.endsWith('/dashboard/WidgetPickerItem.tsx')) return source.replace('return <button',
          'globalThis.__ideaPicker.buttons.push({ definition, initial, onAdd }); return <button');
      },
    }] });
  const calls: { request: ProductRequest; signal: AbortSignal; answer: (data: IdeaHistoryReply) => void; fail: () => void }[] = [];
  const client = { product: (request: ProductRequest, signal: AbortSignal) => new Promise<ProductReply<IdeaHistoryReply>>((resolve, reject) => {
    calls.push({ request, signal, answer: data => resolve({ ...request, data }), fail: () => reject(new Error('Recorded category content could not be loaded.')) });
  }) } as unknown as DashboardClient;
  try {
    const { registerWidgets } = await vite.ssrLoadModule('/ui/src/dashboard/definitions.ts') as typeof import('./definitions.ts');
    const { createWidget, addGridItem } = await vite.ssrLoadModule('/ui/src/dashboard/model.ts') as typeof import('./model.ts');
    const { parseTemplate } = await vite.ssrLoadModule('/ui/src/dashboard/storage.ts') as typeof import('./storage.ts');
    const { contentRevision } = await vite.ssrLoadModule('/ui/src/dashboard/contentRevision.ts') as typeof import('./contentRevision.ts');
    const { definition } = await vite.ssrLoadModule('/ui/src/widgets/idea-timeline/definition.ts');
    const { OPTIONS } = await vite.ssrLoadModule('/ui/src/widgets/idea-timeline/definition.ts');
    const { chosenCategory } = await vite.ssrLoadModule('/ui/src/widgets/idea-timeline/ideaTimelineData.ts') as typeof import('../widgets/idea-timeline/ideaTimelineData.ts');
    const { OptionFields } = await vite.ssrLoadModule('/ui/src/dashboard/settings/OptionFields.tsx') as typeof import('./settings/OptionFields.tsx');
    const { default: pickerItems } = await vite.ssrLoadModule('/ui/src/widgets/idea-timeline/IdeaTimelinePickerItems.tsx');
    const { default: Picker } = await vite.ssrLoadModule('/ui/src/dashboard/WidgetPicker.tsx');
    registerWidgets([{ ...definition, pickerItems }, { ...definition, kind: 'plain-widget', title: 'Plain widget' }]);
    const template: DashboardTemplate = { version: 1, name: 'Synthetic', alignment: 'calendar', roles: [{ id: 'primary', label: 'Primary', selectedTag: null }, { id: 'active', label: 'Active', selectedTag: null }],
      tracks: [{ id: 'selected', roleId: 'active', label: 'Selected', branchIndex: 0, anchorOffset: null }], widgets: [], layouts: {} };
    const track = { ...template.tracks[0], selectedTag: null, source: { id: 'source-a', revision: 'revision-a',
      metadata: { playerCountryId: 14, countries: [{ id: 14, tag: 'recorded_country' }] } }, branch: { id: 'branch' } } as unknown as BoundTrack;
    const runtime = { client, defaultRoleId: 'active', campaignTracks: [{ ...template.tracks[0], id: 'unrelated', roleId: 'unrelated' }, ...template.tracks],
      tracks: [{ ...track, id: 'unrelated', roleId: 'unrelated', source: { ...track.source, id: 'unrelated-source' } }, track] } as unknown as WidgetRuntime;
    const added: WidgetSpec[] = [];
    const render = () => { cursor = 0; fixture.buttons = []; return renderToStaticMarkup(createElement(Picker, { runtime, disabled: false, onOpen() {},
      onAdd: (kind: string, initial?: Pick<WidgetSpec, 'title' | 'options'>) => added.push({ ...createWidget(kind, 'selected', `added-${added.length}`), ...initial }) })); };
    render(); fixture.commit(); await settle(); assert.equal(calls.length, 0, 'a closed picker does not request categories');
    fixture.open = true;
    assert.match(render(), /Loading this campaign’s idea categories/); fixture.commit(); await settle();
    assert.deepEqual(calls[0].request, { sourceId: 'source-a', revision: 'revision-a', product: 'idea-history', params: { branchId: 'branch', tag: 'recorded_country' } });
    calls[0].answer(reply([category('mod_laws', 'Mod laws'), category('mod_staff', 'Mod staff'), category(MOD_CATEGORY, 'Extra council'),
      category('hidden', 'Hidden category', { hidden: true }), category('nonpolitical', 'Outside politics', { politicsTab: false }), category('empty', 'Empty category', { slots: [] })]));
    await settle();
    const markup = render(); fixture.commit();
    assert.match(markup, /<strong>Mod laws<\/strong>/); assert.match(markup, /<strong>Mod staff<\/strong>/); assert.match(markup, /<strong>Extra council<\/strong>/);
    assert.match(markup, /<strong>Idea timeline<\/strong>/); assert.match(markup, /<strong>Plain widget<\/strong>/);
    assert.doesNotMatch(markup, /Hidden category|Outside politics|Empty category/);
    const extra = fixture.buttons.find(button => button.initial?.options?.category === MOD_CATEGORY)!;
    assert.ok(extra, 'the mod category has its own actual picker button'); extra.onAdd(extra.definition.kind, extra.initial);
    assert.equal(added[0].kind, 'idea-timeline'); assert.equal(added[0].title, 'Extra council'); assert.equal(added[0].options?.category, MOD_CATEGORY);
    assert.deepEqual(added[0].countries, { kind: 'selected' }, 'content selection does not invent country identities');
    const restored = parseTemplate({ ...template, widgets: added, layouts: addGridItem(template.layouts, added[0]) });
    assert.ok(restored); assert.equal(restored.widgets[0].options?.category, MOD_CATEGORY);
    assert.equal(restored?.widgets[0].title, 'Extra council');
    assert.equal(chosenCategory([category(MOD_CATEGORY, 'Extra council')], String(restored.widgets[0].options?.category))?.token, MOD_CATEGORY);
    const settings = renderToStaticMarkup(createElement(OptionFields, { fields: OPTIONS.filter((field: { key: string }) => field.key === 'category'),
      values: { category: MOD_CATEGORY }, choices: { category: [{ value: MOD_CATEGORY, label: 'Extra council' }] }, onChange() {} }));
    assert.match(settings, /value="mod council,elite:20:seat" selected=""/); assert.doesNotMatch(settings, /disabled|cannot be saved/);
    const stale = extra.onAdd;
    render(); fixture.commit(); stale(extra.definition.kind, extra.initial); assert.equal(added.length, 1, 'a closed picker rejects its old category callback');
    fixture.open = true; render(); fixture.commit();
    const retiring = fixture.buttons.find(button => button.initial?.options?.category === MOD_CATEGORY)!;
    fixture.unmount();
    retiring.onAdd(retiring.definition.kind, retiring.initial); assert.equal(added.length, 1, 'actual unmount cleanup revokes the prior callback');
    render(); fixture.commit(); await settle(); render(); fixture.commit();
    fixture.replayEffects(); // StrictMode cleanup/setup must restore the current mount without reviving an old mount.
    retiring.onAdd(retiring.definition.kind, retiring.initial); assert.equal(added.length, 1, 'a new mount cannot revive the abandoned callback');
    const replacement = fixture.buttons.find(button => button.initial?.options?.category === MOD_CATEGORY)!;
    assert.ok(replacement); replacement.onAdd(replacement.definition.kind, replacement.initial);
    assert.equal(added.length, 2, 'the current mount remains actionable after effect cleanup/setup');
    fixture.open = true; track.source = { ...track.source, id: 'source-b' };
    assert.doesNotMatch(render(), /Extra council/); fixture.commit(); await settle();
    stale(extra.definition.kind, extra.initial); assert.equal(added.length, 2);
    assert.equal(calls[1].request.sourceId, 'source-b');
    track.source = { ...track.source, id: 'source-c' };
    assert.doesNotMatch(render(), /Extra council/); fixture.commit(); await settle(); assert.equal(calls[1].signal.aborted, true);
    calls[1].answer(reply([category('late', 'Late old campaign')])); await settle(); assert.doesNotMatch(render(), /Late old campaign/);
    calls[2].fail(); await settle(); assert.match(render(), /Recorded category content could not be loaded/);
    contentRevision(client).observe({ game: 'synthetic-content', user: '', contentReady: true });
    assert.doesNotMatch(render(), /Recorded category content could not be loaded/); fixture.commit(); await settle();
    const longName = 'Synthetic category title '.repeat(10);
    calls[3].answer({ ...reply([category('raw_label', null), category('empty_title', ''), category('long_title', longName)]),
      warnings: ['Synthetic category content warning.'] }); await settle();
    const content = render(); assert.match(content, /<strong>raw_label<\/strong>/); assert.match(content, /Synthetic category content warning/);
    const empty = fixture.buttons.find(button => button.initial?.options?.category === 'empty_title')!;
    assert.equal(empty.definition.title, '', 'known empty category names stay empty in the picker');
    assert.equal(empty.initial?.title, definition.title, 'an empty content label gets the existing editable application title');
    empty.onAdd(empty.definition.kind, empty.initial); render(); fixture.commit();
    fixture.open = true; render(); fixture.commit();
    const long = fixture.buttons.find(button => button.initial?.options?.category === 'long_title')!;
    long.onAdd(long.definition.kind, long.initial);
    const full = parseTemplate({ ...template, widgets: added, layouts: added.reduce((layouts, widget) => addGridItem(layouts, widget), {}) });
    assert.ok(full); assert.equal(full.widgets[2].title, definition.title); assert.equal(full.widgets[2].options?.category, 'empty_title');
    assert.equal(full.widgets[3].title, longName, 'a full mod category title survives storage without a length bound');
    stale(extra.definition.kind, extra.initial); assert.equal(added.length, 4);
    fixture.open = true;
    track.selectedTag = 'another_literal_tag'; assert.doesNotMatch(render(), /raw_label/); fixture.commit(); await settle();
    assert.equal((calls[4].request.params as { tag: string }).tag, 'another_literal_tag');
    calls[4].answer(reply([])); await settle(); assert.match(render(), /no political-screen idea categories with slots/);
    runtime.tracks = runtime.tracks.filter(item => item.roleId !== 'active');
    assert.match(render(), /Load a campaign to list its idea categories/); fixture.commit(); await settle(); assert.equal(calls.length, 5);
  } finally { fixture.unmount(); await vite.close(); delete globals.__ideaPicker; }
});
