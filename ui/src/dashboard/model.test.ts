import test from 'node:test';
import assert from 'node:assert/strict';
import type { CountryMetadata, CountrySeries, CountrySeriesPoint } from '../api';
import type { DashboardSource } from './client';
import { bindTracks, branchSelections, clockBounds, createWidget, defaultTemplate, exactPoint, exactWindow, makeQueries,
  mergeSeriesChunks, pinTime, resolveWindow, selectCountries } from './model.ts';
import { parseTemplate } from './storage.ts';
import { registerWidgetDefinition, widgetUnavailable } from './definitions.ts';

// Synthetic code-defined products exercise the query framework independently of UI widgets.
registerWidgetDefinition({ kind: 'current', version: 1, title: 'Test exact data', timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: 'countries', minSize: { w: 2, h: 3 }, defaultSize: { w: 4, h: 6 } });
registerWidgetDefinition({ kind: 'history', version: 1, title: 'Test history data', timeModes: ['follow', 'pin', 'range', 'campaign'], defaultTime: 'campaign', scope: 'history', data: 'countries', minSize: { w: 2, h: 3 }, defaultSize: { w: 4, h: 6 } });

function country(id = '1', tag = 'GER'): CountryMetadata {
  return { id, tag, name: tag, observedBranchId: '7', observedHour: 1000 };
}
function source(id = 'lease-a', firstHour = 1000, countries = [country()]): DashboardSource {
  return { id, path: `C:/recordings/${id}.battleplan`, revision: `revision-${id}`, metadata: {
    path: `C:/recordings/${id}.battleplan`, revision: `revision-${id}`, name: id, format: 1,
    branches: [{ id: '7', sessionId: '9', firstHour, lastHour: firstHour + 1000,
      parentId: null, parentHour: null, sampleBoundary: 'Sample origin unknown', gaps: [] }],
    countries, playerCountryId: countries[0]?.id ?? null, contentVerified: false, warnings: [],
  } };
}
function point(hour: number, raw: string | null = '9007199254740993001'): CountrySeriesPoint {
  const values = { politicalPower: raw, commandPower: raw, stability: raw, warSupport: raw, convoys: raw };
  const sample = raw == null ? null : { hour, values };
  return { firstHour: hour, lastHour: hour, status: raw == null ? 'gap' : 'recorded', recordedHours: raw == null ? 0 : 1,
    gapHours: raw == null ? 1 : 0, unavailableHours: 0, absentHours: 0, first: sample, last: sample, min: values, max: values,
    missingMetricHours: { politicalPower: 0, commandPower: 0, stability: 0, warSupport: 0, convoys: 0 }, reason: null };
}
function series(points: CountrySeriesPoint[]): CountrySeries {
  return { branchId: '7', sessionId: '9', countryId: '1', revision: 'revision-a', firstHour: points[0].firstHour,
    lastHour: points[points.length - 1].lastHour, country: country(), countries: [country()], points,
    coverage: { recordedHours: points.filter((item) => item.status === 'recorded').length, gapHours: points.filter((item) => item.status === 'gap').length,
      unavailableHours: 0, absentHours: 0 }, downsampled: false, sampleBoundary: 'Sample origin unknown', valueScale: '1000', warnings: [] };
}

test('exact values preserve zero and large integers; gaps and intervals have no nearest-sample fallback', () => {
  const input = series([point(1000), point(1001, null), point(1002, '0')]);
  assert.equal(exactPoint(input, 1000)?.first?.values.politicalPower, '9007199254740993001');
  assert.equal(exactPoint(input, 1001)?.status, 'gap');
  assert.equal(exactPoint(input, 1001)?.first, null);
  assert.equal(exactPoint(input, 1002)?.first?.values.politicalPower, '0');
  assert.equal(exactPoint(input, 1003), null);
  const aggregated = series([{ ...point(1000), lastHour: 1010, last: point(1010).last }]);
  assert.equal(exactPoint(aggregated, 1005), null);
  assert.equal(exactPoint(aggregated, 1000), null);
});

test('branch selection on a non-first track leaves sibling branches and saved pins intact', () => {
  const template = defaultTemplate();
  template.tracks.push({ ...template.tracks[0], id: 'comparison', branchIndex: 1 });
  const replay = source();
  replay.metadata.branches.push({ ...replay.metadata.branches[0], id: '8' });
  const pinned = { ...template.widgets[0], trackIds: ['comparison'], time: { mode: 'pin' as const, windows: { comparison: { firstOffset: 20, lastOffset: 40 } } } };
  const changed = { ...template, tracks: branchSelections(template.tracks, { comparison: 0 }) };
  const tracks = bindTracks(changed, new Map([['primary', replay]]));
  assert.deepEqual(tracks.map(track => [track.id, track.branch.id]), [['main', '7'], ['comparison', '7']]);
  assert.equal(template.tracks[1].branchIndex, 1);
  assert.deepEqual(resolveWindow(pinned, tracks[1], tracks, 1700), { firstHour: 1020, lastHour: 1040 });
  assert.deepEqual(branchSelections(template.tracks, { main: 1 }).map(track => track.branchIndex), [1, 1]);
});

