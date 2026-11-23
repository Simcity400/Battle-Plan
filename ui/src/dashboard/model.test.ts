import test from 'node:test';
import assert from 'node:assert/strict';
import type { CountryMetadata } from '../shell/api';
import type { DashboardSource } from './client';
import { bindTracks, clockBounds, createWidget, defaultTemplate, rebaseWidget, recordingEndTime, resolveWindow, widgetAtDefault } from './model.ts';
import { setOption } from './options.ts';
import type { OptionField } from './options';
import { parseTemplate } from './storage.ts';
import { registerWidgets, widgetUnavailable } from './definitions.ts';
import '../widgets/definitions.ts';

registerWidgets([
  { kind: 'current', version: 1, title: 'Test moment data', icon: 'grid', timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: false, minSize: { w: 2, h: 3 }, defaultSize: { w: 4, h: 6 } },
  { kind: 'history', version: 1, title: 'Test history data', icon: 'grid', timeModes: ['follow', 'pin', 'range', 'campaign'], defaultTime: 'campaign', scope: 'history', data: true, history: true, minSize: { w: 2, h: 3 }, defaultSize: { w: 4, h: 6 } },
  { kind: 'world', version: 1, title: 'Test whole-world data', icon: 'grid', timeModes: ['follow', 'pin'], defaultTime: 'follow', scope: 'moment', data: true, history: false, countries: { max: 8, all: true }, minSize: { w: 2, h: 3 }, defaultSize: { w: 4, h: 6 } },
]);

test('two settings changed from the same drawn widget both stay', () => {
  const fields: OptionField[] = [
    { key: 'layers', section: 'options', label: 'Layers', kind: 'choice', default: '5', choices: [{ value: '5', label: '5' }, { value: '8', label: '8' }] },
    { key: 'markers', section: 'options', label: 'Markers', kind: 'toggle', default: true },
  ];
  const drawn = { ...createWidget('history', 'main', 'w'), options: { markers: false } };
  const first = { ...drawn, options: setOption(fields, drawn.options, 'layers', '8')! };
  const second = { ...drawn, options: setOption(fields, drawn.options, 'markers', true)! };
  let current = rebaseWidget(drawn, drawn, first);
  current = rebaseWidget(current, drawn, second);
  assert.deepEqual(current.options, { layers: '8' });
  assert.deepEqual(second.options, {});
  const renamed = rebaseWidget({ ...current, contentScale: 150 }, drawn, { ...drawn, title: 'Renamed' });
  assert.equal(renamed.title, 'Renamed');
  assert.equal(renamed.contentScale, 150);
  assert.deepEqual(renamed.options, { layers: '8' });
  assert.equal(rebaseWidget(current, current, createWidget('history', 'main', 'w')).options, undefined);
});

function country(id = '1', tag = 'GER'): CountryMetadata {
  return { id, tag, name: tag, observedBranchId: '7', observedHour: 1000 };
}
function source(id = 'lease-a', firstHour = 1000, countries = [country()]): DashboardSource {
  return { id, path: `C:/recordings/${id}.battleplan`, revision: `revision-${id}`, metadata: {
    path: `C:/recordings/${id}.battleplan`, revision: `revision-${id}`, name: id, format: 1, captureMode: 'full', campaignKey: 'c0ffee',
    branches: [{ id: '7', sessionId: '9', firstHour, lastHour: firstHour + 1000,
      main: true, fork: null, joins: [], sampleBoundary: 'Sample origin unknown', gaps: [] }],
    countries, playerCountryId: countries[0]?.id ?? null, contentVerified: false, partial: null, warnings: [], buildWarnings: [],
  } };
}

