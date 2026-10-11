import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { tokensFromValues } from '../charts/tokens.ts';

const requests: unknown[] = [];
const CLIENT = {};
const shared = globalThis as typeof globalThis & { airRequests: unknown[]; airClient?: object; airTimeline: unknown; airChoices?: unknown; airDay?: unknown; airDayError?: string; airHeld?: unknown; airWindows?: unknown; airFactionName?: string; airFactionColours?: [string, string][]; airStyleReads?: { tag: string; hour: number }[][]; airStyles?: [string, number, [number, number, number]][]; airLookPlans?: { hour: number; segmentAt: (tag: string, hour: number) => number | null; hours?: ReadonlyMap<string, number>; exact: boolean }[]; airHoverSegment?: (tag: string, hour: number) => number | null; airUseState?: () => unknown; airUseEffect?: (effect: () => unknown, deps: readonly unknown[]) => void };
shared.airRequests = requests;
shared.airTimeline = { view: 'timeline', branchId: '1', firstHour: 0, lastHour: 24, available: true,
  days: [23, 24], sides: [], segments: {}, anchorReads: {}, rows: [], excluded: [], theatreReads: [], warnings: [], scale: 100000, dayHours: 24, badBelow: 40000, goodAbove: 60000 };
const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{
  name: 'air-hook-inputs', enforce: 'pre',
  transform(code, id) {
    if (id.replaceAll('\\', '/').endsWith('/dashboard/useAirSuperiorityColours.ts')) return code.replace("from './countryNamesAt.ts'", "from 'air-hook-inputs'").replace("from './useCountriesAt.ts'", "from 'air-hook-inputs'");
    if (!id.replaceAll('\\', '/').endsWith('/dashboard/useAirSuperiority.ts')) return;
    code = code.replace("import { useCallback, useEffect, useMemo, useRef, useState } from 'react';", "import { useCallback, useMemo, useRef } from 'react'; import { useState, useEffect } from 'air-hook-inputs';");
    for (const path of ['../charts/useChartTokens', './playback', './products', './productWindows', './FactionColours', './countryLooks', './useCountriesAt', './useHoverLabels', './contentRevision.ts']) {
      code = code.replace(`from '${path}'`, "from 'air-hook-inputs'");
    }
    return code;
  },
  resolveId(id) { if (id === 'air-hook-inputs') return '\0air-hook-inputs'; },
  load(id) {
    if (id !== '\0air-hook-inputs') return;
    return `export const useState = () => globalThis.airUseState?.() ?? [globalThis.airHeld, () => {}];
      export const useEffect = (effect, deps) => globalThis.airUseEffect?.(effect, deps);
      export const useChartTokens = () => (${JSON.stringify(tokensFromValues(() => '#8899aa', 'sans-serif'))});
      export const useViewportHeight = () => 1000;
      export const usePlaybackHour = () => {};
      export const usePlaybackSpeed = clock => clock.getSpeed();
      export const useContentRevision = () => 0;
      export const useProduct = (_, request) => { if (request) globalThis.airRequests.push(request); return { data: request?.params.view === 'timeline' ? globalThis.airTimeline : request?.params.view === 'moment' ? globalThis.airDay : request?.params.windowsOnly ? globalThis.airWindows : request?.params.view === 'choices' ? globalThis.airChoices : undefined, error: request?.params.view === 'moment' ? globalThis.airDayError : null, retry() {} }; };
      export const useProductWindows = () => ({ data: new Map(globalThis.airDay ? [[0, globalThis.airDay]] : []), errors: new Map(), retry() {} });
      export const useFactionColours = () => ({ factions: [{ id: '7', name: 'Latest faction name' }], colours: new Map(globalThis.airFactionColours ?? []), displayNames: new Map(globalThis.airFactionName ? [['7', globalThis.airFactionName]] : []), warnings: [], setColour() {}, names: f => ({ name: globalThis.airFactionName ?? f.name, reason: null }) });
      export const STYLES = {};
      export const nameKey = (hour, tag) => JSON.stringify([hour, tag]);
      export const useCountryFactsRead = (_, track, wanted) => {
        globalThis.airStyleReads?.push(wanted);
        return new Map(wanted.map(({ tag, hour }) => [nameKey(hour, tag), { tag, name: tag, color: globalThis.airStyles?.find(read => read[0] === tag && read[1] === hour)?.[2] ?? [136,153,170] }]));
      };
      export const useCountryLooks = (_, track, tags, hour, active, segmentAt, hours, exact = false) => {
        globalThis.airLookPlans?.push({ hour, segmentAt, hours, exact });
        return { name: tag => tag, color: () => [136,153,170], flag: () => undefined }; };
      export const useCountriesAt = () => ({ country: tag => ({ name: tag }) });
      export const FOLLOW_MS = 400;
      export const useHoverLabels = (_, track, active, pending, missing, segmentAt) => { globalThis.airHoverSegment = segmentAt; return { ask() {}, labelAt: () => tag => ({ name: tag }), revision: 0 }; };`;
  },
}] });
const { useAir } = await vite.ssrLoadModule('/ui/src/dashboard/useAirSuperiority.ts').finally(() => vite.close());

