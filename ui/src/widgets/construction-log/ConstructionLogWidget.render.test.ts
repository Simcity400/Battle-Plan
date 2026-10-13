import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { ConstructionLogManifestReply, ConstructionLogReply, ConstructionProject } from '../../generated/dashboard';
import { mergeWindows } from './constructionData.ts';

const interval = { firstHour: 100, lastHour: 120 };
const project = (patch: Partial<ConstructionProject>): ConstructionProject => ({ id: 'building', kind: 'building', tag: 'mod country', building: 0, repair: false,
  state: 1, toState: null, province: null, routeProvinces: null, baseLevel: null, targetLevel: null, costRaw: '100000',
  createdHour: 100, firstRead: 100, runs: [{ ...interval, factories: 5, remaining: 1, repairLevels: 0 }],
  earlierUnknown: false, built: [], unaccounted: [], possible: [], added: [], end: null, firstRepairTick: null, ...patch });
const PROJECTS = [
  // Still building with five factories.
  project({}),
  // Finished at 104 with fifteen factories at its last read.
  project({ id: 'finished', tag: 'other country', firstRead: 101, runs: [{ firstHour: 101, lastHour: 103, factories: 15, remaining: 1, repairLevels: 0 }],
    built: [{ hour: 104, knownAt: 104, count: 1, repair: false }], end: { kind: 'finished', hour: 104, knownAt: 104, unknown: null } }),
  // Repairing, its country's name still being read.
  project({ id: 'repair', tag: 'pending country', repair: true, firstRead: 102, runs: [{ firstHour: 102, lastHour: 120, factories: 0, remaining: 0, repairLevels: 1 }] }),
];

