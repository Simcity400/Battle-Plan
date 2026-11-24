import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import type { ConstructionLogManifestReply, ConstructionLogReply, ConstructionProject } from '../../generated/dashboard';
import { mergeWindows } from './constructionData.ts';
import { BRANCH, humansBy, mountModule, PENDING_HOUR, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';

const interval = { firstHour: 100, lastHour: 120 };
const project = (patch: Partial<ConstructionProject>): ConstructionProject => ({ id: '1', kind: 'building', source: 'census', queueOrdinal: 0, censusRead: patch.firstRead ?? 100, outputOrigin: null, tag: 'mod country', building: 1, repair: false,
  state: 1, toState: null, province: null, routeProvinces: null, baseLevel: null, targetLevel: null, costRaw: '100000',
  createdHour: 100, firstRead: 100, runs: [{ ...interval, factories: 5, remaining: 1, repairLevels: 0 }],
  earlierUnknown: false, built: [], unaccounted: [], possible: [], added: [], end: null, firstRepairTick: null, outputs: [], ...patch });
const PROJECTS = [
  project({ runs: [{ firstHour: 100, lastHour: 111, factories: 5, remaining: 3, repairLevels: 0 }, { firstHour: 112, lastHour: 120, factories: 8, remaining: 2, repairLevels: 0 }],
    built: [{ hour: 106, knownAt: 106, count: 1, repair: false, carried: false }] }),
  project({ id: '2', tag: 'other country', firstRead: 101, runs: [{ firstHour: 101, lastHour: 103, factories: 15, remaining: 1, repairLevels: 0 }],
    built: [{ hour: 104, knownAt: 104, count: 1, repair: false, carried: false }], end: { kind: 'finished', hour: 104, knownAt: 104, unknown: null } }),
  project({ id: '3', tag: 'pending country', repair: true, firstRead: 102, runs: [{ firstHour: 102, lastHour: 120, factories: 0, remaining: 0, repairLevels: 1 }] }),
];
const STATES = [{ id: 1, name: 'Synthetic location' }];

const clockAt = (hour: number) => ({ getHour: () => hour, getSnapshot: () => ({ hoursPerSecond: 24, playing: false }),
  subscribeFrame: () => () => {}, subscribe: () => () => {} });

async function renderer(groupsStub = false) {
  const manifest: ConstructionLogManifestReply = { costScale: 100000, windows: [interval], recorded: true, repairIcon: 'data:repair', warnings: [], missing: [],
    types: [{ id: 1, token: 'mod building', name: 'Synthetic building', icon: null }, { id: 2, token: 'mod second', name: 'Second building', icon: null }] };
  const reply: ConstructionLogReply = { window: 0, interval, projects: PROJECTS, states: STATES, warnings: [] };
  const state = { manifest, log: mergeWindows(manifest, new Map([[0, reply]]), 0) };
  const globals = globalThis as typeof globalThis & { __constructionArt?: typeof state };
  globals.__constructionArt = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/construction-log/ConstructionLogWidget.tsx', {
    '/widgets/construction-log/useConstructionLog.ts': `export const useConstructionLog = () => ({...globalThis.__constructionArt, hour: 110,
      error: null, retry() {}, onDrawn() {}});`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({retry() {}});',
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = () => ({labelAt: () => tag => tag === 'pending country'
      ? {name: 'Country name unavailable'} : {name: 'Named ' + tag, known: true}})`,
    '/charts/log/LogList.tsx': `import {createElement} from 'react'; export default props => createElement('div', {},
      props.rows.map(row => createElement('div', {key: row.key, 'data-row': row.item.project.id}, props.renderRow(row))));`,
    ...groupsStub ? { '/dashboard/groups.ts': 'export const countryMatch = () => ({ready: true, matches: () => true, known: () => true, reason: null});' } : {},
  });
  const render = (widget: Record<string, unknown>, clockHour = 110) => renderToStaticMarkup(createElement(module.default, { active: true, onDiagnostics() {},
    widget: { kind: 'construction-log', trackIds: ['main'], countries: { kind: 'all' }, options: {}, time: { mode: 'campaign' }, ...widget },
    runtime: { client: {}, clock: clockAt(clockHour), tracks: [{ id: 'main', source: { id: 'source', revision: '1' }, branch: { id: '1', ...interval }, axisOffset: 0 }] },
  } as unknown as WidgetProps));
  return { manifest, render, close: async () => { await close(); delete globals.__constructionArt; } };
}

const rowOf = (markup: string, id: string) => markup.match(new RegExp(`<div data-row="${id}">(.*?)</div><div data-row|<div data-row="${id}">(.*)$`))?.slice(1).find(Boolean) ?? '';


test('construction rows align the building and place, the country, a status pill and factories only while in the queue', async () => {
  const { manifest, render, close } = await renderer();
  try {
    const markup = render({});
    const building = rowOf(markup, '1');
    assert.match(building, /class="construction-name">Synthetic building<[^]*class="construction-place"[^]*>Synthetic location</);
    assert.match(building, /class="construction-country">.*?log-flag.*?<span class="construction-ellipsis">Named mod country</);
    assert.match(building, /class="pill accent"><span>Building</);
    assert.match(building, /class="construction-levels"><span>1 of 4<\/span>/);
    assert.match(building, /data-tooltip="Factories on it"><svg[^]*<\/svg><span>5<\/span>/);
    const finished = rowOf(markup, '2');
    assert.match(finished, /class="pill positive"><span>Built</);
    assert.doesNotMatch(finished, /Factories on it|>15</);
    const repair = rowOf(markup, '3');
    assert.match(repair, /class="log-art-overlay" src="data:repair"/);
    assert.match(repair, /class="pill accent"><span>Repairing</);
    assert.match(repair, /class="skeleton construction-name-pending"/);
    assert.match(repair, /class="log-value is-muted" data-tooltip="Factories on it"><svg[^]*<span>0<\/span>/);
    assert.doesNotMatch(markup, /Country name unavailable/);
    assert.match(markup, /class="log-art"[^>]*><span class="art-fallback"/);
    assert.doesNotMatch(markup, /live-dot/);
    const live = render({ time: { mode: 'follow' } });
    assert.match(rowOf(live, '1'), /class="pill accent"><i class="live-dot"/);
    assert.doesNotMatch(rowOf(live, '2'), /live-dot/);
    assert.deepEqual([...render({ options: { queue: true } }).matchAll(/data-row="([^"]+)"/g)].map(match => match[1]).sort(), ['1', '3']);
    manifest.types[0].icon = 'http://artwork.localhost/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png';
    const art = render({});
    assert.match(art, /src="http:\/\/artwork\.localhost\/a{32}\.png"/);
    assert.doesNotMatch(art, /art-fallback/);
  } finally { await close(); }
});

test('following, factories and levels are painted at the clock’s hour, not the hour the rows were made at', async () => {
  const { render, close } = await renderer();
  try {
    const building = rowOf(render({ time: { mode: 'follow' } }, 115), '1');
    assert.match(building, /<span>8<\/span>/);
    assert.match(building, /class="construction-levels"><span>1 of 3<\/span>/);
    assert.match(rowOf(render({}, 115), '1'), /<span>5<\/span>/);
  } finally { await close(); }
});

test('one country is named by the header, so its rows leave the country out', async () => {
  const { render, close } = await renderer(true);
  try {
    const markup = render({ countries: { kind: 'tags', tags: ['mod country'] } });
    assert.match(markup, /class="log-widget construction-log is-one-country"/);
    assert.doesNotMatch(markup, /construction-country|construction-place-country/);
    assert.match(rowOf(markup, '1'), /class="construction-place"><span class="construction-ellipsis">Synthetic location</);
    assert.doesNotMatch(render({ countries: { kind: 'tags', tags: ['mod country', 'other country'] } }), /is-one-country/);
  } finally { await close(); }
});

test('a group\'s log lists a project only where its country is a member at the first read, else says the group built nothing', async () => {
  const span = { firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour };
  const manifest: ConstructionLogManifestReply = { costScale: 100000, windows: [span], recorded: true, repairIcon: null, warnings: [], missing: [],
    types: [{ id: 1, token: 'mod building', name: 'Synthetic building', icon: null }] };
  const reply: ConstructionLogReply = { window: 0, interval: span, warnings: [], states: STATES,
    projects: [project({ tag: 'ENG', createdHour: PENDING_HOUR, firstRead: PENDING_HOUR, runs: [{ firstHour: PENDING_HOUR, lastHour: BRANCH.lastHour, factories: 5, remaining: 1, repairLevels: 0 }] })] };
  const state = { manifest, log: mergeWindows(manifest, new Map([[0, reply]]), 0), groups: pendingGroups({ final: true }) };
  const host = globalThis as typeof globalThis & { __constructionPending?: typeof state }; host.__constructionPending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/construction-log/ConstructionLogWidget.tsx', {
    '/widgets/construction-log/useConstructionLog.ts': `export const useConstructionLog = () => ({ manifest: globalThis.__constructionPending.manifest,
      log: globalThis.__constructionPending.log, hour: ${BRANCH.lastHour}, error: null, retry() {}, onDrawn() {} });`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__constructionPending.groups, retry() {} });',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag, known: true }) });',
    '/charts/log/LogList.tsx': `import { createElement } from 'react'; export default props => createElement('div', {}, props.rows.length
      ? props.rows.map(row => createElement('div', { key: row.key, 'data-row': row.item.project.id })) : props.empty);`,
  });
  try {
    const props = { active: true, onDiagnostics() {},
      widget: { kind: 'construction-log', trackIds: ['main'], countries: humansBy('hour'), options: {}, time: { mode: 'campaign' } },
      runtime: { client: {}, clock: clockAt(BRANCH.lastHour), tracks: [{ id: 'main', source: { id: 'source', revision: '1' }, branch: BRANCH, axisOffset: 0 }] },
    } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    const none = render();
    assert.match(none, /No construction by these countries yet\./);
    assert.doesNotMatch(none, /data-row/);
    state.groups = pendingGroups({ final: true, humans: [{ tag: 'GER', firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour }, { tag: 'ENG', firstHour: PENDING_HOUR - 10, lastHour: BRANCH.lastHour }] });
    const listed = render();
    assert.match(listed, /data-row="1"/);
    assert.doesNotMatch(listed, /No construction/);
    state.groups = pendingGroups({ final: true, humans: [{ tag: 'ENG', firstHour: PENDING_HOUR + 1, lastHour: BRANCH.lastHour }] });
    assert.match(render(), /No construction by these countries yet\./);
  } finally { delete host.__constructionPending; await close(); }
});