function render(hour: number, mode = 'follow', options: Record<string, string | number> = { mode: 'sides', regionCount: 10 }, moment = false, selectedTag: string | null = null, firstHour = 0, playerTag: string | null = null) {
  requests.length = 0;
  let result: ReturnType<typeof import('./useAirSuperiority').useAir> | undefined;
  const track = { selectedTag, id: 'track', axisOffset: 0, branch: { id: '1', firstHour, lastHour: 100, gaps: [] }, source: { id: 'source', revision: 1, metadata: { campaignKey: 'campaign', countries: playerTag ? [{ id: 7, tag: playerTag }] : [], playerCountryId: 7 } } };
  function Probe() {
    result = useAir({ widget: { countries: { kind: 'selected' }, kind: 'air-superiority-timeline', trackIds: ['track'], time: { mode } }, active: true, mapOptions: {}, onDiagnostics() {},
      runtime: { client: shared.airClient ?? CLIENT, tracks: [track], clock: { getHour: () => hour, getSnapshot: () => ({ seekSequence: 0 }), getSpeed: () => 24 } } }, options, moment);
    return null;
  }
  renderToStaticMarkup(createElement(Probe));
  return { result: result!, requests: structuredClone(requests) as { params: Record<string, unknown> }[] };
}

test('follow asks once per day for the timeline and its current-day rows', () => {
  const first = render(25), next = render(26);
  // Bootstrap stays fixed while the first reply discovers cadence.
  assert.equal(first.requests[0].params.lastHour, 0);
  assert.deepEqual(first.requests[0], next.requests[0]);
  assert.deepEqual(first.requests[1], next.requests[1]);
  assert.equal(first.requests[1].params.firstHour, 24);
  assert.equal(first.requests[1].params.lastHour, 47);
  assert.equal(next.result.playhead, 26);
});

test('campaign shows the full extent with the clock cursor and missing viewpoint prompts', () => {
  const campaign = render(7, 'campaign');
  assert.equal(campaign.requests[0].params.lastHour, 100);
  assert.equal(campaign.result.playhead, 7);
  const empty = render(7, 'follow', { mode: 'viewpoint', regionCount: 10 });
  assert.equal(empty.result.needsViewpoint, true);
  assert.deepEqual(empty.requests, []);
});

test('moment window discovery asks only for the replay index', () => {
  const moment = render(7, 'follow', undefined, true);
  assert.equal(moment.requests[0].params.windowsOnly, true);
});

test('Follow supplies every campaign region label independently of ranked rows and playback', () => {
  const original = shared.airTimeline;
  const labels = ['Short', 'A much longer region reached later in the campaign'];
  shared.airChoices = { regions: labels.map((name, id) => ({ id, name })), warnings: [] };
  try {
    const before = render(7);
    assert.deepEqual(before.result.labelCatalogue, labels);
    const catalogue = before.requests.find(request => request.params.view === 'choices')!;
    assert.deepEqual(catalogue.params, { branchId: '1', view: 'choices' });
    shared.airTimeline = { ...(original as object), rows: [{ region: 1, name: labels[1], cells: [[2, null]] }] };
    const after = render(26);
    assert.deepEqual(after.result.labelCatalogue, labels);
    assert.deepEqual(after.requests.find(request => request.params.view === 'choices'), catalogue);
    assert.deepEqual(after.result.projectedTimeline?.rows.map(row => row.label), [labels[1]]);
    delete shared.airChoices;
    assert.equal(render(26).result.labelCatalogue, undefined, 'pending labels cannot fall back to ranked rows');
  } finally { shared.airTimeline = original; delete shared.airChoices; }
});

