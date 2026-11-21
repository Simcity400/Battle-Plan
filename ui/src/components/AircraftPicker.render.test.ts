import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { AircraftCatalogueReply } from '../generated/dashboard';

const reply: AircraftCatalogueReply = {
  matching: 3, aircraft: 3, all: 1500, first: 0,
  entries: [
    { name: 'ENG_fighter_equipment_1_entity', kind: 'entity', titles: ['Spitfire'],
      references: [{ kind: 'ladder', key: 'fighter_equipment_1', prefix: 'ENG' }, { kind: 'pool', bucket: 'ENG', equipment: 'small_plane_airframe', file: 'f' },
        { kind: 'pool', bucket: 'default', equipment: 'small_plane_airframe', file: 'f' }], refusal: null },
    { name: 'jet_mesh', kind: 'mesh', titles: [], references: [{ kind: 'pool', bucket: 'default', equipment: 'jet_equipment', file: 'f' }], refusal: null },
    { name: 'ghost_plane_entity', kind: 'entity', titles: [], references: [{ kind: 'cloneOf', of: 'light_plane_entity' }],
      refusal: { kind: 'noGeometry', reason: 'it names no pdxmesh' } },
  ],
  refusals: [{ kind: 'noGeometry', label: 'no geometry to draw', count: 1 }],
  warnings: ['1 descriptor files do not parse even when closed'],
};

type ListProps = { items: readonly number[]; isInteractive?: (item: number, index: number) => boolean };

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

test('the catalogue lists titles, names, kinds and references, and refused models say why', async () => {
  const { vite, module, catalogue } = await picker();
  try {
    const pages = new catalogue.AircraftPages('fixture');
    pages.begin(0);
    pages.accept(0, reply);
    const markup = renderToStaticMarkup(createElement(module.AircraftCatalogueView, {
      pages, version: 1, scope: 'aircraft', onScope: () => {}, query: '', onQuery: () => {}, chosen: 'ENG_fighter_equipment_1_entity',
      status: { aircraft: 'ENG_fighter_equipment_1_entity', state: { kind: 'ready' } }, onChoose: () => {}, request: () => {}, retry: () => {},
    }));
    assert.match(markup, /Aircraft \(3\)/);
    assert.match(markup, new RegExp(`All models \\(${(1500).toLocaleString()}\\)`));
    assert.match(markup, /Spitfire/);
    assert.match(markup, /<code class="aircraft-row-name">ENG_fighter_equipment_1_entity<\/code>/);
    assert.match(markup, /ENG fighter_equipment_1 model; ENG pool for small_plane_airframe \+1 more/);
    assert.match(markup, /Mesh \(no animation\)/);
    assert.match(markup, /<button type="button" class="aircraft-row" disabled=""[^>]*>.*ghost_plane_entity.*it names no pdxmesh/);
    assert.match(markup, /Flying ENG_fighter_equipment_1_entity/);
    assert.match(markup, /Not flyable: 1 no geometry to draw/);
    assert.match(markup, /Content notes \(1\)/);
    assert.match(markup, /aria-pressed="false"[^>]*>Choose none/, 'an aircraft is chosen, so "Choose none" is not');
  } finally { await vite.close(); }
});

test('the status line follows the chosen aircraft only, and a picker without a campaign is disabled', async () => {
  const { vite, module } = await picker();
  try {
    const { statusText } = module;
    assert.deepEqual(statusText(null, { aircraft: null, state: { kind: 'ready' } }), { text: module.NO_AIRCRAFT, alert: false });
    assert.match(module.NO_AIRCRAFT, /No aircraft is chosen/);
    assert.equal(statusText('a', { aircraft: 'b', state: { kind: 'ready' } }), null, 'another choice is still being answered');
    assert.deepEqual(statusText('a', { aircraft: 'a', state: { kind: 'loading' } }), { text: 'Loading a...', alert: false });
    assert.deepEqual(statusText('a', { aircraft: 'a', state: { kind: 'refused', reason: 'its mesh did not load' } }),
      { text: 'a cannot be flown: its mesh did not load', alert: true });
    const markup = renderToStaticMarkup(createElement(module.default, { client: {} as never, source: null, chosen: null, status: null, onChoose: () => {}, host: null }));
    assert.match(markup, /disabled=""/);
    assert.match(markup, /Open a campaign to choose an aircraft/);
    assert.match(markup, /No aircraft chosen/);
  } finally { await vite.close(); }
});