test('explicit anchors align source hours and missing anchors or branches remain unbound', () => {
  const template = defaultTemplate();
  template.roles.push({ id: 'comparison', label: 'Comparison', selectedTag: null });
  template.tracks.push({ id: 'other', roleId: 'comparison', label: 'Other', branchIndex: 0, anchorOffset: null });
  const sources = new Map([['primary', source()], ['comparison', source('lease-b', 2000)]]);
  let tracks = bindTracks(template, sources);
  assert.deepEqual(clockBounds(tracks), { firstHour: 1000, lastHour: 3000 });
  template.alignment = 'anchors';
  assert.equal(bindTracks(template, sources).length, 0);
  template.tracks[0].anchorOffset = 20;
  template.tracks[1].anchorOffset = 70;
  tracks = bindTracks(template, sources);
  assert.deepEqual(clockBounds(tracks), { firstHour: -70, lastHour: 980 });
  const current = createWidget('current', 'main', 'value');
  assert.equal(resolveWindow(current, tracks[0], tracks, 10)?.lastHour, 1030);
  assert.equal(resolveWindow(current, tracks[1], tracks, 10)?.lastHour, 2080);
  template.tracks[1].branchIndex = 3;
  assert.equal(bindTracks(template, sources).length, 1);
});

test('Follow requests stay chunked, campaign requests stay stable, and no exact request clamps an unavailable target', () => {
  const template = defaultTemplate();
  const tracks = bindTracks(template, new Map([['primary', source()]]));
  const track = tracks[0];
  const current = createWidget('current', 'main', 'current');
  const follow = { ...createWidget('history', 'main', 'history'), time: { mode: 'follow' as const } };
  const campaign = createWidget('history', 'main', 'campaign');
  assert.deepEqual(makeQueries(current, track, tracks, 1250), makeQueries(current, track, tracks, 1270));
  assert.deepEqual(makeQueries(current, track, tracks, 1250), makeQueries(follow, track, tracks, 1250));
  assert.deepEqual(makeQueries(campaign, track, tracks, 1010), makeQueries(campaign, track, tracks, 1900));
  const demand = makeQueries(follow, track, tracks, 1250)[0];
  const view = resolveWindow(follow, track, tracks, 1250)!;
  assert.ok(demand.firstHour <= view.firstHour && demand.lastHour >= view.lastHour);
  assert.ok(demand.lastHour - demand.firstHour + 1 <= 359);
  assert.equal(demand.maxPoints, demand.lastHour - demand.firstHour + 1);
  assert.equal(exactWindow(track, 999), null);
  assert.deepEqual(makeQueries(current, track, tracks, 2001), []);
  assert.equal(resolveWindow(current, track, tracks, 2001)?.lastHour, 2001);
});

test('pinning freezes the selected window and range alignment follows the first track without moving playback', () => {
  const template = defaultTemplate();
  const tracks = bindTracks(template, new Map([['primary', source()]]));
  const widget = { ...createWidget('history', 'main', 'pinned'), time: { mode: 'range' as const, firstOffset: 20, lastOffset: 50 } };
  const pinned = { ...widget, time: pinTime(widget, tracks, 1400) };
  assert.deepEqual(resolveWindow(pinned, tracks[0], tracks, 1100), { firstHour: 1020, lastHour: 1050 });
  assert.deepEqual(resolveWindow(pinned, tracks[0], tracks, 1990), { firstHour: 1020, lastHour: 1050 });
  const missing = { ...pinned, time: { mode: 'pin' as const, windows: {} } };
  assert.equal(resolveWindow(missing, tracks[0], tracks, 1400), null);
});

test('country selectors rebind by role and tag while reporting missing identities; all countries are batched', () => {
  const template = defaultTemplate();
  template.roles[0].selectedTag = 'GER';
  const first = bindTracks(template, new Map([['primary', source()]]))[0];
  assert.equal(selectCountries(first, { kind: 'selected' }).countries[0].id, '1');
  const replacement = source('lease-b', 5000, [country('88', 'GER'), country('99', 'ENG')]);
  const second = bindTracks(template, new Map([['primary', replacement]]))[0];
  assert.equal(selectCountries(second, { kind: 'selected' }).countries[0].id, '88');
  assert.deepEqual(selectCountries(second, { kind: 'tags', tags: ['SOV'] }).countries, []);
  assert.match(selectCountries(second, { kind: 'tags', tags: ['SOV'] }).missing[0], /SOV/);
  const countries = Array.from({ length: 60 }, (_, index) => country(String(index + 1), `C${index}`));
  const allTrack = bindTracks(template, new Map([['primary', source('many', 1000, countries)]]))[0];
  const widget = { ...createWidget('history', 'main', 'all'), countries: { kind: 'all' as const } };
  const demands = makeQueries(widget, allTrack, [allTrack], 1000);
  assert.equal(demands.length, 3);
  assert.equal(new Set(demands.flatMap((demand) => demand.countryIds)).size, 60);
  assert.ok(demands.every((demand) => demand.maxPoints * demand.countryIds.length <= 1200));
});