test('automatic viewpoint reads the dashboard header country', () => {
  const air = render(7, 'follow', { mode: 'viewpoint', regionCount: 10 }, false, 'opaque mod country');
  assert.equal(air.result.needsViewpoint, false);
  assert.equal(air.requests[0].params.viewpoint, 'opaque mod country');
  assert.equal(air.requests[0].params.viewpointCountry, true);
});

test('faction names use dated product text unless a campaign display name overrides it', () => {
  const original = shared.airTimeline;
  shared.airTimeline = { ...(original as object), sides: [{ key: 'f:7', faction: '7', colourFaction: '7', name: 'Earlier faction name', tag: null, leader: 'Country', coBelligerents: 0 }] };
  try {
    assert.equal(render(7).result.legend[0].label, 'Earlier faction name');
    shared.airFactionName = 'Campaign display name';
    assert.equal(render(7).result.legend[0].label, 'Campaign display name');
    shared.airFactionName = 'Latest faction name';
    assert.equal(render(7).result.legend[0].label, 'Latest faction name');
  } finally { shared.airTimeline = original; delete shared.airFactionName; }
});

test('air side colours use the dated majority faction independently of the anchor', () => {
  const original = shared.airTimeline;
  const side = { key: 'f:7', faction: '7', colourFaction: '9', name: 'Mixed alliance', tag: null, leader: 'Country', coBelligerents: 2,
    reads: [[0, 'Mixed alliance', 'Country', null, 2, [], '9'], [24, 'Mixed alliance', 'Country', null, 3, [], null]] };
  shared.airFactionColours = [['7', '#112233'], ['9', '#445566']];
  shared.airStyles = [['Country', 0, [16, 32, 48]]];
  shared.airStyleReads = [];
  shared.airTimeline = { ...(original as object), lastHour: 23, days: [23], sides: [side], segments: { Country: [{ firstHour: 0, lastHour: 24 }] }, anchorReads: { 'f:7': ['Country', 0] } };
  try {
    const before = render(23).result.legend[0] as { color: string; onColor?: unknown };
    assert.equal(before.color, '#445566');
    assert.equal(typeof before.onColor, 'function');
    shared.airTimeline = { ...(shared.airTimeline as object), lastHour: 24, days: [23, 24], sides: [side] };
    const after = render(24).result.legend[0] as { color: string; onColor?: unknown };
    assert.notEqual(after.color, '#112233');
    assert.notEqual(after.color, '#445566');
    assert.equal(after.color, '#102030');
    assert.equal(after.onColor, undefined);
    assert.ok(shared.airStyleReads.flat().some(read => read.tag === 'Country' && read.hour === 0));
  } finally { shared.airTimeline = original; delete shared.airFactionColours; delete shared.airStyles; delete shared.airStyleReads; }
});

test('follow surfaces a refused current-day reply instead of silently holding the timeline', () => {
  shared.airDayError = 'Exact air view exceeds MAX_PRODUCT_REPLY_BYTES';
  try { assert.equal(render(25).result.error, shared.airDayError); }
  finally { delete shared.airDayError; }
});

test('a branch starting at noon never asks for a reversed first partial day', () => {
  const air = render(12, 'follow', undefined, false, null, 12);
  for (const request of air.requests.filter(request => request.params.view !== 'choices')) {
    assert.equal(request.params.firstHour, 12);
    assert.ok(Number(request.params.lastHour) >= 12);
  }
});

test('automatic viewpoint falls back to the recorded player country', () => {
  const air = render(12, 'follow', { mode: 'viewpoint', regionCount: 10 }, false, null, 12, 'mod player');
  assert.equal(air.requests[0].params.viewpoint, 'mod player');
});

test('Follow displays its previous timeline while the next day is pending', () => {
  const reply = shared.airTimeline;
  shared.airHeld = { client: CLIENT, identity: JSON.stringify(['source', 1, '1', 0, 0, { mode: 'sides' }, 10, true]), timeline: reply };
  shared.airTimeline = undefined;
  try {
    const air = render(48);
    assert.equal(air.result.data, reply);
    assert.equal(air.requests[0].params.lastHour, 48);
    assert.equal(air.requests[1].params.firstHour, 48);
  } finally { shared.airTimeline = reply; delete shared.airHeld; }
});

