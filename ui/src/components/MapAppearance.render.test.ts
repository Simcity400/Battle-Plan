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
  const audio = { master: 100, ambient: 50, units: 100, combat: 100, interface: 100, voice: 100 };
  const inherits = { graphics: {}, audio: {}, counters: null, visualMods: null };
  const reply = { settings: inherits, inheritedSettings: { graphics, audio, counters: { kind: 'all' }, visualMods: null },
    mods: [], recordedMods: [], appliedMods: [], appliedNotes: [], sampleCounts: [1, 4], gameGraphics: graphics, gameSettingsError: null, saveWarning: null };
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
    assert.match(inherited, /aria-label="Trees"[^>]*aria-checked="false"/);
    assert.match(inherited, /aria-label="Rivers"[^>]*aria-checked="true"/);
    assert.match(inherited, /<label class="map-option-row" for="([^"]+)"[^>]*><span>Rivers<\/span><span class="map-option-control"><input id="\1"/, 'clicking a setting label toggles its switch');
    const one = render({ ...inherits, graphics: { rivers: false } });
    assert.match(one, /Use global defaults<\/button>/);
    assert.equal(one.match(/data-overridden/g)?.length, 1);
    assert.match(one, /aria-label="Reset rivers to global defaults" data-tooltip="Reset rivers to global defaults"/);
    assert.match(one, /aria-label="Rivers"[^>]*aria-checked="false"/);
    assert.match(one, /aria-label="Trees"[^>]*aria-checked="false"/);
    assert.match(one, /Advanced<\/summary>[\s\S]*aria-label="Sea borders"/);
    assert.match(one, /Advanced<\/summary>[\s\S]*aria-label="Generals captured"/);
    assert.match(render({ ...inherits, counters: { kind: 'players' } }), /Use global defaults<\/button>/);
    const silent = render({ ...inherits, audio: { master: 0 } });
    assert.match(silent, /<output class="map-volume">0<\/output><input[^>]*aria-label="Master volume"/, 'a genuine zero volume is shown as zero');
    host.__mapGraphics = { reply: null, draft: null };
    const unread = renderToStaticMarkup(createElement(MapAppearance, { source, client: { mapSettings: async () => reply } as never }));
    assert.doesNotMatch(unread, /role="switch" aria-label="Rivers"/, 'an unread switch is not shown as off');
    assert.match(unread, /aria-label="Rivers: Loading…">—</);
    assert.match(unread, /<span class="map-volume-control is-unknown"><output class="map-volume" aria-label="Master volume not known yet">—<\/output>/, 'an unread volume is not shown as zero');
  } finally { await vite.close(); delete host.__mapGraphics; }
});

