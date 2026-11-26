import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { AircraftCatalogueReply } from '../generated/dashboard';

const reply: AircraftCatalogueReply = {
  matching: 3, found: 3, first: 0,
  entries: [
    { name: 'ENG_fighter_equipment_1_entity', kind: 'entity', titles: ['Spitfire', 'Carrier Spitfire'],
      references: [{ kind: 'ladder', key: 'fighter_equipment_1', prefix: 'ENG' }, { kind: 'pool', bucket: 'ENG', equipment: 'small_plane_airframe', file: 'f' },
        { kind: 'pool', bucket: 'default', equipment: 'small_plane_airframe', file: 'f' }], refusal: null, origin: { kind: 'game', dlc: null } },
    { name: 'jet_mesh', kind: 'mesh', titles: [], references: [{ kind: 'pool', bucket: 'default', equipment: 'jet_equipment', file: 'f' }], refusal: null,
      origin: { kind: 'mod', id: 'ugc_1', name: 'Modern Wings' } },
    { name: 'ghost_plane_entity', kind: 'entity', titles: [], references: [{ kind: 'cloneOf', of: 'light_plane_entity' }],
      refusal: { kind: 'noGeometry', reason: 'it names no pdxmesh' }, origin: { kind: 'game', dlc: 'By Blood Alone' } },
  ],
  refusals: [{ kind: 'noGeometry', label: 'no geometry to draw', count: 1 }],
  origins: [{ origin: { kind: 'game', dlc: null }, count: 2 }, { origin: { kind: 'mod', id: 'ugc_1', name: 'Modern Wings' }, count: 1 }],
  warnings: ['1 descriptor files do not parse even when closed'],
};

const view = { version: 1, origin: null, onOrigin: () => {}, query: '', onQuery: () => {}, chosen: null, status: null, onChoose: () => {}, request: () => {}, retry: () => {} };

type ListProps = { items: readonly (number | 'refused')[]; isInteractive?: (item: number | 'refused', index: number) => boolean };

async function picker() {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'aircraft-list-props', enforce: 'pre',
      transform(source, id) {
        if (!id.replaceAll('\\', '/').endsWith('/components/AircraftPicker.tsx')) return;
        return source.replace("import { VirtualList } from './VirtualList';", 'import { VirtualList as Listed } from "./VirtualList";\n'
          + 'const VirtualList = (props) => { globalThis.__aircraftList = props; return Listed(props); };');
      },
    }] });
  const module = await vite.ssrLoadModule('/ui/src/components/AircraftPicker.tsx') as typeof import('./AircraftPicker');
  const catalogue = await vite.ssrLoadModule('/ui/src/dashboard/aircraftCatalogue.ts') as typeof import('../dashboard/aircraftCatalogue');
  return { vite, module, catalogue };
}

test('the catalogue lists titles, countries and where each aircraft comes from, and refused models say why under their own heading', async () => {
  const { vite, module, catalogue } = await picker();
  try {
    const pages = new catalogue.AircraftPages('fixture');
    pages.begin(0);
    pages.accept(0, { ...reply, matching: 4, found: 4, refusals: [...reply.refusals, { kind: 'declaration', label: 'a declaration it uses cannot be played', count: 1 }],
      entries: [...reply.entries, { name: 'odd_plane_entity', kind: 'entity', titles: ['Odd plane'], references: [], origin: null,
        refusal: { kind: 'declaration', reason: 'its animation list does not parse' } }] });
    const markup = renderToStaticMarkup(createElement(module.AircraftCatalogueView, {
      ...view, pages, chosen: { name: 'ENG_fighter_equipment_1_entity', title: 'Spitfire' }, status: { aircraft: 'ENG_fighter_equipment_1_entity', state: { kind: 'ready' } },
    }));
    assert.match(markup, /aria-pressed="true">All<span class="aircraft-origin-count">4<\/span>/);
    assert.match(markup, /aria-pressed="false">Vanilla<span class="aircraft-origin-count">2<\/span>/);
    assert.match(markup, /data-origin="mod" aria-pressed="false">Modern Wings<span class="aircraft-origin-count">1<\/span>/);
    assert.match(markup, /class="aircraft-row" aria-pressed="true" data-origin="game" data-tooltip="Click again to go back to the free camera"><span class="aircraft-tile" aria-hidden="true">ENG<\/span>/);
    assert.match(markup, /<span class="aircraft-row-name">Spitfire<\/span><\/span><small class="aircraft-row-detail"><span class="aircraft-row-source">Vanilla<\/span><span class="aircraft-row-also" data-tooltip="Also Carrier Spitfire">Also Carrier Spitfire<\/span>/);
    assert.match(markup, /<span class="aircraft-row-state"><svg/, 'the aircraft flown is ticked');
    for (const internal of ['ENG_fighter_equipment_1_entity', 'jet_mesh', 'ghost_plane_entity', 'small_plane_airframe', 'fighter_equipment_1', 'light_plane_entity', 'ugc_1', 'pool for'])
      assert.ok(!markup.includes(internal), `${internal} is a registered name or content key, not shown anywhere`);
    assert.match(markup, /data-origin="mod"><span class="aircraft-tile" aria-hidden="true"><svg[^]*?<span class="aircraft-row-name">Jet<\/span><small class="aircraft-badge">No animation<\/small><\/span><small class="aircraft-row-detail"><span class="aircraft-row-source">Modern Wings<\/span><\/small>/);
    assert.match(markup, /data-tooltip="1 with a missing 3D model\n1 that BattlePlan can’t read" data-detail="1: no geometry to draw\n1: a declaration it uses cannot be played">Can’t be flown · 2<\/p>/);
    assert.ok(markup.indexOf('Can’t be flown · 2') < markup.indexOf('Ghost plane'), 'the heading leads the refused rows');
    assert.match(markup, /<button type="button" class="aircraft-row" disabled="" aria-pressed="false" data-origin="game">[^]*?<span class="aircraft-row-name">Ghost plane<\/span><\/span><small class="aircraft-row-detail"><span class="aircraft-row-source" data-tooltip="From the By Blood Alone DLC">Vanilla<\/span><\/small><small class="aircraft-row-refusal" data-detail="it names no pdxmesh">Can’t be flown: its 3D model is missing\.<\/small>/,
      'a refused row still says where it comes from');
    assert.ok(!markup.replace(/<[^>]*>/g, '').includes('pdxmesh'), 'the raw engine reason is detail only');
    assert.match(markup, /data-origin="unknown">[^]*?<span class="aircraft-row-name">Odd plane<\/span><\/span><small class="aircraft-row-detail"><span class="aircraft-row-source" data-tooltip="No game or mod file registers this model">Source unknown<\/span><\/small><small class="aircraft-row-refusal" data-detail="its animation list does not parse">Can’t be flown: BattlePlan can’t read this model\.<\/small>/);
    assert.match(markup, /data-detail="1 descriptor files do not parse even when closed">Some game files couldn’t be read, so this list may be incomplete\.</);
    for (const gone of ['Choose none', 'All models', 'Flying Spitfire', 'aircraft-picker-hint'])
      assert.ok(!markup.includes(gone), `${gone} is no longer part of the list`);
  } finally { await vite.close(); }
});

