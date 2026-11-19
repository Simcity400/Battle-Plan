import { test } from 'node:test';
import assert from 'node:assert/strict';
import { inputPictures, researchVerdict } from './research.mjs';

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

test('late remounts are deferred to the next input boundary', () => {
  const shown = [
    { at: 0, shown: [[1, null, 'active']] },
    { at: 2, shown: [[1, null, 'active'], [2, null, 'active']] },
  ];
  const tick = { t: 'tick', at: 0, hour: 100, playing: false, seek: 3 };
  const picture = { t: 'picture', at: 0, hour: 100, since: 90, through: 100, seek: 3, placement: 1 };
  const result = inputPictures([0, 1, 2], shown, [tick, picture]);
  assert.equal(result.pass, true);
  assert.deepEqual(result.missing, []);
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

test('Research requires five visible dated widgets, a full playing window and sampled exact values for every visible surface', () => {
  const kinds = ['active-research', 'research', 'special-projects', 'nuclear-arsenal', 'doctrine-mastery'];
  const widgets = kinds.map(kind => ({ id: kind, kind, visible: true, surfaces: [kind] }));
  widgets[2].surfaces.push('special-project-details');
  const audit = widgets.flatMap(w => w.surfaces.map(surface => ({ widget: w.id, surface, placement: surface, hour: 720, error: null })));
  const window = { complete: true, from: 0, to: 1000, seconds: 1, widgets, audit, pressureMerges: 0,
    presented: Array.from({ length: 101 }, (_, i) => 10 * i) };
  const records = Array.from({ length: 101 }, (_, i) => {
    const at = 10 * i, hour = Math.floor(at * 1.44);
    return [{ t: 'tick', at, hour: at * 1.44, speed: 1440, playing: true },
      ...widgets.flatMap(w => w.surfaces.map(surface => ({ t: 'picture', at, head: hour, hour, eligibleHour: hour, widget: w.id, surface,
        renderer: 'worker', placement: surface, follow: 7, readHour: hour }))),
      ...(i === 0 || Math.floor(i * .6) > Math.floor((i - 1) * .6) ? [{ t: 'workerFrame', at, head: hour, playing: true, changed: 1, workMs: 0.2 }] : []),
      { t: 'frame', at, playing: true, commits: 0 }];
  }).flat();
  window.inputs = { actions: ['rewind','scrub','pin','speed','scroll','pause'].map((kind,i) => ({ kind, at: 1100 + i*100, pinWidget: kind === 'pin' ? 'active-research' : undefined })),
    to: 1700, presented: [1110,1210,1310,1410,1510,1610], visibility: [{ at: 1100, shown: [['research', null, 'research']] },
      { at: 1300, shown: [['research', null, 'research'], ['pin', 30, 'active-research']] }] };
  records.push({ t: 'tick', at: 1100, playing: false, hour: 50, seek: 2 },
    { t: 'picture', at: 1100, placement: 'research', hour: 50, since: 50, through: 50, seek: 2 },
    { t: 'picture', at: 1300, placement: 'pin', hour: 30, since: 30, through: 30, seek: 2, fixedHour: 30 });
  const verdict = (events = records, w = window) => researchVerdict([w], events).pass;
  assert.equal(verdict(), true);
  assert.equal(verdict([...records, ...records.filter(r => r.t === 'workerFrame')]), false, 'double presentation exceeds the 60 FPS cap');
  const frames = records.map(r => r.t === 'frame' ? { ...r, commits: r.at === 0 ? 1 : 0, commitSequence: 1 } : r);
  assert.equal(verdict([...frames, { t: 'commit', serial: 1, at: -1, widgets: [], changes: [] }]), true,
    'a commit completed before playback falls outside the window even when counted by its first page frame');
  assert.equal(verdict([...records, ...records.filter(r => r.widget === 'research').map(r => ({ ...r, placement: 'second' }))]), false,
    'each visible placement of a surface needs its own sampled exact read');
  assert.equal(verdict(records.map(r => r.t === 'frame' && r.at === 600 ? { ...r, commits: 1 } : r)), false);
  const committed = records.map(r => r.t === 'frame' && r.at === 600 ? { ...r, commits: 1 } : r);
  const commit = { t: 'commit', at: 599, widgets: ['research'], changes: [{ widget: 'research', source: 'research history', hour: 860,
    before: ['a'], after: ['a','b'], verified: true }] };
  assert.equal(verdict([...committed, commit]), true);
  assert.equal(verdict([...committed, { ...commit, changes: [{ ...commit.changes[0], verified: false }] }]), false);
  assert.equal(verdict([...committed, { ...commit, changes: [{ ...commit.changes[0], after: ['a'] }] }]), false);
  assert.equal(verdict([...committed, { ...commit, widgets: ['research', 'special-projects'] }]), false,
    'a shared commit is structural only when every widget it rendered has a verified row change');
  assert.equal(verdict(records.map(r => r.widget === 'nuclear-arsenal' ? { ...r, hour: r.hour - 72 } : r)), false);
  assert.equal(verdict(records.map(r => r.widget === 'research' ? { ...r, hour: r.hour - 72 } : r)), true);
  assert.equal(verdict(records.map(r => r.widget === 'research' ? { ...r, eligibleHour: r.hour - 72 } : r)), false);
  assert.equal(verdict(records.map(r => r.widget === 'research' ? { ...r, eligibleHour: null } : r)), false);
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
  assert.equal(verdict(records, { ...window, widgets: widgets.slice(1) }), false);
  assert.equal(verdict(records.filter(r => r.widget !== 'research')), false);
  const hidden = records.filter(r => !(r.surface === 'special-project-details' && r.at >= 100));
  assert.equal(verdict(hidden), false);
  assert.equal(verdict(hidden, { ...window, visibility: [{ at: 100, id: 'special-projects', surface: 'special-project-details', visible: false }] }), true);
  assert.equal(verdict(records.map(r => r.t === 'picture' ? { ...r, hour: r.hour - 400 } : r)), false);
  assert.equal(verdict([...records, { ...records[1], hour: 1 }]), false);
  assert.equal(verdict(records.map(r => ({ ...r, renderer: 'dom' }))), false);
  assert.equal(verdict(records.filter(r => r.t !== 'workerFrame')), false);
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