test('follow day requests use the reply cadence and clamp a held noon start', () => {
  const original = shared.airTimeline;
  const reply = { ...(original as object), dayHours: 18 };
  shared.airTimeline = reply;
  shared.airHeld = { client: CLIENT, identity: JSON.stringify(['source', 1, '1', 0, 12, { mode: 'sides' }, 10, true]), timeline: reply };
  try {
    const first = render(12, 'follow', undefined, false, null, 12);
    assert.equal(first.requests[0].params.lastHour, 12);
    assert.equal(first.requests[1].params.firstHour, 12);
    assert.equal(first.requests[1].params.lastHour, 17);
    const next = render(19, 'follow', undefined, false, null, 12);
    assert.equal(next.requests[0].params.lastHour, 18);
    assert.equal(next.requests[1].params.firstHour, 18);
    assert.equal(next.requests[1].params.lastHour, 35);
  } finally { shared.airTimeline = original; delete shared.airHeld; }
});

test('a new day reply cannot relabel the held previous timeline day', () => {
  const original = shared.airTimeline;
  const reply = { ...(original as object), lastHour: 23, days: [23] };
  shared.airHeld = { client: CLIENT, identity: JSON.stringify(['source', 1, '1', 0, 0, { mode: 'sides' }, 10, true]), timeline: reply };
  shared.airTimeline = undefined;
  shared.airDay = { ...(original as object), view: 'moment', fallback: [], firstHour: 24, lastHour: 47, regions: [], recorded: [{ firstHour: 24, lastHour: 47 }], complete: [], wingsKnown: [] };
  try { assert.equal(render(25).result.projectedTimeline?.periods[0].readHour, 23); }
  finally { shared.airTimeline = original; delete shared.airHeld; delete shared.airDay; }
});

test('Follow uses current-day native identity bounds while coalition colours retain their dated anchor', () => {
  const original = shared.airTimeline;
  shared.airTimeline = { ...(original as object), segments: { opaque: [{ firstHour: 0, lastHour: 24 }] } };
  shared.airDay = { ...(original as object), view: 'moment', fallback: [], firstHour: 24, lastHour: 47, regions: [],
    recorded: [{ firstHour: 24, lastHour: 47 }], complete: [], wingsKnown: [],
    segments: { opaque: [{ firstHour: 0, lastHour: 25 }, { firstHour: 26, lastHour: 47 }],
      anchor: [{ firstHour: 0, lastHour: 5 }] }, sides: [{ key: 'w:1:101', faction: null, colourFaction: null, tag: 'opaque', leader: null, name: 'opaque', coBelligerents: 1 }], anchorReads: { 'w:1:101': ['anchor', 2] } };
  shared.airLookPlans = [];
  shared.airStyleReads = [];
  try {
    render(26);
    const [current] = shared.airLookPlans;
    assert.equal(current.hour, 26);
    assert.equal(current.segmentAt('opaque', 26), 26);
    assert.equal(current.segmentAt('opaque', 25), 0);
    assert.ok(shared.airStyleReads.flat().some(read => read.tag === 'anchor' && read.hour === 2));
  } finally { shared.airTimeline = original; delete shared.airDay; delete shared.airLookPlans; delete shared.airStyleReads; }
});

test('Follow dates an unchanged daily score by its latest reached recorded hour', () => {
  const original = shared.airTimeline;
  shared.airDay = { ...(original as object), view: 'moment', fallback: [], firstHour: 24, lastHour: 47, regions: [],
    recorded: [{ firstHour: 24, lastHour: 47 }], complete: [], wingsKnown: [], excluded: [], theatreReads: [] };
  try { assert.equal(render(26).result.projectedTimeline?.periods.at(-1)?.readHour, 26); }
  finally { delete shared.airDay; }
});