test('every installed mod can be chosen and a chosen mod no longer installed keeps its name', async () => {
  const settings = { graphics: {}, audio: {}, counters: null, visualMods: [{ id: 'workshop/9', name: 'Gone Terrain' }] };
  const reply = { settings, inheritedSettings: { graphics: {}, audio: {}, counters: { kind: 'all' }, visualMods: null },
    mods: [{ id: 'workshop/1', name: 'Total Conversion', recordedId: '1', source: 'F:/workshop/1', problem: null },
      { id: 'mod/broken.mod', name: 'Broken Pack', recordedId: 'broken.mod', source: '', problem: 'its folder missing cannot be read' }],
    recordedMods: [{ recordedId: '1', name: 'Total Conversion' }], appliedMods: [], appliedNotes: ['Visual mod Gone Terrain is not installed, so it is not shown.'],
    sampleCounts: [1, 4], gameGraphics: null, gameSettingsError: null, saveWarning: null };
  const host = globalThis as typeof globalThis & { __mapGraphics?: { reply: unknown; draft: unknown } };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'map-mods-fixture', enforce: 'pre',
      transform(source, id) {
        if (!id.replaceAll('\\', '/').endsWith('/components/MapAppearance.tsx')) return;
        return source.replace('useState<MapSettingsReply | null>(null)', 'useState(globalThis.__mapGraphics.reply)')
          .replace('useState<CampaignMapSettings | null>(null)', 'useState(globalThis.__mapGraphics.draft)')
          .replace('useState<MapFailure | null>(null)', 'useState(globalThis.__mapGraphics.error ?? null)');
      },
    }] });
  try {
    const { default: MapAppearance } = await vite.ssrLoadModule('/ui/src/components/MapAppearance.tsx') as typeof import('./MapAppearance');
    const failed = { text: 'Couldn’t save that change.', detail: 'denied', write: true, retry() {} };
    host.__mapGraphics = { reply, draft: { ...settings, visualMods: [...settings.visualMods, { id: 'workshop/1', name: 'Total Conversion' }] }, error: failed } as never;
    const unsaved = renderToStaticMarkup(createElement(MapAppearance, { global: true, source: null, client: { mapSettings: async () => reply } as never }));
    const modsBlock = unsaved.slice(unsaved.indexOf('id="graphics-visual-mods"'));
    assert.match(modsBlock, /Couldn’t save your change to visual mods\.[\s\S]*Try again/, 'a failed mod switch is reported in Visual mods, beside the switch');
    assert.doesNotMatch(unsaved.slice(0, unsaved.indexOf('id="graphics-display"')), /Couldn’t save/, 'not up in Game settings');
    assert.match(modsBlock, /Total Conversion<small>In the recording<\/small><small class="map-option-unsaved">[\s\S]*?Not saved<\/small>/, 'the switch that wasn’t saved says so');
    assert.doesNotMatch(modsBlock, /Gone Terrain<small>No longer installed<\/small><small class="map-option-unsaved">/, 'a mod whose choice was saved isn’t marked');
    host.__mapGraphics = { reply: { ...reply, settings: { ...settings, graphics: { trees: true } } }, draft: { ...settings, graphics: { trees: false } }, error: failed } as never;
    const tree = renderToStaticMarkup(createElement(MapAppearance, { global: true, source: null, client: { mapSettings: async () => reply } as never }));
    assert.match(tree.slice(tree.indexOf('id="graphics-world"'), tree.indexOf('id="graphics-labels-and-tooltips"')), /Trees<span class="map-option-unsaved">[\s\S]*Couldn’t save your change to trees\./);
    host.__mapGraphics = { reply, draft: settings };
    const markup = renderToStaticMarkup(createElement(MapAppearance, { source: { id: 'source', revision: '1', metadata: { countries: [], playerCountryId: null } } as never, client: { mapSettings: async () => reply } as never }));
    const boxes = [...markup.matchAll(/<label class="map-mod-row">.*?(<input type="checkbox"[^>]*>)<\/label>/g)].map(match => match[1]);
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

test('anti-aliasing is unavailable only when the renderer has said its adapter lacks it', async () => {
  const settings = { graphics: { samples: 8 }, audio: {}, counters: null, visualMods: null };
  const reply = (sampleCounts: number[] | null) => ({ settings, inheritedSettings: { graphics: { samples: 4 }, audio: {}, counters: { kind: 'all' }, visualMods: null },
    mods: [], recordedMods: [], appliedMods: [], appliedNotes: [], sampleCounts, gameGraphics: null, gameSettingsError: null, saveWarning: null });
  const host = globalThis as typeof globalThis & { __mapGraphics?: { reply: unknown; draft: unknown } };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'map-samples-fixture', enforce: 'pre',
      transform(source, id) {
        if (!id.replaceAll('\\', '/').endsWith('/components/MapAppearance.tsx')) return;
        return source.replace('useState<MapSettingsReply | null>(null)', 'useState(globalThis.__mapGraphics.reply)')
          .replace('useState<CampaignMapSettings | null>(null)', 'useState(globalThis.__mapGraphics.draft)');
      },
    }] });
  try {
    const { default: MapAppearance } = await vite.ssrLoadModule('/ui/src/components/MapAppearance.tsx') as typeof import('./MapAppearance');
    const render = (sampleCounts: number[] | null, draft: unknown = settings) => {
      host.__mapGraphics = { reply: reply(sampleCounts), draft };
      return renderToStaticMarkup(createElement(MapAppearance, { global: true, source: null, client: { mapSettings: async () => reply(sampleCounts) } as never }));
    };
    assert.match(render(null), /8× MSAA/);
    assert.doesNotMatch(render(null), /8× MSAA \(unavailable\)/);
    assert.doesNotMatch(render([1, 2, 4, 8]), /\(unavailable\)/);
    assert.match(render([1, 4]), /8× MSAA \(unavailable\)/);
    const unset = render(null, { ...settings, graphics: {} });
    assert.match(unset, />Default \(4× MSAA\)</, 'the unset choice names the value it falls back to');
    assert.doesNotMatch(unset, /Inherit/);
  } finally { await vite.close(); delete host.__mapGraphics; }
});