test('a listing without its first page says it is reading, and a failed first page offers a retry', async () => {
  const { vite, module, catalogue } = await picker();
  try {
    const render = (pages: InstanceType<typeof catalogue.AircraftPages>) => renderToStaticMarkup(createElement(module.AircraftCatalogueView, {
      pages, version: 0, scope: 'all', onScope: () => {}, query: 'spit', onQuery: () => {}, chosen: null, status: null,
      onChoose: () => {}, request: () => {}, retry: () => {} }));
    const reading = new catalogue.AircraftPages('reading');
    reading.begin(0);
    assert.match(render(reading), /Reading the content&#x27;s aircraft/);
    const { RefusedError } = await vite.ssrLoadModule('/ui/src/dashboard/queryError.ts');
    reading.fail(0, new RefusedError('Choose the game and user folders in Settings first'));
    const refused = render(reading);
    assert.match(refused, /role="alert" data-detail="Choose the game and user folders in Settings first">Choose the game and user folders in Settings first.*Retry/);
    reading.fail(0, new Error('Synthetic aircraft decoder failure'));
    const failed = render(reading);
    assert.match(failed, /role="alert" data-detail="Synthetic aircraft decoder failure">Couldn’t load aircraft\..*Retry/);
    assert.doesNotMatch(failed.replace(/<[^>]*>/g, ''), /Synthetic aircraft decoder failure/);
    const none = new catalogue.AircraftPages('none');
    none.begin(0);
    none.accept(0, { ...reply, matching: 0, entries: [] });
    assert.match(render(none), /No model matches the search/);
  } finally { await vite.close(); }
});

test('the docked aircraft list is no popover, so the map stays shown while planes are tried', async () => {
  const { vite, module } = await picker();
  try {
    const markup = renderToStaticMarkup(createElement(module.AircraftPanel, { id: 'panel', onClose: () => {}, children: 'list' }));
    assert.doesNotMatch(markup, /\spopover(target)?=/);
    assert.match(markup, /^<section id="panel" class="aircraft-picker" aria-label="Aircraft">/);
    assert.match(markup, /aria-label="Close the aircraft list"/);
    const picker = renderToStaticMarkup(createElement(module.default, { client: {} as never, source: { id: 's', revision: '1' } as never,
      chosen: null, status: null, onChoose: () => {}, host: null }));
    assert.doesNotMatch(picker, /\spopover(target)?=/, 'the trigger opens the docked list, not a popover');
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
      pages, version: 2, scope: 'aircraft', onScope: () => {}, query: '', onQuery: () => {}, chosen: null, status: null,
      onChoose: () => {}, request: () => {}, retry: () => {} }));
    const list = host.__aircraftList;
    assert.ok(list?.isInteractive, 'the list is told which rows are destinations');
    assert.equal(list.items.length, 3);
    assert.deepEqual(list.items.map((item, index) => list.isInteractive!(item, index)), [true, true, false],
      'the refused ghost plane in row 3 is skipped');
    assert.equal(catalogue.aircraftRowInteractive(pages, 4), true, "a failed page's retry is a destination");
    assert.equal(catalogue.aircraftRowInteractive(new catalogue.AircraftPages('unloaded'), 0), false, 'a row still loading has nothing to focus');
  } finally { await vite.close(); delete host.__aircraftList; }
});