test('the region view reads country colours at the displayed hour when scores are unchanged', () => {
  const original = shared.airTimeline;
  shared.airWindows = { windows: [{ firstHour: 0, lastHour: 47 }] };
  shared.airDay = { ...(original as object), view: 'moment', firstHour: 0, lastHour: 47,
    sides: [{ key: 'c:Country', tag: 'Country', faction: null, colourFaction: null, leader: 'Country', name: 'Country', coBelligerents: 0 }],
    segments: { Country: [{ firstHour: 0, lastHour: 47 }] }, regions: [], fallback: [],
    recorded: [{ firstHour: 0, lastHour: 47 }], complete: [], wingsKnown: [] };
  shared.airStyleReads = [];
  try {
    render(26, 'follow', undefined, true);
    assert.deepEqual(shared.airStyleReads.flat(), [{ tag: 'Country', hour: 26 }]);
  } finally { delete shared.airWindows; delete shared.airDay; delete shared.airStyleReads; }
});

test('Follow keeps the default state of selected regions absent from current-day rows', () => {
  const original = shared.airTimeline;
  shared.airTimeline = { ...(original as object), lastHour: 24, days: [23, 24],
    rows: [{ region: 7000, name: 'Mod region', cells: [[2, ['1', '1']]] }] };
  shared.airDay = { ...(original as object), view: 'moment', firstHour: 24, lastHour: 47, regions: [],
    fallback: [[24, 47, 'no-air-object']], recorded: [{ firstHour: 24, lastHour: 47 }], complete: [], wingsKnown: [], excluded: [], theatreReads: [] };
  try {
    const current = render(26).result.projectedTimeline!;
    assert.equal(current.viewAt(7000, current.periods.length - 1)?.state, 'no-air-object');
    shared.airDay = { ...(shared.airDay as object), fallback: [[24, 47, 'outside-theatres']] };
    const outside = render(26).result.projectedTimeline!;
    assert.equal(outside.viewAt(7000, outside.periods.length - 1)?.reason, 'outside-theatres');
    assert.equal(outside.rows[0].cells.at(-1)?.filtered, true);
  } finally { shared.airTimeline = original; delete shared.airDay; }
});

test('Name choices omit the viewpoint comparison and side filter', () => {
  shared.airWindows = { windows: [{ firstHour: 0, lastHour: 100 }] };
  try {
    const air = render(7, 'follow', { mode: 'viewpoint', regionCount: 10, order: 'name' }, true, 'mod player');
    const choices = air.requests.find(request => request.params.view === 'choices' && !request.params.windowsOnly)!;
    for (const field of ['mode', 'viewpoint', 'viewpointCountry', 'sides']) assert.ok(!(field in choices.params));
  } finally { delete shared.airWindows; }
});


test('cold Follow keeps its bootstrap request stable across played hours', () => {
  const original = shared.airTimeline;
  shared.airTimeline = undefined;
  try {
    const first = render(25), next = render(26);
    assert.deepEqual(first.requests, next.requests);
    assert.equal(first.requests[0].params.lastHour, 0);
    assert.equal(first.result.data, undefined);
    assert.equal(first.requests.length, 2);
  } finally { shared.airTimeline = original; }
});