test('every scope applies a change at once and saves its own scope without losing other overrides', async () => {
  type Settings = import('../dashboard/client').CampaignMapSettings;
  type Reply = import('../dashboard/client').MapSettingsReply;
  type Target = import('../dashboard/client').MapSettingsTarget;
  const settings = { graphics: { dayNight: null, borders: false }, audio: { voice: 40 }, counters: null, visualMods: null } as unknown as Settings;
  const reply = { settings, inheritedSettings: { graphics: { dayNight: true }, audio: {}, counters: { kind: 'all' }, visualMods: null },
    mods: [], recordedMods: [], appliedMods: [], appliedNotes: [], sampleCounts: [1, 4], gameGraphics: null, gameSettingsError: null, saveWarning: null } as unknown as Reply;
  const host = globalThis as typeof globalThis & { __mapGraphics?: { reply: Reply; draft: Settings };
    __mapGraphicsActions?: { setGraphics: (key: string, value: unknown) => void; setCounters: (value: string) => void } };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'map-settings-apply-fixture', enforce: 'pre',
      transform(source, id) {
        if (!id.replaceAll('\\', '/').endsWith('/components/MapAppearance.tsx')) return;
        return source.replace('useState<MapSettingsReply | null>(null)', 'useState(globalThis.__mapGraphics.reply)')
          .replace('const [draft, setDraft] = useState<CampaignMapSettings | null>(null);',
            'const draft = globalThis.__mapGraphics.draft; const setDraft = value => { globalThis.__mapGraphics.draft = value; };')
          .replace("  const display = [selectRow('samples', ANTI_ALIASING, samples),",
            "  globalThis.__mapGraphicsActions = { setGraphics, setCounters }; const display = [selectRow('samples', ANTI_ALIASING, samples),");
      },
    }] });
  const stored = new Map<string, string>();
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, writable: true,
    value: { getItem: (key: string) => stored.get(key) ?? null, setItem: (key: string, value: string) => { stored.set(key, value); }, removeItem: (key: string) => { stored.delete(key); } } });
  try {
    const { default: MapAppearance } = await vite.ssrLoadModule('/ui/src/components/MapAppearance.tsx') as typeof import('./MapAppearance');
    const source = { id: 'source', revision: 'revision', metadata: { countries: [], playerCountryId: null } } as never;
    for (const scope of [{ global: true }, {}, { viewId: 'widget-map' }]) {
      const writes: { sourceId: string; revision: string; settings: Settings; target: Target }[] = [];
      const client = { async mapSettings(sourceId: string, revision: string, saved: Settings, _signal: AbortSignal, target: Target) {
        writes.push({ sourceId, revision, settings: saved, target });
        return { ...reply, settings: saved };
      } } as never;
      host.__mapGraphics = { reply, draft: settings };
      const render = () => renderToStaticMarkup(createElement(MapAppearance, { ...scope, source: scope.global ? null : source, client }));
      render();
      host.__mapGraphicsActions!.setGraphics('dayNight', false);
      await Promise.resolve();
      assert.equal(writes.length, 1, 'a change applies as soon as it is chosen');
      const markup = render();
      assert.match(markup, /aria-label="Day\/night cycle"[^>]*aria-checked="false"/);
      assert.doesNotMatch(markup, />(Apply|Cancel)<\/button>/, 'no scope has an Apply step');
      assert.deepEqual(writes, [{ sourceId: scope.global ? '' : 'source', revision: scope.global ? '' : 'revision',
        settings: { ...settings, graphics: { ...settings.graphics, dayNight: false } },
        target: scope.global ? { scope: 'global' } : scope.viewId ? { scope: 'widget', id: scope.viewId } : { scope: 'campaign' } }]);
    }
    const client = { async mapSettings(_id: string, _revision: string, saved: Settings) { return { ...reply, settings: saved }; } } as never;
    const campaign = { id: 'source', revision: 'revision', metadata: { campaignKey: 'ab12', countries: [], branches: [], playerCountryId: null } };
    const counters = (component: typeof MapAppearance, campaign: unknown, start: unknown, ...choices: string[]) => {
      host.__mapGraphics = { reply, draft: { ...settings, counters: start } as unknown as Settings };
      for (const choice of choices) {
        renderToStaticMarkup(createElement(component, { source: campaign as never, client }));
        host.__mapGraphicsActions!.setCounters(choice);
      }
      return host.__mapGraphics.draft.counters;
    };
    assert.deepEqual(counters(MapAppearance, campaign, { kind: 'countries', tags: ['GER', 'ITA'] }, 'players'), { kind: 'players' });
    assert.deepEqual(counters(MapAppearance, campaign, { kind: 'countries', tags: ['GER', 'ITA'] }, 'players', 'countries'), { kind: 'countries', tags: ['GER', 'ITA'] },
      'returning to chosen countries brings the chosen set back');
    assert.deepEqual(counters(MapAppearance, campaign, { kind: 'countries', tags: ['GER', 'ITA'] }, '', 'all', 'countries'), { kind: 'countries', tags: ['GER', 'ITA'] });
    const other = { id: 'other', revision: 'revision', metadata: { campaignKey: 'cd34', countries: [], branches: [], playerCountryId: null } };
    assert.deepEqual(counters(MapAppearance, other, { kind: 'players' }, 'countries'), { kind: 'countries', tags: [] }, 'another campaign keeps its own choice');
    vite.moduleGraph.invalidateAll();
    const { default: reopened } = await vite.ssrLoadModule('/ui/src/components/MapAppearance.tsx') as typeof import('./MapAppearance');
    assert.notEqual(reopened, MapAppearance, 'the app is reopened with nothing kept in memory');
    assert.deepEqual(counters(reopened, { ...campaign, id: 'reopened' }, { kind: 'players' }, 'countries'), { kind: 'countries', tags: ['GER', 'ITA'] },
      'the chosen set survives quitting and reopening the same campaign');
  } finally { await vite.close(); delete host.__mapGraphics; delete host.__mapGraphicsActions; delete (globalThis as { localStorage?: unknown }).localStorage; }
});
