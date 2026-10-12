import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

test('map graphics offer to revert only while they differ from what they revert to', async () => {
  const graphics = { samples: 4, textureQuality: 0, fullResolution: true, dayNight: true, natoCounters: false, showAlliedPlans: false, highGfxShaders: true,
    waterReflections: true, trees: true, shadows: true, rivers: true, citySprawl: true, buildings: true, units: true, weather: true, sky: true };
  const reply = { settings: { useGameGraphics: true, graphics, visualMods: null }, inheritedSettings: { graphics, visualMods: null },
    mods: [], recordedMods: [], sampleCounts: [1, 4], gameGraphics: graphics };
  const host = globalThis as typeof globalThis & { __mapGraphics?: { reply: unknown; draft: unknown } };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'map-graphics-fixture', enforce: 'pre',
      transform(source, id) {
        if (!id.replaceAll('\\', '/').endsWith('/components/MapAppearance.tsx')) return;
        return source.replace('useState<MapSettingsReply | null>(null)', 'useState(globalThis.__mapGraphics.reply)')
          .replace('useState<CampaignMapSettings | null>(null)', 'useState(globalThis.__mapGraphics.draft)');
      },
    }] });
  try {
    const { default: MapAppearance } = await vite.ssrLoadModule('/ui/src/components/MapAppearance.tsx') as typeof import('./MapAppearance');
    const source = { id: 'source', revision: '1' } as never;
    const render = (draft: unknown) => {
      host.__mapGraphics = { reply, draft };
      return renderToStaticMarkup(createElement(MapAppearance, { source, client: { mapSettings: async () => reply } as never }));
    };
    assert.doesNotMatch(render(reply.settings), /Use global defaults/, 'graphics already inherited have nothing to revert');
    assert.match(render({ ...reply.settings, useGameGraphics: false, graphics: { ...graphics, trees: false } }), /Use global defaults/);
  } finally { await vite.close(); delete host.__mapGraphics; }
});
