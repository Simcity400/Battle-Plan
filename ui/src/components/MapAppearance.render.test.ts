import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

test('each map setting inherits on its own and offers its own reset', async () => {
  const graphics = { samples: 4, textureQuality: 0, fullResolution: true, highGfxShaders: true, waterReflections: true, shadows: true, dayNight: true, natoCounters: false,
    water: true, borders: true, landBorders: true, seaBorders: true, demilitarizedZones: true, straitCrossings: true, rivers: true, trees: false, railways: true, rails: true,
    railwayBridges: true, supplyTrains: true, citySprawl: true, buildings: true, mapTable: true, nuclearStrikes: true, bombardmentExplosions: true, missileExplosions: true, mapText: true, textFakeDivisions: true,
    textRailwayGuns: true, textBombardments: true, textGenerals: true, textParadrops: true, textResistance: true, textBuildings: true, weather: true, sky: true, countryNames: true, units: true, unitFigures: true,
    aircraft: true, airPatrols: true, airReconnaissance: true, airSupport: true, airBombing: true, airCombat: true, airKamikaze: true, airSupply: true, airMissiles: true, battlePlans: true, frontLines: true, offensiveLines: true, plannedMovement: true, navalInvasions: true, paradrops: true, raidPaths: true, planLabels: true,
    showAlliedPlans: false, movementArrows: true, counters: true, landCounters: true, railwayGunCounters: true, navalCounters: true, portCounters: true, baseIcons: true,
    theatrePins: true, landCombatMarkers: true, navalMarkers: true, raidIcons: true, cityNames: true, tooltips: true };
  const audio = { master: 100, ambient: 50, units: 100, combat: 100, voice: 100 };
  const inherits = { graphics: {}, audio: {}, counters: null, visualMods: null };
  const reply = { settings: inherits, inheritedSettings: { graphics, audio, counters: { kind: 'all' }, visualMods: null },
    mods: [], recordedMods: [], appliedMods: [], appliedNotes: [], sampleCounts: [1, 4], gameGraphics: graphics, gameSettingsError: null };
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
    const { default: MapAppearance, inheritingAll } = await vite.ssrLoadModule('/ui/src/components/MapAppearance.tsx') as typeof import('./MapAppearance');
    // Reverting clears every choice in the stored shape, keeping visual mods, so reverting
    // settings that already inherit everything is no change to apply.
    const mods = [{ id: 'workshop/1', name: 'Terrain' }];
    const stored = { graphics: { rivers: null, trees: false }, audio: { voice: 40.5 }, counters: { kind: 'players' }, visualMods: mods } as never;
    assert.deepEqual(inheritingAll(stored), { graphics: { rivers: null, trees: null }, audio: { voice: null }, counters: null, visualMods: mods });
    const already = { graphics: { rivers: null, trees: null }, audio: { voice: null }, counters: null, visualMods: null } as never;
    assert.equal(JSON.stringify(inheritingAll(already)), JSON.stringify(already));
    const source = { id: 'source', revision: '1', metadata: { countries: [], playerCountryId: null } } as never;
    const render = (draft: unknown) => {
      host.__mapGraphics = { reply, draft };
      return renderToStaticMarkup(createElement(MapAppearance, { source, client: { mapSettings: async () => reply } as never }));
    };
    const inherited = render(inherits);
    assert.doesNotMatch(inherited, /Use global defaults<\/button>/, 'settings already inherited have nothing to revert');
    assert.doesNotMatch(inherited, /data-overridden/);
    // Inherited switches show the parent's value: trees off, rivers on.
    assert.match(inherited, /aria-label="Trees"[^>]*aria-checked="false"/);
    assert.match(inherited, /aria-label="Rivers"[^>]*aria-checked="true"/);
    const one = render({ ...inherits, graphics: { rivers: false } });
    assert.match(one, /Use global defaults<\/button>/);
    // Only the changed row is marked and offers to inherit again.
    assert.equal(one.match(/data-overridden/g)?.length, 1);
    assert.match(one, /aria-label="Rivers: use global defaults"/);
    assert.match(one, /aria-label="Rivers"[^>]*aria-checked="false"/);
    assert.match(one, /aria-label="Trees"[^>]*aria-checked="false"/);
    // Parts sit under Advanced, beside their main switch.
    assert.match(one, /Advanced<\/summary>[\s\S]*aria-label="Sea borders"/);
    assert.match(one, /Advanced<\/summary>[\s\S]*aria-label="Generals captured"/);
    assert.match(render({ ...inherits, counters: { kind: 'players' } }), /Use global defaults<\/button>/);
  } finally { await vite.close(); delete host.__mapGraphics; }
});

test('every installed mod can be chosen and a chosen mod no longer installed keeps its name', async () => {
  const settings = { graphics: {}, audio: {}, counters: null, visualMods: [{ id: 'workshop/9', name: 'Gone Terrain' }] };
  const reply = { settings, inheritedSettings: { graphics: {}, audio: {}, counters: { kind: 'all' }, visualMods: null },
    mods: [{ id: 'workshop/1', name: 'Total Conversion', recordedId: '1', source: 'F:/workshop/1', problem: null },
      { id: 'mod/broken.mod', name: 'Broken Pack', recordedId: 'broken.mod', source: '', problem: 'its folder missing cannot be read' }],
    recordedMods: [{ recordedId: '1', name: 'Total Conversion' }], appliedMods: [], appliedNotes: ['Visual mod Gone Terrain is not installed, so it is not shown.'],
    sampleCounts: [1, 4], gameGraphics: null, gameSettingsError: null };
  const host = globalThis as typeof globalThis & { __mapGraphics?: { reply: unknown; draft: unknown } };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'map-mods-fixture', enforce: 'pre',
      transform(source, id) {
        if (!id.replaceAll('\\', '/').endsWith('/components/MapAppearance.tsx')) return;
        return source.replace('useState<MapSettingsReply | null>(null)', 'useState(globalThis.__mapGraphics.reply)')
          .replace('useState<CampaignMapSettings | null>(null)', 'useState(globalThis.__mapGraphics.draft)');
      },
    }] });
  try {
    const { default: MapAppearance } = await vite.ssrLoadModule('/ui/src/components/MapAppearance.tsx') as typeof import('./MapAppearance');
    host.__mapGraphics = { reply, draft: settings };
    const markup = renderToStaticMarkup(createElement(MapAppearance, { source: { id: 'source', revision: '1', metadata: { countries: [], playerCountryId: null } } as never, client: { mapSettings: async () => reply } as never }));
    const boxes = markup.match(/<input type="checkbox"[^>]*>/g)?.filter(box => !box.includes('role="switch"')) ?? [];
    assert.equal(boxes.length, 3, 'two installed mods and the missing selection');
    assert.ok(boxes.every(box => !/disabled/.test(box)), 'no installed mod is refused by an eligibility check');
    assert.match(markup, /Gone Terrain<small>No longer installed<\/small>/);
    assert.match(markup, /Broken Pack<small>its folder missing cannot be read<\/small>/);
    assert.match(markup, /Total Conversion<small>In the recording<\/small>/);
    assert.match(markup, /Recorded with Total Conversion\./);
    assert.match(markup, /Gone Terrain is not installed/);
    assert.match(markup, />Refresh</);
  } finally { await vite.close(); delete host.__mapGraphics; }
});
