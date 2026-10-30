import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { IdeaHistoryReply } from '../../generated/dashboard';

test('spirit tiles keep names and focus access with reserved fallback artwork', async () => {
  const data: IdeaHistoryReply = { branchId: '1', tag: 'mod_country', unobserved: [], identityStarts: [100], modifiers: [], texts: [], warnings: [],
    categories: [{ token: 'mod_spirits', name: 'Mod spirits', kind: 'national_spirit', hidden: false, politicsTab: true, slots: [] }],
    ideas: [{ token: 'mod_spirit', category: 'mod_spirits', slot: null, law: false, designer: false, effects: [], targeted: [], equipment: [], research: [],
      traits: [], tooltips: [], effectsKnown: true, unknown: null }], spans: [{ token: 'mod_spirit', firstHour: 100, lastHour: 200, slot: null, character: null }],
    artwork: [{ token: 'mod_spirit', name: 'Mod spirit', description: null, baseDurationDays: null, icon: null }], holders: [] };
  const replacements: Record<string, string> = {
    '/dashboard/ideaHistory.ts': `export const useIdeaHistory = () => ({ track: { source: { id: 'source', revision: '1', metadata: { campaignKey: 'campaign' } },
      branch: { id: '1', firstHour: 100, lastHour: 200 }, axisOffset: 0 }, tag: 'mod_country', data: ${JSON.stringify(data)} });`,
    '/dashboard/useIdeaTooltips.ts': 'export const useIdeaTooltips = () => undefined;',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'spirit-art-fixture', enforce: 'pre',
      transform: (_source, id) => Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1],
    }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/national-spirits/NationalSpiritsWidget.tsx');
    const props = { widget: { time: { mode: 'follow' } }, runtime: { clock: { getHour: () => 150 }, tracks: [], client: {} },
      active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const markup = renderToStaticMarkup(createElement(Widget, props));
    assert.match(markup, /art-fallback/);
    assert.match(markup, /tabindex="0" aria-label="Mod spirit, held before /);
    assert.match(markup, /class="spirit-name">Mod spirit/);
    assert.doesNotMatch(markup, /✦|artwork unavailable|portrait not recorded|Country: /i);
  } finally { await vite.close(); }
});