test('a chosen source is the pressed tab, a DLC model counts as the base game, and the flown row shows loading and refusal', async () => {
  const { vite, module, catalogue } = await picker();
  try {
    const pages = new catalogue.AircraftPages('sources');
    pages.begin(0);
    pages.accept(0, { ...reply, refusals: [], entries: [{ ...reply.entries[0], origin: { kind: 'game', dlc: 'By Blood Alone' } }], matching: 1 });
    const chosen = { name: 'ENG_fighter_equipment_1_entity', title: 'Spitfire' };
    const render = (status: Parameters<typeof module.statusText>[1], origin: { mod: string } | 'game' | null = null) =>
      renderToStaticMarkup(createElement(module.AircraftCatalogueView, { ...view, pages, origin, chosen, status }));
    const modded = render({ aircraft: chosen.name, state: { kind: 'ready' } }, { mod: 'ugc_1' });
    assert.match(modded, /aria-pressed="false">All</);
    assert.match(modded, /data-origin="mod" aria-pressed="true">Modern Wings</);
    assert.match(render(null), /<span class="aircraft-row-source" data-tooltip="From the By Blood Alone DLC">Vanilla<\/span>/);
    assert.match(render(null), /<span class="spinner" role="status" aria-label="Loading"><\/span>/, 'a choice not yet answered is loading');
    const refused = render({ aircraft: chosen.name, state: { kind: 'refused', reason: 'its mesh did not load' } });
    assert.match(refused, /class="aircraft-row-state is-alert"/);
    assert.match(refused, /role="alert" data-detail="its mesh did not load"><svg[^]*?<span>Spitfire can’t be flown\. Flying as a free camera\.<\/span>/);
    const flagged = renderToStaticMarkup(createElement(module.AircraftCatalogueView, { ...view, pages,
      flag: (tag: string) => createElement('i', { className: 'stub-flag' }, tag) }));
    assert.match(flagged, /<span class="aircraft-tile has-flag" aria-hidden="true"><i class="stub-flag">ENG<\/i><\/span>/,
      'a row with a country shows the flag the rest of the app draws');
    const none = renderToStaticMarkup(createElement(module.AircraftCatalogueView, { ...view, pages }));
    assert.match(none, /<p class="aircraft-picker-hint" role="status">Flying as a free camera\. Choose an aircraft to pilot it\.<\/p>/);
    const unsourced = new catalogue.AircraftPages('unsourced');
    unsourced.begin(0);
    unsourced.accept(0, { ...reply, origins: [] });
    assert.doesNotMatch(renderToStaticMarkup(createElement(module.AircraftCatalogueView, { ...view, pages: unsourced })), /aircraft-origins/,
      'no tabs when no aircraft has a known source');
  } finally { await vite.close(); }
});