async function renderer(groupsStub = false) {
  const manifest: ConstructionLogManifestReply = { costScale: 100000, windows: [interval], recorded: true, repairIcon: 'data:repair', warnings: [],
    types: [{ id: 1, token: 'mod building', name: 'Synthetic building', icon: null }, { id: 2, token: 'mod second', name: 'Second building', icon: null }] };
  const reply: ConstructionLogReply = { window: 0, interval, outputReads: [], outputUnavailable: [], projects: PROJECTS, outputs: [], warnings: [] };
  const state = { manifest, log: mergeWindows(manifest, new Map([[0, reply]]), 0) };
  const globals = globalThis as typeof globalThis & { __constructionArt?: typeof state };
  globals.__constructionArt = state;
  const replacements: Record<string, string> = {
    '/widgets/construction-log/useConstructionLog.ts': `export const useConstructionLog = () => ({...globalThis.__constructionArt, hour: 110,
      retry() {}, onDrawn() {}});`,
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => 110;',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({retry() {}});',
    // A name not read yet has no \`known\`; the others were read.
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = () => ({labelAt: () => tag => tag === 'pending country'
      ? {name: 'Country name unavailable'} : {name: 'Named ' + tag, known: true}})`,
    '/dashboard/useStateNamesAt.ts': 'export const useStateNamesAt = () => (_hour, state) => ({state, name: "Synthetic location"});',
    '/charts/log/LogList.tsx': `import {createElement} from 'react'; export default props => createElement('div', {},
      props.rows.map(row => createElement('div', {key: row.key, 'data-row': row.item.project.id}, props.renderRow(row))));`,
    ...groupsStub ? { '/dashboard/groups.ts': 'export const countryMatch = () => ({ready: true, matches: () => true, reason: null});' } : {},
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'construction-art', enforce: 'pre',
      transform: (_source, id) => Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1],
    }] });
  const Widget = (await vite.ssrLoadModule('/ui/src/widgets/construction-log/ConstructionLogWidget.tsx')).default;
  const render = (widget: Record<string, unknown>) => renderToStaticMarkup(createElement(Widget, { active: true, onDiagnostics() {},
    widget: { kind: 'construction-log', trackIds: ['main'], countries: { kind: 'all' }, options: {}, time: { mode: 'campaign' }, ...widget },
    runtime: { client: {}, clock: { getHour: () => 110 }, tracks: [{ id: 'main', source: { id: 'source', revision: '1' }, branch: { id: '1', ...interval }, axisOffset: 0 }] },
  } as unknown as WidgetProps));
  return { manifest, render, close: async () => { await vite.close(); delete globals.__constructionArt; } };
}

const rowOf = (markup: string, id: string) => markup.match(new RegExp(`<div data-row="${id}">(.*?)</div><div data-row|<div data-row="${id}">(.*)$`))?.slice(1).find(Boolean) ?? '';

test('construction filters sit on one line, with a searchable building-type list instead of a chip per type', async () => {
  const { render, close } = await renderer();
  try {
    const markup = render({});
    assert.match(markup, /class="log-chips construction-filters"/);
    assert.match(markup, /aria-label="Projects"[^]*>New and repairs<[^]*>New buildings<[^]*>Repairs</);
    assert.match(markup, /aria-label="Queue"[^]*aria-pressed="false"[^>]*>In the queue</);
    assert.match(markup, /aria-label="Choose building types"[^]*>Every type</);
    assert.doesNotMatch(markup, /Second building/, 'no chip for a type no row shows');
    assert.match(render({ options: { queue: true, types: 'mod_second' } }), /aria-pressed="true"[^>]*>In the queue<[^]*>1 selected</);
    assert.doesNotMatch(render({ options: { chips: false } }), /construction-filters/);
  } finally { await close(); }
});

test('construction rows align the building and place, the country, a status pill and factories only while in the queue', async () => {
  const { manifest, render, close } = await renderer();
  try {
    const markup = render({});
    const building = rowOf(markup, 'building');
    assert.match(building, /class="construction-name">Synthetic building<[^]*class="construction-place"[^]*>Synthetic location</);
    assert.match(building, /class="construction-country">.*?log-flag.*?<span class="construction-ellipsis">Named mod country</);
    assert.match(building, /class="pill accent"><span>Building</);
    assert.match(building, /data-tooltip="Factories on it"><svg[^]*<\/svg>5</);
    // Ended: a finished pill and no factories, however many its last read had.
    const finished = rowOf(markup, 'finished');
    assert.match(finished, /class="pill positive"><span>Built</);
    assert.doesNotMatch(finished, /Factories on it|>15</);
    // The game's repair frame stays over the art; a name still being read is a placeholder.
    const repair = rowOf(markup, 'repair');
    assert.match(repair, /class="log-art-overlay" src="data:repair"/);
    assert.match(repair, /class="pill accent"><span>Repairing</);
    assert.match(repair, /class="skeleton construction-name-pending"/);
    assert.doesNotMatch(markup, /Country name unavailable/);
    assert.match(markup, /class="log-art"[^>]*><span class="art-fallback"/);
    assert.doesNotMatch(markup, /live-dot/);
    // Live: in-queue pills pulse, finished ones do not.
    const live = render({ time: { mode: 'follow' } });
    assert.match(rowOf(live, 'building'), /class="pill accent"><i class="live-dot"/);
    assert.doesNotMatch(rowOf(live, 'finished'), /live-dot/);
    // Only projects still in the queue.
    assert.deepEqual([...render({ options: { queue: true } }).matchAll(/data-row="([^"]+)"/g)].map(match => match[1]).sort(), ['building', 'repair']);
    manifest.types[0].icon = 'data:image/png;base64,synthetic';
    const art = render({});
    assert.match(art, /src="data:image\/png;base64,synthetic"/);
    assert.doesNotMatch(art, /art-fallback/);
  } finally { await close(); }
});

test('one country is named by the header, so its rows leave the country out', async () => {
  const { render, close } = await renderer(true);
  try {
    const markup = render({ countries: { kind: 'tags', tags: ['mod country'] } });
    assert.match(markup, /class="log-widget construction-log is-one-country"/);
    assert.doesNotMatch(markup, /construction-country|construction-place-country/);
    assert.match(rowOf(markup, 'building'), /class="construction-place"><span class="construction-ellipsis">Synthetic location</);
    assert.doesNotMatch(render({ countries: { kind: 'tags', tags: ['mod country', 'other country'] } }), /is-one-country/);
  } finally { await close(); }
});
