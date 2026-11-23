import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { datedPlaybackVerdict } from './datedPlayback.mjs';
import { inputPictures } from './datedPlayback.mjs';

test('sweep input coverage marks gestures at the boundary and excludes hidden ink', () => {
  const source = readFileSync(new URL('./sweep.mjs', import.meta.url), 'utf8');
  assert.ok(source.indexOf("await mark('scrub');") > source.indexOf("await app.mouse('mousePressed'"),
    'scrub is marked after the drag starts');
  assert.ok(source.indexOf("await mark('scrub');") < source.indexOf("for (const fraction"),
    'scrub starts when the drag starts');
  assert.ok(source.indexOf("await app.evaluate(`__e2e.setPinWidget") > source.indexOf("data-fixed-hour')`); await s.frames(4);"),
    'pin widget is assigned after the pinned widget appears');
  assert.ok(source.indexOf("await mark('pin');") < source.indexOf("await s.click(time, { text: 'Pinned' })"),
    'pin is marked at the click');
  assert.match(source, /pinWidget = owners\[index\];\s*await app\.evaluate\(`__e2e\.setPinWidget\(\$\{JSON\.stringify\(pinWidget\)\}\)`\)/,
    'the pin action carries the widget whose settings were clicked');
  assert.match(source, /if \(c\.hidden \|\| !c\.checkVisibility\(/, 'hidden worker canvases are not input pictures');
  assert.match(source, /querySelectorAll\('\[data-surface\]'\)/, 'mounted hidden surfaces, drawn or installed into the page, stay declared');
  assert.doesNotMatch(source, /canvas\[data-surface\]:not\(\[hidden\]\)/, 'surface inventory does not discard hidden declarations');
  const rewindMoved = source.indexOf('Playback rewind did not move back');
  const rewindSettled = source.indexOf('const rewind = await settleInputs()');
  const scrubStarts = source.indexOf("await app.mouse('mousePressed'");
  assert.ok(rewindSettled > rewindMoved && rewindSettled < scrubStarts,
    'rewind waits for visible dated widgets before the next input');
  assert.match(source, /const settleInputs = async \(\) =>[\s\S]*s\.until\(`\$\{unsettled\}\.length === 0`, 10_000\)/,
    'rewind settlement has a bounded wait');
  assert.match(source, /rewindUnsettled/, 'rewind settlement failures are returned');
});

test('input checks carry visible pixels across a rewind, its next frames and a change of pin', () => {
  const frames = [0, 1, 2, 3, 4, 5];
  const clock = (at, hour) => ({ t: 'tick', at, hour, playing: false });
  const picture = (at, hour) => ({ t: 'picture', at, hour, placement: 12 });
  const show = (at, pin = null) => ({ at, shown: [[12, pin]] });
  const records = [clock(0, 200), picture(0, 200), clock(1, 80), picture(2, 80)];
  assert.equal(inputPictures(frames, [show(0)], records).future, 1);
  const visibility = [show(0), { at: 1, shown: [] }, show(2), show(4, 70)];
  assert.equal(inputPictures(frames, visibility, [...records, picture(4, 70)]).future, 0);
  assert.equal(inputPictures(frames, visibility, [...records, picture(4, 70), picture(5, 80)]).future, 1);
});

test('an input applies once every drawing lane has presented its seek, not the first', () => {
  const tick = (at, seek, hour) => ({ t: 'tick', at, hour, playing: false, seek });
  const frame = (at, lane, seek, head) => ({ t: 'workerFrame', at, lane, seek, head, playing: false });
  const picture = (at, seek, hour) => ({ t: 'picture', at, placement: 12, seek, hour, since: hour, through: hour });
  const shown = [{ at: 0, shown: [[12, null, 'w']] }];
  const records = [tick(0, 1, 100), frame(0, 0, 1, 100), frame(0, 1, 1, 100), picture(0, 1, 100),
    tick(10, 2, 200), frame(11, 0, 2, 200), frame(16, 1, 2, 200), picture(16, 2, 200)];
  const checked = inputPictures([0, 12, 20], shown, records);
  assert.equal(checked.wrongSeek, 0, 'the old picture on the lane that has not yet applied is not judged');
  assert.equal(checked.pass, true);
});

test('input marks judge a scrub from the start of its drag and a pin after it applies', () => {
  const tick = { t: 'tick', at: 0, hour: 100, playing: false, seek: 2 };
  const picture = { t: 'picture', at: 15, placement: 1, hour: 80, since: 80, through: 80, seek: 2, fixedHour: 80 };
  const window = { complete: true, from: 0, to: 100, seconds: 1, presented: [0, 50, 100], pressureMerges: 0,
    widgets: [{ id: 'w', kind: 'synthetic', visible: true, surfaces: ['surface'] }],
    audit: [{ widget: 'w', surface: 'surface', placement: 1, at: 500, error: null }],
    inputs: { actions: [{ kind: 'rewind', at: 0 }, { kind: 'scrub', at: 10 }, { kind: 'pin', at: 20, pinWidget: 'w' },
      { kind: 'speed', at: 30 }, { kind: 'scroll', at: 40, notApplicable: 'nothing to scroll' }, { kind: 'pause', at: 50 }],
      to: 100, presented: [15, 25, 30, 50, 75], visibility: [{ at: 10, shown: [[1, null, 'w']] }, { at: 20, shown: [[1, 80, 'w']] }] } };
  const records = [tick, { ...picture, at: 15, hour: 100, since: 100, through: 100, fixedHour: undefined }, { ...picture, at: 25 }];
  assert.equal(inputPictures([25], window.inputs.visibility, records, 'w').pass, true,
    'the pin window starts at the click and uses the pinned widget target');
  assert.equal(inputPictures([25], window.inputs.visibility, records.map(r => r.t === 'picture' ? { ...r, hour: 79 } : r), 'w').pass, false,
    'a picture at the moving head does not satisfy a pinned target');
  assert.equal(datedPlaybackVerdict([window], records).reasons.includes('Missing input coverage for scrub'), false);
  assert.equal(datedPlaybackVerdict([window], records).reasons.includes('Missing input coverage for pin'), false);
});

test('each pinned placement settles on its seek and keeps confirmed pixels through the next input', () => {
  const shown = [{ at: 0, shown: [[1, 80, 'active'], [2, 80, 'other']] }];
  const tick = { t: 'tick', at: 0, playing: false, hour: 90, seek: 3 };
  const picture = { t: 'picture', at: 0, hour: 80, since: 70, through: 80, fixedHour: 80, seek: 3, placement: 1 };
  const check = (...rows) => inputPictures([0, 1, 2], shown, [tick, { ...picture, placement: 2 }, ...rows], 'active').pass;
  assert.equal(check(picture), true);
  assert.equal(check({ ...picture, at: 1 }), true);
  assert.equal(check({ ...picture, hour: 79 }), false);
  assert.equal(check({ ...picture, hour: 79 }, { ...picture, at: 1 }), false);
  assert.equal(check({ ...picture, seek: 2 }), false);
  assert.equal(check({ ...picture, through: 79 }), false);
  assert.equal(check(), false);
  assert.equal(check(picture, { ...picture, at: 1, hour: null }, { ...picture, at: 2 }), false);
  assert.equal(inputPictures([0], [{ at: 0, shown: [[2, 80, 'other']] }], [tick, { ...picture, placement: 2 }], 'active').pass, false);
});

test('a detached pinned placement is removed before its replacement settles', () => {
  const visibility = [
    { at: 0, shown: [[1, 80, 'active']] },
    { at: 2, shown: [[2, 80, 'active']] },
  ];
  const tick = { t: 'tick', at: 0, hour: 90, playing: false, seek: 3 };
  const picture = { t: 'picture', at: 0, hour: 80, since: 70, through: 80, fixedHour: 80, seek: 3 };
  const records = [tick, { ...picture, placement: 1 }, { ...picture, at: 1, placement: 1, detached: true },
    { ...picture, at: 2, placement: 2 }];
  const result = inputPictures([0, 1, 2], visibility, records, 'active');
  assert.equal(result.pass, true);
  assert.equal(result.lost, 0);
});

test('review 6 D: a card scrolled in is judged like the rest, so one that never paints fails', () => {
  const shown = [
    { at: 0, shown: [[1, null, 'active']] },
    { at: 2, shown: [[1, null, 'active'], [2, null, 'active']] },
  ];
  const tick = { t: 'tick', at: 0, hour: 100, playing: false, seek: 3 };
  const picture = { t: 'picture', at: 0, hour: 100, since: 90, through: 100, seek: 3, placement: 1 };
  const blank = inputPictures([0, 1, 2], shown, [tick, picture]);
  assert.deepEqual([blank.pass, blank.missing], [false, [2]], 'the card that never painted is missing');
  const painted = inputPictures([0, 1, 2, 3], shown, [tick, picture, { ...picture, at: 2, placement: 2 }]);
  assert.deepEqual([painted.pass, painted.missing], [true, []]);
});

test('review 6 D: a lane that served twice and then stalled for the rest of the window fails', () => {
  const presented = Array.from({ length: 61 }, (_, i) => i * 1000 / 60);
  const records = [
    { t: 'tick', at: 0, hour: 0, speed: 1440, playing: true },
    { t: 'tick', at: 1000, hour: 1440, speed: 1440, playing: true },
    ...presented.map(at => ({ t: 'picture', at, widget: 'w', surface: 'surface', placement: 1, hour: Math.floor(at * 1.44), head: Math.floor(at * 1.44), eligibleHour: Math.floor(at * 1.44), renderer: 'page' })),
    { t: 'workerFrame', at: 0, playing: true, changed: 1, workMs: 0.1 }, { t: 'workerFrame', at: 10, playing: true, changed: 1, workMs: 0.1 },
    ...presented.map(at => ({ t: 'frame', at, playing: true, commits: 0, commitSequence: 0 })),
  ];
  const window = { page: 'Synthetic', complete: true, from: 0, to: 1000, seconds: 1, presented, pressureMerges: 0,
    widgets: [{ id: 'w', kind: 'synthetic', visible: true, surfaces: ['surface'] }], audit: [{ widget: 'w', surface: 'surface', placement: 1, at: 500, error: null }],
    inputs: { actions: ['rewind', 'scrub', 'pin', 'speed', 'scroll', 'pause'].map(kind => ({ kind, notApplicable: 'synthetic window has no input controls' })), to: 1000, presented: [] } };
  assert.equal(datedPlaybackVerdict([window], records).pass, false);
  const served = [...records.filter(r => r.t !== 'workerFrame'), ...presented.map(at => ({ t: 'workerFrame', at, playing: true, changed: 1, workMs: 0.1 }))];
  assert.equal(datedPlaybackVerdict([window], served).pass, true, 'the same window passes when the lane serves every frame');
});

test('a scrub that presents an empty remount frame judges the held placement', () => {
  const visibility = [
    { at: 0, shown: [] },
    { at: 2, shown: [[1, null, 'active']] },
  ];
  const records = [
    { t: 'tick', at: 0, hour: 100, playing: false, seek: 4 },
    { t: 'workerFrame', at: 0, head: 100, playing: false, seek: 4 },
    { t: 'picture', at: 2, placement: 1, hour: 100, since: 90, through: 100, seek: 4 },
  ];
  const result = inputPictures([0, 1, 2, 3], visibility, records);
  assert.equal(result.checked, 2);
  assert.equal(result.pass, true);
});

test('ink-hidden transition frames do not turn a settled picture into a loss', () => {
  const shown = [{ at: 0, shown: [[1, null, 'active']] }, { at: 2, shown: [] }];
  const records = [
    { t: 'tick', at: 0, hour: 100, playing: false, seek: 3 },
    { t: 'picture', at: 0, hour: 100, since: 90, through: 100, seek: 3, placement: 1 },
    { t: 'ink', at: 1, placement: 1, visible: false },
  ];
  const result = inputPictures([0, 1], shown, records);
  assert.equal(result.pass, true);
  assert.equal(result.lost, 0);
});

test('a rewind may stay pending while the old placement is dropped before the next input', () => {
  const shown = [{ at: 0, shown: [[1, null, 'active']] }, { at: 2, shown: [] }];
  const records = [
    { t: 'tick', at: 0, hour: 100, playing: false, seek: 3 },
    { t: 'picture', at: 0, hour: 100, since: 90, through: 100, seek: 3, placement: 1 },
    { t: 'tick', at: 1, hour: 50, playing: false, seek: 4 },
    { t: 'picture', at: 1, hour: null, seek: 4, placement: 1 },
    { t: 'picture', at: 2, hour: null, seek: 4, placement: 1, detached: true },
  ];
  const result = inputPictures([0, 1, 2], shown, records);
  assert.equal(result.pass, true);
  assert.equal(result.checked, 1, 'the old confirmed picture may be checked before the rewind lands');
});

test('pause input starts judging after the worker applies the paused clock', () => {
  const shown = [{ at: 0, shown: [[1, null, 'active']] }];
  const records = [
    { t: 'tick', at: 0, hour: 100, playing: true, seek: 3 },
    { t: 'picture', at: 0, hour: 100, since: 90, through: 100, seek: 3, placement: 1 },
    { t: 'tick', at: 10, hour: 110, playing: false, seek: 3 },
    { t: 'picture', at: 10, hour: 109, since: 90, through: 109, seek: 3, placement: 1 },
    { t: 'picture', at: 11, hour: 110, since: 90, through: 110, seek: 3, placement: 1 },
    { t: 'workerFrame', at: 11, head: 110, seek: 3, playing: false },
  ];
  const result = inputPictures([0, 10, 11], shown, records);
  assert.equal(result.pass, true);
  assert.equal(result.stale, 0);
});

test('a paused placement needs its exact target and rejects a stale picture after settling', () => {
  const shown = [{ at: 0, shown: [[1, null, 'paused']] }];
  const tick = { t: 'tick', at: 0, playing: false, hour: 80, seek: 4 };
  const picture = { t: 'picture', at: 0, hour: 80, since: 80, through: 80, seek: 4, placement: 1 };
  assert.equal(inputPictures([0, 1], shown, [tick, picture]).pass, true);
  assert.equal(inputPictures([0, 1], shown, [tick, picture, { ...picture, at: 1, hour: 79 }]).pass, false);
  assert.equal(inputPictures([0], shown, [tick]).pass, false);
});

test('Research requires five visible dated widgets, a full playing window and sampled exact values for every visible surface', () => {
  const kinds = ['active-research', 'research', 'special-projects', 'nuclear-arsenal', 'doctrine-mastery'];
  const widgets = kinds.map(kind => ({ id: kind, kind, visible: true, surfaces: [kind] }));
  widgets[2].surfaces.push('special-project-details');
  const audit = widgets.flatMap(w => w.surfaces.map(surface => ({ widget: w.id, surface, placement: surface, hour: 720, at: 500, error: null })));
  const window = { complete: true, from: 0, to: 1000, seconds: 1, widgets, audit, pressureMerges: 0,
    presented: Array.from({ length: 101 }, (_, i) => 10 * i) };
  const records = Array.from({ length: 101 }, (_, i) => {
    const at = 10 * i, hour = Math.floor(at * 1.44);
    return [{ t: 'tick', at, hour: at * 1.44, speed: 1440, playing: true },
      ...widgets.flatMap(w => w.surfaces.map(surface => ({ t: 'picture', at, head: hour, hour, eligibleHour: hour, widget: w.id, surface,
        renderer: 'page', placement: surface, follow: 7, readHour: hour }))),
      ...(i === 0 || Math.floor(i * .6) > Math.floor((i - 1) * .6) ? [{ t: 'workerFrame', at, head: hour, playing: true, changed: 1, workMs: 0.2 }] : []),
      { t: 'frame', at, playing: true, commits: 0 }];
  }).flat();
  window.inputs = { actions: ['rewind','scrub','pin','speed','scroll','pause'].map((kind,i) => ({ kind, at: 1100 + i*100, pinWidget: kind === 'pin' ? 'active-research' : undefined })),
    to: 1700, presented: [1110,1210,1310,1410,1510,1610], visibility: [{ at: 1100, shown: [['research', null, 'research']] },
      { at: 1300, shown: [['research', null, 'research'], ['pin', 30, 'active-research']] }] };
  records.push({ t: 'tick', at: 1100, playing: false, hour: 50, seek: 2 },
    { t: 'picture', at: 1100, placement: 'research', hour: 50, since: 50, through: 50, seek: 2 },
    { t: 'picture', at: 1300, placement: 'pin', hour: 30, since: 30, through: 30, seek: 2, fixedHour: 30 });
  const verdict = (events = records, w = window) => datedPlaybackVerdict([w], events).pass;
  assert.equal(verdict(), true);
  assert.equal(verdict(records, { ...window, inputs: { ...window.inputs,
    actions: window.inputs.actions.map(action => action.kind === 'pin' ? { kind: 'pin', at: action.at } : action) } }), false,
    'a pin action without a pin widget is missing coverage');
  assert.equal(verdict([...records, ...records.filter(r => r.t === 'workerFrame')]), false, 'double presentation exceeds the 60 FPS cap');
  assert.equal(verdict([...records, ...records.filter(r => r.t === 'workerFrame').map(r => ({ ...r, lane: 1 }))]), true,
    'each drawing lane presents at 60 FPS of its own');
  assert.equal(verdict(records.map(r => r.t === 'workerFrame' ? { ...r, lanes: 2 } : r)), false, 'a lane that never reported is counted as not presenting');
  assert.equal(verdict([...records, ...records.filter(r => r.t === 'workerFrame').filter((_, at) => at % 2 === 0).map(r => ({ ...r, lane: 1 }))]), false,
    'a lane presenting fewer frames fails the cap');
  const frames = records.map(r => r.t === 'frame' ? { ...r, commits: r.at === 0 ? 1 : 0, commitSequence: 1 } : r);
  assert.equal(verdict([...frames, { t: 'commit', serial: 1, at: -1, widgets: [], changes: [] }]), true,
    'a commit completed before playback falls outside the window even when counted by its first page frame');
  assert.equal(verdict([...records, ...records.filter(r => r.widget === 'research').map(r => ({ ...r, placement: 'second' }))]), false,
    'each visible placement of a surface needs its own sampled exact read');
  assert.equal(verdict(records.map(r => r.t === 'frame' && r.at === 600 ? { ...r, commits: 1 } : r)), false);
  const committed = records.map(r => r.t === 'frame' && r.at === 600 ? { ...r, commits: 1 } : r);
  const commit = { t: 'commit', at: 599, widgets: ['research'], changes: [{ widget: 'research', source: 'research', hour: 860,
    before: ['a'], after: ['a','b'], verified: true }] };
  assert.equal(verdict([...committed, commit]), true);
  assert.equal(verdict([...committed, { ...commit, changes: [{ ...commit.changes[0], verified: false }] }]), false);
  assert.equal(verdict([...committed, { ...commit, changes: [{ ...commit.changes[0], after: ['a'] }] }]), false);
  assert.equal(verdict([...committed, { ...commit, widgets: ['research', 'special-projects'] }]), false,
    'a shared commit is structural only when every widget it rendered has a verified row change');
  assert.equal(verdict(records.map(r => r.widget === 'nuclear-arsenal' ? { ...r, hour: r.hour - 72 } : r)), false,
    'a late displayed frame without a verified structure crossing fails every placement');
  assert.equal(verdict(records.map(r => r.widget === 'research' ? { ...r, hour: r.hour - 72 } : r)), false);
  assert.equal(verdict(records.map(r => r.widget === 'research' ? { ...r, eligibleHour: r.hour - 72 } : r)), false);
  assert.equal(verdict(records.map(r => r.widget === 'research' ? { ...r, eligibleHour: null } : r)), false);
  const fourActions = window.inputs.actions.filter(action => !['pin', 'scroll'].includes(action.kind));
  assert.equal(verdict(records, { ...window, inputs: { ...window.inputs, actions: fourActions } }), false,
    'four recorded actions without applicability records leave canonical input coverage missing');
  const actionsWithApplicability = [...fourActions,
    { kind: 'pin', at: 1300, notApplicable: 'no widget offers a pinnable time setting' },
    { kind: 'scroll', at: 1500, notApplicable: 'nothing to scroll' }].toSorted((a, b) => a.at - b.at);
  assert.equal(verdict(records, { ...window, inputs: { ...window.inputs, actions: actionsWithApplicability } }), true);
  const mixedApplicability = [...fourActions,
    { kind: 'pin', at: 1300, notApplicable: 'no widget offers a pinnable time setting' },
    { kind: 'pin', at: 1400 },
    { kind: 'scroll', at: 1500, notApplicable: 'nothing to scroll' }].toSorted((a, b) => a.at - b.at);
  const mixedInputs = { ...window.inputs, actions: mixedApplicability,
    presented: [...window.inputs.presented, 1410], visibility: [...window.inputs.visibility, { at: 1400, shown: [['research', null, 'research'], ['pin', 30, 'active-research']] }] };
  assert.equal(verdict(records, { ...window, inputs: mixedInputs }), false,
    'one not-applicable action cannot suppress another action of the same kind');
  assert.equal(verdict(records, { ...window, expectedWidgets: [...widgets.map(widget => ({ id: widget.id })), { id: 'missing-widget' }] }), false,
    'an expected widget without an inventory entry fails coverage');
  assert.equal(verdict(records, { ...window, audit: [] }), false);
  assert.equal(verdict(records, { ...window, audit: audit.map((a,i) => i ? a : { ...a, error: 'Wrong row' }) }), false);
  assert.equal(verdict(records, { ...window, pressureMerges: 1 }), false);
  assert.equal(verdict(records, { ...window, presented: [] }), false);
  assert.equal(verdict(records.filter(r => r.t !== 'frame')), false);
  assert.equal(verdict(records.filter(r => r.at < 100)), false);
  assert.equal(verdict(records.map(r => ({ ...r, hour: 0, head: 0, cellHour: 0, readHour: 0 }))), false);
  const afterPause = [...records.map(r => ({ ...r, renderer: 'dom' })), ...records.filter(r => r.t === 'picture').map(r => ({...r, at: 1020}))];
  assert.equal(verdict(afterPause, { ...window, to: 1030 }), false);
  assert.equal(verdict(records, { ...window, widgets: [] }), false);
  assert.equal(verdict(records, { ...window, widgets: widgets.slice(1) }), true,
    'the page verdict judges only the placements the selected page presents');
  assert.equal(verdict(records.filter(r => r.widget !== 'research')), false);
  const hidden = records.filter(r => !(r.surface === 'special-project-details' && r.at >= 100));
  assert.equal(verdict(hidden), false);
  assert.equal(verdict(hidden, { ...window, visibility: [{ at: 100, id: 'special-projects', surface: 'special-project-details', visible: false }] }), true);
  assert.equal(verdict(records.map(r => r.t === 'picture' ? { ...r, hour: r.hour - 400 } : r)), false);
  assert.equal(verdict([...records, { ...records[1], hour: 1 }]), false);
  assert.equal(verdict(records.map(r => r.t === 'picture' ? { ...r, shownAt: r.at + 1e9 } : r)), false,
    'a picture is held only from a display its frame could reach');
  assert.equal(verdict(records.map(r => ({ ...r, renderer: 'dom' }))), false);
  assert.equal(verdict(records.map(r => r.t === 'picture' ? { ...r, renderer: 'page' } : r)), true, 'astra 15: a picture the page installed is measured, not discarded');
  assert.equal(verdict(records.filter(r => r.t !== 'workerFrame')), false);
  const one = records.find(r => r.t === 'picture' && r.at > 300);
  assert.equal(verdict([...records, { ...one, at: one.at + 10, hour: null }]), false,
    'review 4 finding 15: one blank frame in a hundred fails');
});

test('placement visibility keeps a hidden surface out of the held-frame gate until it is shown', () => {
  const presented = Array.from({ length: 61 }, (_, i) => i * 1000 / 60);
  const dated = at => ({ t: 'picture', at, widget: 'w', surface: 'surface', placement: 1, hour: Math.floor(at * 1.44), head: Math.floor(at * 1.44), eligibleHour: Math.floor(at * 1.44), renderer: 'page' });
  const hidden = at => ({ ...dated(at), placement: 2, hour: null, eligibleHour: null });
  const shown = at => ({ ...dated(at), placement: 2 });
  const records = [
    { t: 'tick', at: 0, hour: 0, speed: 1440, playing: true },
    { t: 'tick', at: 1000, hour: 1440, speed: 1440, playing: true },
    ...presented.map(at => dated(at)),
    ...presented.filter(at => at < 500).map(at => hidden(at)),
    ...presented.filter(at => at >= 500).map(at => shown(at)),
    ...presented.map(at => ({ t: 'workerFrame', at, playing: true, changed: 0, workMs: 0.1 })),
    ...presented.map(at => ({ t: 'frame', at, playing: true, commits: 0, commitSequence: 0 })),
  ];
  const window = { page: 'Synthetic', complete: true, from: 0, to: 1000, seconds: 1, presented, pressureMerges: 0,
    widgets: [{ id: 'w', kind: 'synthetic', visible: true, surfaces: ['surface'] }],
    visibility: [{ id: 'w', placement: 1, at: 0, visible: true }, { id: 'w', placement: 2, at: 0, visible: false }, { id: 'w', placement: 2, at: 500, visible: true }],
    audit: [{ widget: 'w', surface: 'surface', placement: 1, at: 500, error: null }, { widget: 'w', surface: 'surface', placement: 2, at: 500, error: null }],
    inputs: { actions: ['rewind', 'scrub', 'pin', 'speed', 'scroll', 'pause'].map(kind => ({ kind, notApplicable: 'synthetic window has no input controls' })), to: 1000, presented: [] } };
  assert.equal(datedPlaybackVerdict([window], records).pass, true);
});

test('the dated verdict rejects a stale pinned picture after it has settled', () => {
  const presented = Array.from({ length: 61 }, (_, i) => i * 1000 / 60);
  const records = [
    { t: 'tick', at: 0, hour: 0, speed: 1440, playing: true },
    { t: 'tick', at: 1000, hour: 1440, speed: 1440, playing: true },
    ...presented.map(at => ({ t: 'picture', at, widget: 'w', surface: 'w', placement: 'p', hour: Math.floor(at * 1.44), head: Math.floor(at * 1.44), eligibleHour: Math.floor(at * 1.44), renderer: 'page' })),
    ...presented.map(at => ({ t: 'workerFrame', at, playing: true, changed: 0, workMs: 0.1 })),
    ...presented.map(at => ({ t: 'frame', at, playing: true, commits: 0, commitSequence: 0 })),
    { t: 'tick', at: 1100, hour: 10, playing: false, seek: 1 },
    { t: 'picture', at: 1100, widget: 'w', surface: 'w', placement: 'p', hour: 10, seek: 1, fixedHour: 10, since: 10, through: 10 },
    { t: 'picture', at: 1210, widget: 'w', surface: 'w', placement: 'p', hour: 9, seek: 1, fixedHour: 10, since: 10, through: 10 },
  ];
  const window = { complete: true, from: 0, to: 1000, seconds: 1, presented, pressureMerges: 0,
    widgets: [{ id: 'w', kind: 'synthetic', visible: true, surfaces: ['w'] }],
    audit: [{ widget: 'w', surface: 'w', placement: 'p', at: 500, error: null }],
    inputs: { actions: [{ kind: 'pin', at: 1100, pinWidget: 'w' }], to: 1300, presented: [1110, 1210],
      visibility: [{ at: 1100, shown: [['p', 10, 'w']] }] } };
  assert.equal(datedPlaybackVerdict([window], records).pass, false);
});

test('displayed age p95 fails even when every late frame crosses a verified structure boundary', () => {
  const presented = Array.from({ length: 61 }, (_, i) => i * 1000 / 60);
  const target = at => 1000 + at * 1.44;
  const records = [
    { t: 'tick', at: 0, hour: 1000, speed: 1440, playing: true },
    { t: 'tick', at: 1000, hour: 2440, speed: 1440, playing: true },
    ...presented.map(at => ({ t: 'picture', at, widget: 'w', surface: 's', placement: 1,
      hour: Math.floor(target(at) - 109), head: Math.floor(target(at)), eligibleHour: Math.floor(target(at)), renderer: 'page' })),
    ...presented.map(at => ({ t: 'workerFrame', at, playing: true, changed: 0, workMs: 0.1 })),
    ...presented.map((at, index) => ({ t: 'frame', at, playing: true, commits: index === 0 ? 1 : 0, commitSequence: 1 })),
    { t: 'commit', at: 0, serial: 1, widgets: ['w'], changes: [{ widget: 'w', source: 's', hour: 100000,
      previousHour: 0, before: ['a'], after: ['b'], verified: true }] },
  ];
  const window = { complete: true, from: 0, to: 1000, seconds: 1, presented, pressureMerges: 0,
    widgets: [{ id: 'w', kind: 'synthetic', visible: true, surfaces: ['s'] }],
    audit: [{ widget: 'w', surface: 's', placement: 1, at: 500, error: null }],
    inputs: { actions: ['rewind', 'scrub', 'pin', 'speed', 'scroll', 'pause'].map(kind => ({ kind, notApplicable: 'synthetic' })), to: 1000, presented: [] } };
  const result = datedPlaybackVerdict([window], records);
  assert.equal(result.placements[0].p95, 109);
  assert.equal(result.placements[0].structureCrossings > 0, true);
  assert.equal(result.pass, false, 'structure crossings do not exempt displayed age');
});

test('an unmatched or ambiguous commit source fails instead of being assigned by surface position', () => {
  const presented = Array.from({ length: 61 }, (_, i) => i * 1000 / 60);
  const picture = (surface, placement = surface) => presented.map(at => ({ t: 'picture', at, widget: 'w', surface, placement,
    hour: Math.floor(at * 1.44), head: Math.floor(at * 1.44), eligibleHour: Math.floor(at * 1.44), renderer: 'page' }));
  const records = [
    { t: 'tick', at: 0, hour: 0, speed: 1440, playing: true },
    { t: 'tick', at: 1000, hour: 1440, speed: 1440, playing: true },
    ...picture('one', 1), ...picture('two', 2),
    ...presented.map(at => ({ t: 'workerFrame', at, playing: true, changed: 0, workMs: 0.1 })),
    ...presented.map((at, index) => ({ t: 'frame', at, playing: true, commits: index === 0 ? 1 : 0, commitSequence: 1 })),
    { t: 'commit', at: 0, serial: 1, widgets: ['w'], changes: [{ widget: 'w', source: 'missing-source', hour: 100,
      previousHour: 0, before: ['a'], after: ['b'], verified: true }] },
  ];
  const base = { complete: true, from: 0, to: 1000, seconds: 1, presented, pressureMerges: 0,
    widgets: [{ id: 'w', kind: 'synthetic', visible: true, surfaces: ['one', 'two'] }],
    audit: [{ widget: 'w', surface: 'one', placement: 1, at: 500, error: null }, { widget: 'w', surface: 'two', placement: 2, at: 500, error: null }],
    inputs: { actions: ['rewind', 'scrub', 'pin', 'speed', 'scroll', 'pause'].map(kind => ({ kind, notApplicable: 'synthetic' })), to: 1000, presented: [] } };
  const unmatched = datedPlaybackVerdict([base], records);
  assert.equal(unmatched.pass, false);
  assert.ok(unmatched.reasons.some(reason => reason.includes('Unmapped commit source') && reason.includes('missing-source')));
  const ambiguousWindow = { ...base, widgets: [{ ...base.widgets[0], surfaces: ['foo-bar', 'foobar'] }],
    audit: [{ widget: 'w', surface: 'foo-bar', placement: 1, at: 500, error: null }, { widget: 'w', surface: 'foobar', placement: 2, at: 500, error: null }] };
  const ambiguousRecords = records.map(record => record.t === 'picture' ? { ...record, surface: record.placement === 1 ? 'foo-bar' : 'foobar' } : record)
    .map(record => record.t === 'commit' ? { ...record, changes: [{ ...record.changes[0], source: 'FOO BAR' }] } : record);
  const ambiguous = datedPlaybackVerdict([ambiguousWindow], ambiguousRecords);
  assert.equal(ambiguous.pass, false);
  assert.ok(ambiguous.reasons.some(reason => reason.includes('Ambiguous commit source') && reason.includes('FOO BAR')));
});

test('a visible placeholder is reported as not migrated and leaves the page unmeasured', () => {
  const presented = Array.from({ length: 61 }, (_, i) => i * 1000 / 60);
  const records = [
    { t: 'tick', at: 0, hour: 0, speed: 1440, playing: true },
    { t: 'tick', at: 1000, hour: 1440, speed: 1440, playing: true },
    ...presented.map(at => ({ t: 'workerFrame', at, playing: true, changed: 0, workMs: 0.1 })),
    ...presented.map(at => ({ t: 'frame', at, playing: true, commits: 0, commitSequence: 0 })),
  ];
  const window = { complete: true, from: 0, to: 1000, seconds: 1, presented, pressureMerges: 0,
    widgets: [{ id: 'legacy', kind: 'legacy-kind', title: 'Legacy', visible: true, surfaces: [], notMigrated: true }] };
  const result = datedPlaybackVerdict([window], records);
  assert.equal(result.pass, false);
  assert.ok(result.reasons.includes('nothing measured'));
  assert.deepEqual(result.placements.map(p => [p.kind, p.status, p.pass]), [['legacy-kind', 'not migrated', null]]);
  assert.deepEqual(result.widgets.map(w => [w.kind, w.notMigrated, w.pass]), [['legacy-kind', 1, null]]);
});

test('a future picture counts once the worker grace window has passed, and for widgets mounted after the input', () => {
  const tick = { t: 'tick', at: 0, hour: 100, playing: false, seek: 2 };
  const ahead = (placement, at) => ({ t: 'picture', at, placement, hour: 150, since: 150, through: 150, seek: 2 });
  const one = [{ at: 0, shown: [['a', null, 'w']] }];
  const applied = { t: 'workerFrame', at: 50, seek: 2, playing: false, head: 100 };
  assert.equal(inputPictures([10, 60], one, [tick, ahead('a', 1), applied], null).future, 1,
    'inside the grace window an old picture is not judged; after the worker presents, a future one counts');
  const unapplied = { t: 'workerFrame', at: 30, seek: 1, playing: false, head: 100 };
  assert.equal(inputPictures([10, 40], one, [tick, ahead('a', 1), unapplied], null).future, 1,
    'past the grace window a future picture counts even before the worker presents the new anchor');
  const later = [{ at: 0, shown: [['a', null, 'w']] }, { at: 50, shown: [['a', null, 'w'], ['late', null, 'w']] }];
  const early = { t: 'workerFrame', at: 35, seek: 2, playing: false, head: 100 };
  assert.equal(inputPictures([40, 60], later, [tick, early, ahead('late', 55)], null).future, 1,
    'a widget mounted after the input is still checked for future pictures');
});

test('a playing wrong-seek picture still fails after a valid picture arrives', () => {
  const shown = [{ at: 0, shown: [[1, null, 'w']] }];
  const records = [
    { t: 'tick', at: 0, hour: 10, playing: true, speed: 1440, seek: 2 },
    { t: 'picture', at: 0, placement: 1, hour: 10, since: 10, through: 10, seek: 1 },
    { t: 'picture', at: 1, placement: 1, hour: 11, since: 10, through: 11, seek: 2 },
  ];
  const result = inputPictures([0, 1], shown, records);
  assert.equal(result.wrongSeek, 1);
  assert.equal(result.pass, false);
});

test('a pin action with no settled pictures fails input coverage', () => {
  const result = inputPictures([0, 1], [{ at: 0, shown: [[1, 20, 'w']] }], [
    { t: 'tick', at: 0, hour: 20, playing: false, seek: 4 },
  ], 'w');
  assert.equal(result.pass, false);
  assert.deepEqual(result.missing, [1]);
});

test('an unsettled scrub is reported as a verdict failure with each widget reason', () => {
  const presented = Array.from({ length: 61 }, (_, i) => i * 1000 / 60);
  const window = { complete: true, from: 0, to: 1000, seconds: 1, presented, pressureMerges: 0,
    widgets: [{ id: 'w', kind: 'synthetic', visible: true, surfaces: ['surface'] }],
    audit: [{ widget: 'w', surface: 'surface', placement: 1, at: 500, error: null }],
    inputs: { actions: ['rewind', 'scrub', 'pin', 'speed', 'scroll', 'pause'].map(kind => ({ kind, notApplicable: 'synthetic window' })),
      unsettled: [{ widget: 'Nuclear arsenal', reason: 'blank' }] } };
  const records = [
    { t: 'tick', at: 0, hour: 0, speed: 1440, playing: true },
    { t: 'tick', at: 1000, hour: 1440, speed: 1440, playing: true },
    ...presented.map(at => ({ t: 'picture', at, widget: 'w', surface: 'surface', placement: 1, hour: Math.floor(at * 1.44), head: Math.floor(at * 1.44), eligibleHour: Math.floor(at * 1.44), renderer: 'page' })),
    ...presented.map(at => ({ t: 'workerFrame', at, playing: true, changed: 0, workMs: 0.1 })),
    ...presented.map(at => ({ t: 'frame', at, playing: true, commits: 0, commitSequence: 0 })),
  ];
  const verdict = datedPlaybackVerdict([window], records);
  assert.equal(verdict.pass, false);
  assert.ok(verdict.reasons.includes('Dashboard scrub did not settle: Nuclear arsenal: blank'));
  assert.deepEqual(verdict.metrics[0].inputs.at(-1), { kind: 'scrub-settle', widget: 'Nuclear arsenal', reason: 'blank', pass: false });
});

test('an unsettled rewind is reported as a verdict failure with its wait time', () => {
  const presented = Array.from({ length: 61 }, (_, i) => i * 1000 / 60);
  const window = { complete: true, from: 0, to: 1000, seconds: 1, presented, pressureMerges: 0,
    widgets: [{ id: 'w', kind: 'synthetic', visible: true, surfaces: ['surface'] }],
    audit: [{ widget: 'w', surface: 'surface', placement: 1, at: 500, error: null }],
    inputs: { actions: ['rewind', 'scrub', 'pin', 'speed', 'scroll', 'pause'].map(kind => ({ kind, notApplicable: 'synthetic window' })),
      rewindSettledMs: 10_000, rewindUnsettled: [{ widget: 'Nuclear arsenal', reason: 'blank' }] } };
  const records = [
    { t: 'tick', at: 0, hour: 0, speed: 1440, playing: true },
    { t: 'tick', at: 1000, hour: 1440, speed: 1440, playing: true },
    ...presented.map(at => ({ t: 'picture', at, widget: 'w', surface: 'surface', placement: 1, hour: Math.floor(at * 1.44), head: Math.floor(at * 1.44), eligibleHour: Math.floor(at * 1.44), renderer: 'page' })),
    ...presented.map(at => ({ t: 'workerFrame', at, playing: true, changed: 0, workMs: 0.1 })),
    ...presented.map(at => ({ t: 'frame', at, playing: true, commits: 0, commitSequence: 0 })),
  ];
  const verdict = datedPlaybackVerdict([window], records);
  assert.equal(verdict.pass, false);
  assert.ok(verdict.reasons.includes('Dashboard rewind did not settle: Nuclear arsenal: blank'));
  assert.equal(verdict.metrics[0].rewindSettledMs, 10_000);
  assert.deepEqual(verdict.metrics[0].inputs.at(-1), { kind: 'rewind-settle', widget: 'Nuclear arsenal', reason: 'blank', pass: false });
});

test('stale paused evidence survives a new seek and is counted', () => {
  const shown = [{ at: 0, shown: [[1, null, 'w']] }];
  const records = [
    { t: 'tick', at: 0, hour: 80, playing: false, seek: 1 },
    { t: 'workerFrame', at: 0, head: 80, playing: false, seek: 1 },
    { t: 'picture', at: 0, placement: 1, hour: 80, since: 80, through: 80, seek: 1 },
    { t: 'tick', at: 1, hour: 70, playing: false, seek: 2 },
    { t: 'picture', at: 1, placement: 1, hour: 80, since: 80, through: 80, seek: 1 },
    { t: 'workerFrame', at: 40, head: 70, playing: false, seek: 2 },
    { t: 'picture', at: 50, placement: 1, hour: 70, since: 70, through: 70, seek: 2 },
  ];
  const result = inputPictures([0, 1, 40, 50], shown, records);
  assert.ok(result.stale > 0);
  assert.equal(result.pass, false);
});

test('a visible two-surface widget fails exact coverage when one surface reports', () => {
  const presented = Array.from({ length: 61 }, (_, i) => i * 1000 / 60);
  const records = [
    { t: 'tick', at: 0, hour: 0, speed: 1440, playing: true },
    { t: 'tick', at: 1000, hour: 1440, speed: 1440, playing: true },
    ...presented.map(at => ({ t: 'picture', at, widget: 'w', surface: 'one', placement: 1, hour: Math.floor(at * 1.44), head: Math.floor(at * 1.44), eligibleHour: Math.floor(at * 1.44), renderer: 'page' })),
    ...presented.map(at => ({ t: 'workerFrame', at, playing: true, changed: 0, workMs: 0.1 })),
    ...presented.map(at => ({ t: 'frame', at, playing: true, commits: 0, commitSequence: 0 })),
  ];
  const window = { complete: true, from: 0, to: 1000, seconds: 1, presented, pressureMerges: 0,
    widgets: [{ id: 'w', kind: 'synthetic', visible: true, surfaces: ['one', 'two'] }],
    audit: [{ widget: 'w', surface: 'one', placement: 1, at: 500, error: null }, { widget: 'w', surface: 'two', placement: 2, at: 500, error: null }],
    inputs: { actions: ['rewind', 'scrub', 'pin', 'speed', 'scroll', 'pause'].map(kind => ({ kind, notApplicable: 'synthetic window has no input controls' })), to: 1000, presented: [] } };
  const result = datedPlaybackVerdict([window], records);
  assert.equal(result.placements[0].exact, false);
  assert.equal(result.pass, false);
  const undeclared = datedPlaybackVerdict([{ ...window, widgets: [{ ...window.widgets[0], surfaces: [] }],
    audit: [{ widget: 'w', surface: 'one', placement: 1, at: 500, error: null }] }], records);
  assert.equal(undeclared.placements[0].exact, false, 'reported surfaces do not become required surfaces');
  assert.equal(undeclared.pass, false);
});

test('an empty window list fails with nothing measured', () => {
  const result = datedPlaybackVerdict([], []);
  assert.equal(result.pass, false);
  assert.deepEqual(result.reasons, ['nothing measured']);
});

test('review 7 F: a paused placement may keep its shown entry dimmed until the covering one lands; a different uncovered one is stale', () => {
  const shown = [{ at: 0, shown: [[1, null, 'active']] }];
  const tick = { t: 'tick', at: 0, hour: 120, playing: false, seek: 3 };
  const held = { t: 'picture', at: 0, hour: 110, since: 100, through: 110, seek: 3, placement: 1, dim: true, entry: 7 };
  const landed = { ...held, at: 2, hour: 120, through: 120, dim: false, entry: 8 };
  assert.deepEqual([inputPictures([0, 1, 2, 3], shown, [held, tick, landed]).pass, inputPictures([0, 1, 2, 3], shown, [held, tick, landed]).stale], [true, 0], 'the held picture then its replacement');
  const other = { ...held, at: 1, entry: 9 };
  assert.equal(inputPictures([0, 1, 2, 3], shown, [held, tick, other, landed]).stale > 0, true, 'a different uncovered picture is caught');
});

test('review 7 F: every lane is held to both rate bounds', () => {
  const presented = Array.from({ length: 61 }, (_, i) => i * 1000 / 60);
  const records = [
    { t: 'tick', at: 0, hour: 0, speed: 1440, playing: true },
    { t: 'tick', at: 1000, hour: 1440, speed: 1440, playing: true },
    ...presented.map(at => ({ t: 'picture', at, widget: 'w', surface: 'surface', placement: 1, hour: Math.floor(at * 1.44), head: Math.floor(at * 1.44), eligibleHour: Math.floor(at * 1.44), renderer: 'page' })),
    ...presented.map(at => ({ t: 'workerFrame', at, lane: 0, lanes: 2, playing: true, changed: 1, workMs: 0.1 })),
    ...presented.flatMap(at => [at, at + 8].map(when => ({ t: 'workerFrame', at: when, lane: 1, lanes: 2, playing: true, changed: 1, workMs: 0.1 }))).filter(r => r.at <= 1000),
    ...presented.map(at => ({ t: 'frame', at, playing: true, commits: 0, commitSequence: 0 })),
  ];
  const window = { page: 'Synthetic', complete: true, from: 0, to: 1000, seconds: 1, presented, pressureMerges: 0,
    widgets: [{ id: 'w', kind: 'synthetic', visible: true, surfaces: ['surface'] }], audit: [{ widget: 'w', surface: 'surface', placement: 1, at: 500, error: null }],
    inputs: { actions: ['rewind', 'scrub', 'pin', 'speed', 'scroll', 'pause'].map(kind => ({ kind, notApplicable: 'synthetic window has no input controls' })), to: 1000, presented: [] } };
  assert.equal(datedPlaybackVerdict([window], records).pass, false, 'a lane at 120/s on a 60 FPS display fails though the other is at 60');
});

test('review 8 5: the held entry is the one shown when the pause began, not the first one judged after it', () => {
  const shown = [{ at: 0, shown: [[1, null, 'active']] }];
  const before = { t: 'picture', at: 0, hour: 110, since: 100, through: 110, seek: 3, placement: 1, dim: false, entry: 7 };
  const tick = { t: 'tick', at: 0, hour: 120, playing: false, seek: 3 };
  const other = { ...before, at: 0.5, dim: true, entry: 9 };
  const landed = { ...before, at: 2, hour: 120, through: 120, entry: 8 };
  assert.equal(inputPictures([1, 2, 3], shown, [before, tick, other, landed]).stale > 0, true, 'a different uncovered picture landing before the first judged frame is stale');
  assert.equal(inputPictures([1, 2, 3], shown, [before, tick, { ...before, at: 0.5, dim: true }, landed]).stale, 0, 'the paused entry itself, dimmed, is held');
});

test('review 9: a paused picture with the same displayed content is the held one, whatever its entry id', () => {
  const shown = [{ at: 0, shown: [[1, null, 'active']] }];
  const before = { t: 'picture', at: 0, hour: 110, since: 100, through: 110, seek: 3, placement: 1, dim: false, entry: 7, face: 'model:3=structure:A' };
  const tick = { t: 'tick', at: 0, hour: 120, playing: false, seek: 3 };
  const landed = { ...before, at: 2, hour: 120, through: 120, entry: 8 };
  assert.equal(inputPictures([1, 2, 3], shown, [before, tick, { ...before, at: 0.5, dim: true, entry: 11 }, landed]).stale, 0, 'confirmation moving on gives a new id, not new content');
  assert.equal(inputPictures([1, 2, 3], shown, [before, tick, { ...before, at: 0.5, dim: true, entry: 11, face: 'model:3=structure:B' }, landed]).stale > 0, true);
});

test('review 9: a table placement passes exact content only on its own renderer\'s comparison', () => {
  const presented = Array.from({ length: 61 }, (_, i) => i * 1000 / 60);
  const records = [
    { t: 'tick', at: 0, hour: 0, speed: 1440, playing: true },
    { t: 'tick', at: 1000, hour: 1440, speed: 1440, playing: true },
    ...presented.map(at => ({ t: 'picture', at, widget: 'w', surface: 'table', placement: 1, hour: Math.floor(at * 1.44), head: Math.floor(at * 1.44), eligibleHour: Math.floor(at * 1.44), renderer: 'page' })),
    ...presented.map(at => ({ t: 'workerFrame', at, playing: true, changed: 1, workMs: 0.1 })),
    ...presented.map(at => ({ t: 'frame', at, playing: true, commits: 0, commitSequence: 0 })),
  ];
  const window = owner => ({ page: 'Synthetic', complete: true, from: 0, to: 1000, seconds: 1, presented, pressureMerges: 0,
    widgets: [{ id: 'w', kind: 'synthetic', visible: true, surfaces: ['table'] }], audit: [{ widget: 'w', surface: 'table', placement: 1, owner, at: 500, error: null }],
    inputs: { actions: ['rewind', 'scrub', 'pin', 'speed', 'scroll', 'pause'].map(kind => ({ kind, notApplicable: 'synthetic window has no input controls' })), to: 1000, presented: [] } });
  assert.equal(datedPlaybackVerdict([window(-1)], records).pass, true, 'the table renderer compared its own rows');
  assert.equal(datedPlaybackVerdict([window(-1000)], records).pass, false, 'the raw panel answer at the same placement is not the table\'s comparison');
  assert.equal(datedPlaybackVerdict([window(undefined)], records).pass, false, 'review 10: a comparison without an owner is not the renderer\'s');
  const late = window(-1);
  late.audit = late.audit.map(sample => ({ ...sample, at: 1500 }));
  assert.equal(datedPlaybackVerdict([late], records).pass, false, 'review 10: a comparison from the later idle phase does not stand for the judged window');
  const paused = { ...window(-1), to: 2000 };
  paused.audit = paused.audit.map(sample => ({ ...sample, at: 1500 }));
  assert.equal(datedPlaybackVerdict([paused], records).pass, false, 'review 11: a comparison after the last judged frame, still inside the window, does not count');
});