test('branch selection on a non-first track leaves sibling branches and saved pins intact', () => {
  const template = defaultTemplate();
  template.tracks.push({ ...template.tracks[0], id: 'comparison', branchIndex: 1 });
  const replay = source();
  replay.metadata.branches.push({ ...replay.metadata.branches[0], id: '8' });
  const pinned = { ...template.widgets[0], trackIds: ['comparison'], time: { mode: 'pin' as const, windows: { comparison: { firstOffset: 20, lastOffset: 40 } } } };
  const tracks = bindTracks(template, new Map([['primary', replay]]), { comparison: '7' });
  assert.deepEqual(tracks.map(track => [track.id, track.branch.id]), [['main', '7'], ['comparison', '7']]);
  assert.equal(template.tracks[1].branchIndex, 1);
  assert.deepEqual(resolveWindow(pinned, tracks[1], tracks, 1700), { firstHour: 1020, lastHour: 1040 });
  assert.deepEqual(bindTracks(template, new Map([['primary', replay]]), { main: '8' }).map(track => track.branch.id), ['8', '8']);
  replay.metadata.branches[1] = { ...replay.metadata.branches[1], joins: [{ hour: 1, segmentId: '9', parentId: '8', evidence: 'chosen', choices: [] }] };
  assert.deepEqual(bindTracks(template, new Map([['primary', replay]]), { main: '9', comparison: '10' }).map(track => track.branch.id), ['8', '8']);
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

test('pinning freezes the selected window and range alignment follows the first track without moving playback', () => {
  const template = defaultTemplate();
  const tracks = bindTracks(template, new Map([['primary', source()]]));
  const widget = { ...createWidget('history', 'main', 'pinned'), time: { mode: 'range' as const, firstOffset: 20, lastOffset: 50 } };
  const pinned = { ...widget, time: { mode: 'pin' as const, windows: { [tracks[0].id]: { firstOffset: 20, lastOffset: 50 } } } };
  assert.deepEqual(resolveWindow(pinned, tracks[0], tracks, 1100), { firstHour: 1020, lastHour: 1050 });
  assert.deepEqual(resolveWindow(pinned, tracks[0], tracks, 1990), { firstHour: 1020, lastHour: 1050 });
  const missing = { ...pinned, time: { mode: 'pin' as const, windows: {} } };
  assert.equal(resolveWindow(missing, tracks[0], tracks, 1400), null);
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
});

test('unlinked maps hold the branch end and linked maps follow the shared axis', () => {
  const template = defaultTemplate();
  const tracks = bindTracks(template, new Map([['primary', source()]]));
  const track = { ...tracks[0], axisOffset: 1000 };
  const linked = createWidget('map', 'main');
  assert.equal(linked.time.mode, 'follow');
  assert.equal(createWidget('current', 'main').time.mode, 'follow');
  assert.deepEqual(createWidget('world', 'main').countries, { kind: 'all' });
  assert.deepEqual(createWidget('current', 'main').countries, { kind: 'selected' });
  const map = { ...linked, time: { mode: 'campaign' as const } };
  assert.equal(widgetUnavailable(map), null);
  assert.deepEqual(resolveWindow(map, track, [track], 0), { firstHour: 2000, lastHour: 2000 });
  assert.deepEqual(resolveWindow(map, track, [track], 500), { firstHour: 2000, lastHour: 2000 });
  assert.deepEqual(resolveWindow(linked, track, [track], 120), { firstHour: 1120, lastHour: 1120 });
  assert.deepEqual(resolveWindow(linked, track, [track], 121), { firstHour: 1121, lastHour: 1121 });
  const history = createWidget('history', 'main');
  assert.deepEqual(resolveWindow(history, track, [track], 120), { firstHour: 1000, lastHour: 2000 });
  template.widgets[0].time = { mode: 'campaign' };
  assert.deepEqual(parseTemplate(JSON.parse(JSON.stringify(template)))?.widgets.map((widget) => widget.time), template.widgets.map((widget) => widget.time));
});

test('previews show each campaign at its end, where widgets have data', () => {
  const tracks = [{ id: 'a', branch: { firstHour: 100, lastHour: 9_000 } }, { id: 'b', branch: { firstHour: 40, lastHour: 50 } }];
  assert.deepEqual(recordingEndTime({ timeModes: ['campaign', 'follow', 'pin'], scope: 'moment' }, tracks), { mode: 'campaign' });
  assert.deepEqual(recordingEndTime({ timeModes: ['follow', 'pin'], scope: 'moment' }, tracks),
    { mode: 'pin', windows: { a: { firstOffset: 8_900, lastOffset: 8_900 }, b: { firstOffset: 10, lastOffset: 10 } } }, 'a follow-only moment pins the last hour, not the first');
  assert.deepEqual(recordingEndTime({ timeModes: ['follow', 'pin'], scope: 'history' }, tracks),
    { mode: 'pin', windows: { a: { firstOffset: 0, lastOffset: 8_900 }, b: { firstOffset: 0, lastOffset: 10 } } });
  assert.equal(recordingEndTime({ timeModes: ['follow'], scope: 'moment' }, tracks), null);
});

test('a widget is at its default only when a reset would change nothing visible', () => {
  const widget = createWidget('army-size', 'main', 'army');
  assert.equal(widgetAtDefault(widget, 'main'), true);
  assert.equal(widgetAtDefault({ ...widget, options: { view: 'race' } }, 'main'), true, 'a stored default value changes nothing');
  assert.equal(widgetAtDefault({ ...widget, trackIds: ['other'] }, 'main'), true, 'a widget following the sidebar ignores stale tracks');
  for (const changed of [{ ...widget, title: 'My army' }, { ...widget, contentScale: 90 }, { ...widget, options: { view: 'lines' } },
    { ...widget, time: { mode: 'campaign' } } as typeof widget, { ...widget, campaign: 'tracks', trackIds: ['other'] } as typeof widget,
    { ...widget, configVersion: widget.configVersion + 1 }])
    assert.equal(widgetAtDefault(changed, 'main'), false, JSON.stringify(changed));
});