test('Follow effects retain paired reads across days and clear them across client visits', () => {
  const original = shared.airTimeline;
  const timeline = { ...(original as object), lastHour: 24, days: [23, 24], rows: [{ region: 7000, name: 'Mod region', cells: [[2, ['1', '1']]] }] };
  const region = { id: 7000, name: 'Mod region', active: [], wings: [], runs: [[24, 47, ['2', '3']]] };
  const day = { ...(original as object), view: 'moment', fallback: [], firstHour: 24, lastHour: 47, regions: [region], recorded: [{ firstHour: 24, lastHour: 47 }], complete: [], wingsKnown: [] };
  let held: { timeline?: unknown; day?: unknown } | undefined;
  let effectIndex = 0;
  const dependencies: (readonly unknown[])[] = [];
  const pending: (() => unknown)[] = [];
  shared.airUseState = () => [held, (update: (previous: unknown) => typeof held) => { held = update(held); }];
  shared.airUseEffect = (effect, deps) => {
    const index = effectIndex++, previous = dependencies[index];
    dependencies[index] = deps;
    if (!previous || deps.some((value, index) => !Object.is(value, previous[index]))) pending.push(effect);
  };
  const draw = (hour: number) => {
    effectIndex = 0;
    const air = render(hour);
    for (const effect of pending.splice(0)) effect();
    return air;
  };
  shared.airTimeline = timeline; shared.airDay = day;
  try {
    draw(25);
    assert.equal(held?.timeline, timeline); assert.equal(held?.day, day);
    shared.airTimeline = undefined; shared.airDay = undefined;
    const waiting = draw(48);
    assert.equal(waiting.result.data, timeline);
    assert.equal(waiting.result.projectedTimeline?.periods.at(-1)?.readHour, 47);
    assert.equal(waiting.result.projectedTimeline?.rows[0].cells.at(-1)?.parts[0].share, 2 / 5);
    assert.equal(held?.day, day);
    const nextTimeline = { ...timeline, lastHour: 48, days: [23, 47, 48], rows: [{ region: 7000, name: 'Mod region', cells: [[3, ['1', '1']]] }] };
    const nextDay = { ...day, firstHour: 48, lastHour: 71, regions: [{ ...region, runs: [[48, 71, ['3', '1']]] }], recorded: [{ firstHour: 48, lastHour: 71 }] };
    shared.airTimeline = nextTimeline; shared.airDay = nextDay;
    const arrived = draw(49);
    assert.equal(arrived.result.data, nextTimeline);
    assert.equal(arrived.result.projectedTimeline?.periods.at(-1)?.readHour, 49);
    assert.equal(arrived.result.projectedTimeline?.rows[0].cells.at(-1)?.parts[0].share, 3 / 4);
    assert.equal(held?.timeline, nextTimeline); assert.equal(held?.day, nextDay);
    shared.airTimeline = undefined; shared.airDay = undefined;
    shared.airClient = {};
    const switched = draw(50);
    assert.equal(switched.result.data, undefined);
    assert.equal(switched.result.projectedTimeline, null);
    assert.equal(switched.requests.length, 2);
    assert.equal(switched.requests[0].params.lastHour, 0);
    assert.equal(held?.timeline, undefined); assert.equal(held?.day, undefined);
    delete shared.airClient;
    const returned = draw(51);
    assert.equal(returned.result.data, undefined);
    assert.equal(returned.result.projectedTimeline, null);
    assert.equal(returned.requests.length, 2);
    assert.equal(returned.requests[0].params.lastHour, 0);
  } finally {
    shared.airTimeline = original; delete shared.airDay;
    delete shared.airClient;
    delete shared.airUseState; delete shared.airUseEffect;
  }
});


test('timeline cells resolve country colours at their supplying hours through tag reuse', () => {
  const original = shared.airTimeline;
  shared.airStyles = [['X', 23, [16, 32, 48]], ['X', 47, [112, 128, 144]]];
  shared.airStyleReads = [];
  shared.airTimeline = { ...(original as object), lastHour: 47, days: [23, 47],
    sides: [{ key: 'c:X', faction: null, colourFaction: null, tag: 'X', leader: 'X', name: 'X', coBelligerents: 0 }],
    segments: { X: [{ firstHour: 0, lastHour: 23 }, { firstHour: 24, lastHour: 47 }] },
    rows: [{ region: 7000, name: 'Synthetic region', cells: [[2, { scores: [[0, '100']] }]] }] };
  try {
    const cells = render(47, 'campaign').result.projectedTimeline!.rows[0].cells;
    assert.equal(cells[0].parts[0].color, '#102030');
    assert.equal(cells[1].parts[0].color, '#708090');
    assert.deepEqual(shared.airStyleReads.flat(), [{ tag: 'X', hour: 23 }, { tag: 'X', hour: 47 }]);
  } finally { shared.airTimeline = original; delete shared.airStyles; delete shared.airStyleReads; }
});


test('air hover labels receive the recorded country identity bounds, including reuse and missing evidence', () => {
  const original = shared.airTimeline;
  shared.airTimeline = { ...(original as object), segments: { A: [{ firstHour: 0, lastHour: 10 }, { firstHour: 12, lastHour: 24 }] } };
  try {
    render(7, 'campaign');
    assert.equal(typeof shared.airHoverSegment, 'function');
    assert.equal(shared.airHoverSegment!('A', 7), 0);
    assert.equal(shared.airHoverSegment!('A', 11), null);
    assert.equal(shared.airHoverSegment!('A', 20), 12);
    assert.equal(shared.airHoverSegment!('missing', 7), null);
  } finally { shared.airTimeline = original; delete shared.airHoverSegment; }
});