test('the status line follows the chosen aircraft only, and a picker without a campaign is disabled', async () => {
  const { vite, module } = await picker();
  try {
    const { statusText } = module;
    assert.deepEqual(statusText(null, { aircraft: null, state: { kind: 'ready' } }), { text: module.FREE_CAMERA, alert: false });
    assert.match(module.FREE_CAMERA, /Flying as a free camera/);
    const fighter = { name: 'a_entity', title: 'Fighter I' };
    assert.equal(statusText(fighter, { aircraft: 'b_entity', state: { kind: 'ready' } }), null, 'another choice is still being answered');
    assert.deepEqual(statusText(fighter, { aircraft: 'a_entity', state: { kind: 'loading' } }),
      { text: 'Loading Fighter I… The free camera flies until it’s ready.', alert: false });
    assert.deepEqual(statusText(fighter, { aircraft: 'a_entity', state: { kind: 'refused', reason: 'its mesh did not load' } }),
      { text: 'Fighter I can’t be flown. Flying as a free camera.', alert: true, detail: 'its mesh did not load' });
    const trigger = renderToStaticMarkup(createElement(module.default, { client: {} as never, source: { id: 's', revision: '1' } as never,
      chosen: fighter, status: { aircraft: 'a_entity', state: { kind: 'ready' } }, onChoose: () => {}, host: null }));
    assert.match(trigger, /data-state="ready" aria-expanded="false" disabled="" aria-label="Aircraft: Fighter I" data-tooltip="Flying Fighter I"><svg[^]*<span>Fighter I<\/span><\/button>/);
    assert.doesNotMatch(trigger, /a_entity/);
    const freeTrigger = renderToStaticMarkup(createElement(module.default, { client: {} as never, source: { id: 's', revision: '1' } as never,
      chosen: null, status: null, onChoose: () => {}, host: null }));
    assert.match(freeTrigger, /data-state="free" aria-expanded="false" disabled="" aria-label="Aircraft: none, free camera" data-tooltip="Flying as a free camera\. Choose an aircraft to pilot it\."><svg[^]*<span>Free camera<\/span><\/button>/);
    assert.doesNotMatch(freeTrigger, /aircraft-picker"/, 'entering Fly with no aircraft leaves the panel closed');
    const markup = renderToStaticMarkup(createElement(module.default, { client: {} as never, source: null, chosen: null, status: null, onChoose: () => {}, host: null }));
    assert.match(markup, /disabled=""/);
    assert.match(markup, /Open a campaign to choose an aircraft/);
    assert.match(markup, /Free camera/);
  } finally { await vite.close(); }
});

test('a listing without its first page says it is reading, and a failed first page offers a retry', async () => {
  const { vite, module, catalogue } = await picker();
  try {
    const render = (pages: InstanceType<typeof catalogue.AircraftPages>) => renderToStaticMarkup(createElement(module.AircraftCatalogueView, {
      ...view, pages, version: 0, query: 'spit' }));
    const reading = new catalogue.AircraftPages('reading');
    reading.begin(0);
    assert.match(render(reading), /Reading aircraft…/);
    const { RefusedError } = await vite.ssrLoadModule('/ui/src/dashboard/queryError.ts');
    reading.fail(0, new RefusedError('Choose the game and user folders in Settings first'));
    const refused = render(reading);
    assert.match(refused, /class="failure-line aircraft-picker-failure" role="alert" data-detail="Choose the game and user folders in Settings first"><svg[^]*?<span>Choose the game and user folders in Settings first<\/span>.*Try again/);
    reading.fail(0, new Error('Synthetic aircraft decoder failure'));
    const failed = render(reading);
    assert.match(failed, /role="alert" data-detail="Synthetic aircraft decoder failure"><svg[^]*?<span>Couldn’t load aircraft\.<\/span>.*Try again/);
    assert.doesNotMatch(failed.replace(/<[^>]*>/g, ''), /Synthetic aircraft decoder failure/);
    const none = new catalogue.AircraftPages('none');
    none.begin(0);
    none.accept(0, { ...reply, matching: 0, entries: [] });
    assert.match(render(none), /No aircraft match the search\./);
  } finally { await vite.close(); }
});


test('refused rows keep their places in the list but are no keyboard destinations', async () => {
  const { vite, module, catalogue } = await picker();
  const host = globalThis as typeof globalThis & { __aircraftList?: ListProps };
  try {
    const pages = new catalogue.AircraftPages('keys', 3);
    pages.begin(0);
    pages.accept(0, reply);
    pages.begin(1);
    pages.fail(1, new Error('The data service stopped'));
    renderToStaticMarkup(createElement(module.AircraftCatalogueView, {
      ...view, pages, version: 2 }));
    const list = host.__aircraftList;
    assert.ok(list?.isInteractive, 'the list is told which rows are destinations');
    assert.deepEqual(list.items, [0, 1, 'refused', 2]);
    assert.deepEqual(list.items.map((item, index) => list.isInteractive!(item, index)), [true, true, false, false],
      'the refused heading and the refused ghost plane are skipped');
    assert.equal(catalogue.aircraftRowInteractive(pages, 4), true, "a failed page's retry is a destination");
    assert.equal(catalogue.aircraftRowInteractive(new catalogue.AircraftPages('unloaded'), 0), false, 'a row still loading has nothing to focus');
  } finally { await vite.close(); delete host.__aircraftList; }
});