test('transport chunks merge by country with exact deduplication and explicit discontinuities', () => {
  const first = series([point(1000), point(1001)]);
  const later = series([point(1001), point(1004, null), point(1005, '0')]);
  const merged = mergeSeriesChunks([later, first]);
  assert.equal(merged.length, 1);
  assert.deepEqual(merged[0].points.map((item) => item.firstHour), [1000, 1001, 1004, 1005]);
  assert.equal(merged[0].points[0].first?.values.politicalPower, '9007199254740993001');
  assert.equal(merged[0].coverage.recordedHours, 3);
  assert.equal(merged[0].coverage.gapHours, 1);
  assert.equal(exactPoint(merged[0], 1002), null);
  assert.throws(() => mergeSeriesChunks([first, series([point(1001, '42')])]), /conflicting/);
  assert.throws(() => mergeSeriesChunks([first, { ...later, branchId: '8' }]), /identities/);
});

test('saved templates strip runtime fields and reject corruption without binding stale file handles', () => {
  const input = { ...defaultTemplate(), path: 'C:/private/replay.battleplan', sourceId: 'stale-lease', data: [42] };
  const parsed = parseTemplate(input);
  assert.ok(parsed);
  assert.equal('path' in parsed, false);
  assert.equal('sourceId' in parsed, false);
  assert.equal('data' in parsed, false);
  assert.equal(parseTemplate({ ...input, version: 99 }), null);
  assert.equal(parseTemplate({ ...input, roles: [{ ...input.roles[0], id: '__proto__' }] }), null);
  const malformed = defaultTemplate();
  malformed.layouts.lg![0].x = 50;
  assert.equal(parseTemplate(malformed), null);
  const restored = bindTracks(parsed, new Map([['primary', source('replacement', 8000)]]));
  assert.equal(restored[0].source.id, 'replacement');
  assert.equal(restored[0].branch.firstHour, 8000);
  assert.equal(bindTracks(parsed, new Map()).length, 0);
});

test('widget versions migrate independently and unavailable widgets survive template round trips', () => {
  const legacy = defaultTemplate();
  const input = { ...legacy, widgets: legacy.widgets.map(({ configVersion: _version, ...widget }) => widget) };
  assert.equal(parseTemplate(input)?.widgets[0].configVersion, 1);
  const template = defaultTemplate();
  template.widgets[0].kind = 'future-widget';
  template.widgets[1].configVersion = 12;
  template.widgets[2] = createWidget('history', 'main', template.widgets[2].id);
  const restored = parseTemplate(JSON.parse(JSON.stringify(template)));
  assert.ok(restored);
  assert.deepEqual(restored.widgets, template.widgets);
  assert.match(widgetUnavailable(restored.widgets[0])!, /not installed/);
  assert.match(widgetUnavailable(restored.widgets[1])!, /version 12/);
  assert.equal(widgetUnavailable(restored.widgets[2]), null);
  const tracks = bindTracks(restored, new Map([['primary', source()]]));
  assert.deepEqual(makeQueries(restored.widgets[0], tracks[0], tracks, 1000), []);
  assert.deepEqual(makeQueries(restored.widgets[1], tracks[0], tracks, 1000), []);
  assert.equal(makeQueries(restored.widgets[2], tracks[0], tracks, 1000).length, 1);
});

test('only campaign maps start live; unlinked maps hold the branch end and linked maps follow the shared axis', () => {
  const template = defaultTemplate();
  assert.ok(template.widgets.every((widget) => widget.time.mode === (widget.kind === 'map' ? 'follow' : 'campaign')));
  const tracks = bindTracks(template, new Map([['primary', source()]]));
  const track = { ...tracks[0], axisOffset: 1000 };
  const linked = createWidget('map', 'main');
  assert.equal(linked.time.mode, 'follow');
  assert.equal(createWidget('placeholder', 'main').time.mode, 'campaign');
  const map = { ...linked, time: { mode: 'campaign' as const } };
  assert.equal(widgetUnavailable(map), null);
  assert.deepEqual(resolveWindow(map, track, [track], 0), { firstHour: 2000, lastHour: 2000 });
  assert.deepEqual(resolveWindow(map, track, [track], 500), { firstHour: 2000, lastHour: 2000 });
  assert.deepEqual(resolveWindow(linked, track, [track], 120), { firstHour: 1120, lastHour: 1120 });
  assert.deepEqual(resolveWindow(linked, track, [track], 121), { firstHour: 1121, lastHour: 1121 });
  const history = createWidget('history', 'main');
  assert.deepEqual(resolveWindow(history, track, [track], 120), { firstHour: 1000, lastHour: 2000 });
  template.widgets[0].time = { mode: 'follow' };
  assert.deepEqual(parseTemplate(JSON.parse(JSON.stringify(template)))?.widgets.map((widget) => widget.time), template.widgets.map((widget) => widget.time));
});
