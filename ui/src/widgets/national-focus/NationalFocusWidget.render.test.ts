import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { FocusReply } from '../../generated/dashboard';

for (const completedCount of [0, 3]) test(`Whole campaign includes its unfinished focus after ${completedCount} completions in the ordered list`, async () => {
  const data: FocusReply = { history: { segments: [{ firstHour: 0, lastHour: 30, order: {
    completed: Array.from({ length: completedCount }, (_, ordinal) => ({ ordinal, token: `synthetic_completed_${ordinal}`, completedByHour: ordinal + 1 })),
    current: 'synthetic_unfinished', continuous: null, paused: false,
  } }], progress: [] }, missing: [], artwork: [], continuousStaticText: [], warnings: [] };
  const host = globalThis as typeof globalThis & { __focusListReply?: FocusReply; __focusListInteractive?: boolean[];
    __focusListPredicate?: (item: unknown, index: number) => boolean };
  host.__focusListReply = data;
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': 'export const useProduct = () => ({ data: globalThis.__focusListReply }); export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });',
    '/dashboard/useDiagnostics.ts': 'export const useDiagnostics = () => {};',
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/national-focus/useFocusArtwork.ts': `export const useFocusArtwork = (_client, _track, _country, wanted) => ({ warnings: [],
      read: new Map(wanted.map(({ tag, hour }) => [JSON.stringify([hour, tag]), { artwork: { token: tag,
        name: 'Localized ' + tag, description: null, baseDurationDays: 10, icon: null }, reason: null }])) });
      export const useContinuousFocusArtwork = () => ({ read: new Map(), warnings: [], details: new Map() });`,
  };
  const vite = await createServer({ root: fileURLToPath(new URL('../../../../', import.meta.url)), configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'focus-list-fixture', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/components/VirtualList.tsx')) return source.replace('export function VirtualList<T>', 'function VirtualListInner<T>') + `
          export function VirtualList(props) {
            globalThis.__focusListPredicate = props.isInteractive;
            globalThis.__focusListInteractive = props.items.map((item, index) => props.isInteractive?.(item, index) ?? true);
            return VirtualListInner(props);
          }`;
        return Object.entries(replacements).find(([suffix]) => path.endsWith(suffix))?.[1];
      },
    }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/national-focus/NationalFocusWidget.tsx');
    const props = { widget: { id: 'focus', kind: 'national-focus', trackIds: ['track'], countries: { kind: 'tags', tags: ['synthetic'] }, time: { mode: 'campaign' } },
      runtime: { client: {}, tracks: [{ id: 'track', axisOffset: 0, branch: { id: 'branch', firstHour: 0, lastHour: 30 },
        source: { id: 'source', revision: '1', metadata: { campaignKey: 'campaign', playerCountryId: null, countries: [] } } }], clock: { getHour: () => 30 } }, active: true, headerSlot: null, onDiagnostics() {} } as unknown as WidgetProps;
    const markup = renderToStaticMarkup(createElement(Widget, props));
    assert.deepEqual(host.__focusListInteractive, [...Array.from({ length: completedCount }, () => true), false],
      'completed rows are keyboard destinations; the unfinished ordered row is excluded');
    const list = /<ol\b[^>]*class="focus-list"[^>]*>([\s\S]*?)<\/ol>/.exec(markup);
    assert.ok(list, 'the ordered focus list is present');
    const rows = [...list[1].matchAll(/<li\b([^>]*)>([\s\S]*?)<\/li>/g)];
    assert.equal(rows.length, completedCount + 1, 'the unfinished focus is the final list item');
    for (const [index, row] of rows.entries()) {
      assert.match(row[1], new RegExp(`aria-posinset="${index + 1}"`));
      assert.match(row[1], new RegExp(`aria-setsize="${completedCount + 1}"`));
    }
    const unfinished = rows.at(-1)!;
    assert.match(unfinished[1], /class="focus-entry is-unfinished"/);
    assert.match(unfinished[2], /Localized synthetic_unfinished/);
    assert.match(unfinished[2], /In progress/);
    assert.match(unfinished[2], new RegExp(`focus-number" aria-hidden="true">${String(completedCount + 1).padStart(2, '0')}`));
    assert.doesNotMatch(unfinished[1], /tabindex|oncontextmenu/i);
    assert.doesNotMatch(unfinished[2], /<summary|<details|<button/);
    assert.doesNotMatch(markup.slice(list.index + list[0].length), /synthetic_unfinished|is-unfinished/, 'the row has no duplicate outside its list');
    const predicate = host.__focusListPredicate;
    renderToStaticMarkup(createElement(Widget, props));
    assert.equal(host.__focusListPredicate, predicate, 'unchanged content renders keep the eligibility index reusable');
    data.history.segments[0].order.paused = true;
    assert.match(renderToStaticMarkup(createElement(Widget, props)), /Paused/);
  } finally { delete host.__focusListReply; delete host.__focusListInteractive; delete host.__focusListPredicate; await vite.close(); }
});
