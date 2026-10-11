import assert from 'node:assert/strict';
import test from 'node:test';
import type { ConstructionLogManifestReply, ConstructionLogReply, ConstructionOutputRun, ConstructionProject, ProjectOutput, SpendingGap } from '../../generated/dashboard';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { resolveCountries } from '../../dashboard/groups.ts';
import { definition } from './definition.ts';
import { buildingChoices, builtLabel, constructionWindows, describeRow as datedDescribeRow, factoriesShown, jumpHours, kindLabel, levelsLabel, logReach, logRows, mergeWindows, outputAt, placeLabel as datedPlaceLabel, projectAt, projectTooltip as datedProjectTooltip, statusLabel, validateConstructionLogManifest, validateConstructionLogWindow } from './constructionData.ts';
import type { ConstructionLog, LogFilter, ProjectAt } from './constructionData';
import type { LogRow } from '../../charts/log/logData';
import type { StateNameAt } from '../../dashboard/construction';

const project = (patch: Partial<ConstructionProject>): ConstructionProject => ({
  id: '1', kind: 'building', tag: 'GER', building: 0, repair: false, state: 5, toState: null, province: null, routeProvinces: null, baseLevel: null,
  targetLevel: null, costRaw: '1080000000', createdHour: 90, firstRead: 100, runs: [{ firstHour: 100, lastHour: 147, factories: 10, remaining: 1, repairLevels: 0 }],
  earlierUnknown: false, built: [], unaccounted: [], possible: [], added: [], end: null, firstRepairTick: null, ...patch,
});

const MANIFEST: ConstructionLogManifestReply = {
  costScale: 100000, windows: [{ firstHour: 0, lastHour: 119 }, { firstHour: 120, lastHour: 159 }, { firstHour: 160, lastHour: 300 }],
  recorded: true, repairIcon: 'data:repair', warnings: [],
  types: [{ id: 20013, token: 'industrial_complex', name: 'Civilian Factory', icon: 'data:civ' }, { id: 12203, token: 'infrastructure', name: null, icon: null },
    { id: 19649, token: 'rail_way', name: 'Railway', icon: null }, { id: 7, token: null, name: null, icon: null }],
};
const STATES = [{ id: 5, name: 'Ruhr' }, { id: 6, name: null }, { id: 40, name: 'Moscow' }, { id: 41, name: 'Smolensk' }];
const PROJECTS = [
  // Built at 144, read gone at 148.
  project({ built: [{ hour: 144, knownAt: 148, count: 1, repair: false }], end: { kind: 'finished', hour: 144, knownAt: 148, unknown: null } }),
  // A repair of state 6's infrastructure, still going, first read at 124.
  project({ id: '2', building: 1, repair: true, state: 6, createdHour: 110, firstRead: 124, runs: [{ firstHour: 124, lastHour: 151, factories: 0, remaining: 0, repairLevels: 2 }] }),
  // Three levels: one built at 120; removed by 172 after a gap (not carried past 151).
  project({ id: '3', createdHour: 95, runs: [{ firstHour: 100, lastHour: 123, factories: 15, remaining: 3, repairLevels: 0 }, { firstHour: 124, lastHour: 151, factories: 12, remaining: 2, repairLevels: 0 }],
    built: [{ hour: 120, knownAt: 124, count: 1, repair: false }], end: { kind: 'removed', hour: null, knownAt: 172, unknown: null } }),
  // A railway of ITA from Moscow to Smolensk whose ending is unknown.
  project({ id: '4', kind: 'railway', tag: 'ITA', building: 2, state: 40, toState: 41, routeProvinces: 3, baseLevel: 1, targetLevel: 2, costRaw: '56000000',
    runs: [{ firstHour: 100, lastHour: 123, factories: 4, remaining: 2, repairLevels: 0 }, { firstHour: 124, lastHour: 147, factories: 4, remaining: 1, repairLevels: 0 }],
    built: [{ hour: null, knownAt: 124, count: 1, repair: false }],
    end: { kind: 'unknown', hour: null, knownAt: 148, unknown: 'notRecorded' } }),
];
/** A window of the manifest listing projects, with output runs and the output table's reads. */
const windowOf = (projects: ConstructionProject[], outputs: ProjectOutput[] = [], outputReads: number[] = [], index = 0,
  outputUnavailable: ConstructionLogReply['outputUnavailable'] = []): ConstructionLogReply => ({
  window: index, interval: MANIFEST.windows[index], outputReads, outputUnavailable, projects, outputs, warnings: [] });
/** The log as one window holding every project shows it: as the whole-branch reply did. */
const WHOLE: ConstructionLogManifestReply = { ...MANIFEST, windows: [{ firstHour: 0, lastHour: 300 }] };
const logOf = (projects: ConstructionProject[], outputs: ProjectOutput[] = [], outputReads: number[] = []) =>
  mergeWindows(WHOLE, new Map([[0, { ...windowOf(projects, outputs, outputReads), interval: WHOLE.windows[0] }]]), 0);
const LOG = logOf(PROJECTS);
const RUN: ConstructionOutputRun = { from: 100, to: 100, constructionRaw: '1', repairRaw: '0', reason: null, beforeObservation: false, absent: false };

const every: LogFilter = { types: [], projects: 'all', country: () => true, from: null };
const fixtureNames: StateNameAt = (_hour, state) => ({ state, name: STATES.find(name => name.id === state)?.name ?? null, missing: null });
const placeLabel = (_log: ConstructionLog, item: ConstructionProject) => datedPlaceLabel(item, 148, fixtureNames);
const describeRow = (log: ConstructionLog, row: LogRow<ProjectAt>, country: string) => datedDescribeRow(log, row, country, 148, fixtureNames);
const projectTooltip = (log: ConstructionLog, at: ProjectAt, hour: number, country: string) => datedProjectTooltip(log, at, hour, country, fixtureNames);

test('log state labels follow shown-hour renames, stored owners and named refusals without authored backfill', () => {
  const data = logOf([PROJECTS[0]]);
  const calls: number[] = [];
  const names: StateNameAt = (hour, state) => {
    calls.push(hour);
    return { state, name: hour === 100 ? 'Stored owner name' : hour === 124 ? 'Renamed state' : null,
      missing: hour === 130 ? 'States read failed' : hour === 148 ? 'Stored state name language differs from recorded language' : null };
  };
  const [row] = logRows(data, 124, every, 'change');
  assert.equal(datedPlaceLabel(row.item.project, 100, names), 'Stored owner name');
  assert.match(datedDescribeRow(data, row, 'Country', 124, names), /in Renamed state,/);
  for (const [hour, refusal] of [[130, 'States read failed'], [148, 'language differs']] as const) {
    const tip = datedProjectTooltip(data, row.item, hour, 'Country', names)[0];
    assert.match(tip.subtitle ?? '', /State 5/);
    assert.equal(tip.note, 'State name unavailable.');
    assert.doesNotMatch(tip.note, new RegExp(refusal));
  }
  assert.deepEqual(calls, [100, 124, 130, 148]);
});

test('construction selection accepts every recorded country', () => {
  assert.equal(definition.countries.max, Number.MAX_SAFE_INTEGER);
  const tags = Array.from({ length: 5000 }, (_, index) => `MOD${index}`);
  assert.deepEqual(resolveCountries({ kind: 'tags', tags }, [], undefined, 172, definition.countries.max).tags, tags);
});

test('the manifest and its windows are checked before use', () => {
  assert.equal(validateConstructionLogManifest(MANIFEST), MANIFEST);
  assert.throws(() => validateConstructionLogManifest({ ...MANIFEST, costScale: 0 }), /malformed/);
  assert.throws(() => validateConstructionLogManifest({ ...MANIFEST, windows: [...MANIFEST.windows].reverse() }), /malformed/);
  const reply = windowOf(PROJECTS, [{ id: '1', runs: [RUN] }], [100]);
  assert.equal(validateConstructionLogWindow(reply, MANIFEST.types.length), reply);
  const bad = (patch: Partial<ConstructionLogReply>) => assert.throws(() => validateConstructionLogWindow({ ...reply, ...patch }, MANIFEST.types.length), /malformed/);
  bad({ projects: [project({ building: 9 })] });
  bad({ projects: [project({ runs: [{ firstHour: 101, lastHour: 147, factories: 1, remaining: 1, repairLevels: 0 }] })] });
  bad({ projects: [project({ end: { kind: 'finished', hour: 144, knownAt: 100, unknown: null } })] });
  bad({ projects: [project({ end: { kind: 'gone' as 'finished', hour: null, knownAt: 148, unknown: null } })] });
  bad({ projects: [project({ earlierUnknown: undefined })] });
  bad({ projects: [project({ possible: undefined })] });
  bad({ projects: [project({ possible: [{ hour: null, knownAt: 99, count: 1, repair: false }] })] });
  bad({ projects: [project({ firstRepairTick: 99 })] });
  bad({ outputReads: [124, 100] });
  bad({ outputUnavailable: [[124, 'readFailed']] });
  bad({ outputs: [{ id: '1', runs: [{ ...RUN, absent: true }] }] });
  bad({ outputs: [{ id: '1', runs: [RUN, RUN] }] });
});

test('extra possible completions widen both counts only from their read, including the row and tooltip', () => {
  const uncertain = project({ runs: [
    { firstHour: 100, lastHour: 147, factories: 15, remaining: 2, repairLevels: 0 },
    { firstHour: 148, lastHour: 171, factories: 15, remaining: 1, repairLevels: 0 },
  ], built: [{ hour: null, knownAt: 148, count: 1, repair: false }],
  possible: [{ hour: null, knownAt: 148, count: 1, repair: false }] });
  assert.equal(builtLabel(projectAt(uncertain, 147)!), '0 of 2');
  const at = projectAt(uncertain, 148)!;
  assert.deepEqual([at.built, at.possible, at.levels, at.added, levelsLabel(at)], [1, 1, 2, 0, '1–2 of 2–3']);
  for (const tip of projectTooltip(LOG, at, 148, 'Germany')) {
    assert.equal(tip.rows.find(item => item.label === 'Levels built')?.value, '1–2 of 2–3');
    assert.equal(tip.rows.find(item => item.label === 'Added while under way')?.value, '0–1 level');
    assert.match(tip.note ?? '', /Completion count uncertain/);
  }
  const [row] = logRows(logOf([uncertain]), 148, every, 'change');
  assert.deepEqual([row.hour, row.by, at.stage], [148, true, 2]);
  assert.match(describeRow(LOG, row, 'Germany'), /Civilian Factory 1–2 of 2–3 in Ruhr, building, by /);
  const ended = project({ ...uncertain, built: [...uncertain.built, { hour: 168, knownAt: 172, count: 1, repair: false }],
    end: { kind: 'finished', hour: 168, knownAt: 172, unknown: null } });
  assert.equal(levelsLabel(projectAt(ended, 172)!), '2–3 of 2–3');
});

test('possible completions combine with unaccounted levels and earlier unknown work', () => {
  const uncertain = project({ earlierUnknown: true, runs: [
    { firstHour: 100, lastHour: 123, factories: 15, remaining: 3, repairLevels: 0 },
    { firstHour: 124, lastHour: 147, factories: 15, remaining: 2, repairLevels: 0 },
    { firstHour: 148, lastHour: 171, factories: 15, remaining: 2, repairLevels: 0 },
  ], unaccounted: [{ hour: null, knownAt: 124, count: 1, repair: false }],
  built: [{ hour: null, knownAt: 148, count: 1, repair: false }],
  added: [{ hour: null, knownAt: 148, count: 1, repair: false }],
  possible: [{ hour: null, knownAt: 148, count: 1, repair: false }] });
  assert.equal(levelsLabel(projectAt(uncertain, 147)!), '0–1+ of 3+');
  const at = projectAt(uncertain, 148)!;
  assert.equal(levelsLabel(at), '1–3+ of 4–5+');
  for (const tip of projectTooltip(LOG, at, 148, 'Germany')) {
    assert.equal(tip.rows.find(item => item.label === 'Levels built')?.value, '1–3+ of 4–5+');
    assert.equal(tip.rows.find(item => item.label === 'Added while under way')?.value, '1–2 levels');
    assert.equal(tip.rows.find(item => item.label === 'Built or dropped, not recorded')?.value, '1 level');
  }
  const [row] = logRows(logOf([uncertain]), 148, every, 'change');
  assert.match(describeRow(LOG, row, 'Germany'), /1–3\+ of 4–5\+.*earlier progress unknown/);
});

test('possible work alone changes a merged run without claiming exact work', () => {
  const uncertain = project({ possible: [{ hour: null, knownAt: 124, count: 1, repair: false }] });
  assert.equal(projectAt(uncertain, 123)!.possible, 0);
  const at = projectAt(uncertain, 124)!;
  assert.deepEqual([at.built, at.added, at.levels, at.changed, at.changedBy, at.stage], [0, 0, 1, 124, true, 1]);
  assert.equal(levelsLabel(at), '0–1 of 1–2');
});

test('work before the first read stays unknown, including on a single level', () => {
  // Created at 110 with two levels; one built at 120 before the first read at 124.
  const opening = project({ createdHour: 110, firstRead: 124, earlierUnknown: true,
    runs: [{ firstHour: 124, lastHour: 147, factories: 10, remaining: 1, repairLevels: 0 }],
    built: [{ hour: 144, knownAt: 148, count: 1, repair: false }], end: { kind: 'finished', hour: 144, knownAt: 148, unknown: null } });
  assert.equal(projectAt(opening, 123), null);
  assert.equal(builtLabel(projectAt(opening, 124)!), '0+ of 1+');
  const at = projectAt(opening, 148)!;
  assert.equal(at.earlierUnknown, true);
  assert.equal(builtLabel(at), '1+ of 1+');
  assert.equal(levelsLabel(at), '1+ of 1+');
  const [row] = logRows(logOf([opening]), 148, every, 'change');
  assert.match(describeRow(LOG, row, 'Germany'), /earlier progress unknown/);
  for (const tip of projectTooltip(LOG, at, 148, 'Germany')) {
    assert.equal(tip.rows.find(item => item.label === 'Levels built')?.value, '1+ of 1+');
    assert.match(tip.note ?? '', /Earlier progress unknown/);
  }
  const unseen = project({ earlierUnknown: true,
    runs: [{ firstHour: 100, lastHour: 123, factories: 10, remaining: 2, repairLevels: 0 },
      { firstHour: 124, lastHour: 147, factories: 10, remaining: 1, repairLevels: 0 }],
    unaccounted: [{ hour: null, knownAt: 124, count: 1, repair: false }] });
  assert.equal(builtLabel(projectAt(unseen, 124)!), '0–1+ of 2+');
  assert.equal(levelsLabel(projectAt(unseen, 124)!), '0–1+ of 2+');
  assert.equal(builtLabel(projectAt(project({}), 100)!), '0 of 1');
  assert.equal(levelsLabel(projectAt(project({}), 100)!), null);
});

test('a single uncertain level stays visible on the row and in its tooltip', () => {
  const uncertain = project({
    runs: [{ firstHour: 100, lastHour: 123, factories: 10, remaining: 1, repairLevels: 0 }],
    unaccounted: [{ hour: null, knownAt: 124, count: 1, repair: false }],
    end: { kind: 'unknown', hour: null, knownAt: 124, unknown: 'notRecorded' },
  });
  const at = projectAt(uncertain, 124)!;
  assert.deepEqual([at.built, at.unknown, at.levels, levelsLabel(at)], [0, 1, 1, '0–1 of 1']);
  assert.equal(projectTooltip(LOG, at, 124, 'Germany')[0].rows.find(item => item.label === 'Levels built')?.value, '0–1 of 1');
  const [row] = logRows(logOf([uncertain]), 124, every, 'change');
  assert.match(describeRow(LOG, row, 'Germany'), /0–1 of 1/);
});

test('a merged run uses only reads reached by the hour for its totals', () => {
  const merged = project({ runs: [{ firstHour: 100, lastHour: 147, factories: 10, remaining: 1, repairLevels: 0 }],
    built: [], unaccounted: [], added: [{ hour: null, knownAt: 124, count: 1, repair: true }] });
  assert.equal(projectAt(merged, 100)!.levels, 1);
  assert.equal(projectAt(merged, 124)!.levels, 1);
});

test('a project shows from its first read, and its ending only from the read that shows it', () => {
  const [built] = PROJECTS;
  assert.equal(projectAt(built, 99), null);
  const before = projectAt(built, 146)!;
  // The factory rose at 144, but no read showed it before 148.
  assert.deepEqual([before.status, before.built, before.levels, before.changed, before.stage], ['building', 0, 1, 90, 0]);
  const after = projectAt(built, 148)!;
  assert.deepEqual([after.status, after.built, after.levels, after.changed, after.changedBy, after.stage], ['finished', 1, 1, 144, false, 2]);
  assert.equal(factoriesShown(after), 10);
  const removed = projectAt(PROJECTS[2], 160)!;
  // Between the gap and the next read nothing is carried.
  assert.deepEqual([removed.status, removed.built, removed.levels, factoriesShown(removed)], ['unread', 1, 3, null]);
  const gone = projectAt(PROJECTS[2], 172)!;
  assert.deepEqual([gone.status, gone.levels, gone.changed, gone.changedBy, factoriesShown(gone)], ['removed', 3, 172, true, 12]);
  assert.equal(levelsLabel(projectAt(PROJECTS[2], 130)!), '1 of 3');
  assert.equal(levelsLabel(gone), '×3');
  assert.equal(levelsLabel(after), null);
});

test('rows are newest first by their latest change, and enter again when they change', () => {
  const keys = (hour: number, filter = every, order: 'change' | 'start' = 'change') => logRows(LOG, hour, filter, order).map(row => `${row.key}@${row.hour}${row.by ? '?' : ''}`);
  assert.deepEqual(keys(100), ['3:0@95', '1:0@90', '4:0@90']);
  assert.deepEqual(keys(124), ['4:1@124?', '3:1@120', '2:0@110', '1:0@90', ]);
  assert.deepEqual(keys(172), ['3:2@172?', '4:2@148?', '1:2@144', '2:0@110']);
  assert.deepEqual(keys(172, every, 'start'), ['2@110', '3@95', '1@90', '4@90']);
  // Filters: types, new or repair, countries at the first read, and a date range.
  assert.deepEqual(keys(172, { ...every, types: ['infrastructure'] }), ['2:0@110']);
  assert.deepEqual(keys(172, { ...every, projects: 'repair' }), ['2:0@110']);
  assert.deepEqual(keys(172, { ...every, projects: 'new' }), ['3:2@172?', '4:2@148?', '1:2@144']);
  assert.deepEqual(keys(172, { ...every, country: (tag, hour) => tag === 'ITA' && hour === 100 }), ['4:2@148?']);
  assert.deepEqual(keys(172, { ...every, from: 145 }), ['3:2@172?', '4:2@148?']);
});

test('a gap or failed queue read leaves status and pending dates unread until the next read', () => {
  for (const nextRead of [148, 172]) {
    for (const [remaining, repairLevels] of [[2, 0], [0, 1], [1, 1]]) {
      const item = project({ runs: [
        { firstHour: 100, lastHour: 123, factories: 10, remaining, repairLevels },
        { firstHour: nextRead, lastHour: nextRead + 23, factories: 12, remaining, repairLevels },
      ] });
      const log = logOf([item]);
      assert.equal(projectAt(item, 123)!.status, 'building');
      for (const hour of [124, nextRead - 1]) {
        const at = projectAt(item, hour)!;
        assert.deepEqual([at.status, statusLabel(at), at.repairing, factoriesShown(at)], ['unread', 'Not recorded', false, null]);
        const [row] = logRows(log, hour, every, 'change');
        assert.match(describeRow(log, row, 'Germany'), /, not recorded, /);
        for (const tip of projectTooltip(log, at, hour, 'Germany')) {
          const dates = tip.rows.filter(row => ['Repaired', 'Built', 'Finished'].includes(row.label));
          assert.equal(dates.length, remaining && repairLevels ? 2 : 1);
          assert.ok(dates.every(row => row.value === 'Not recorded' && row.tone === 'muted'));
          assert.equal(tip.rows.find(row => row.label === 'Factories')?.value, 'Factories unknown');
        }
      }
      const resumed = projectAt(item, nextRead)!;
      assert.deepEqual([resumed.status, resumed.repairing, factoriesShown(resumed)], ['building', repairLevels > 0, 12]);
      const ended = projectAt({ ...item, runs: item.runs.slice(0, 1),
        end: { kind: 'removed', hour: null, knownAt: nextRead, unknown: null } }, nextRead)!;
      assert.deepEqual([ended.status, ended.repairing, factoriesShown(ended)], ['removed', false, 10]);
    }
  }
  const known = projectAt(PROJECTS[2], 160)!;
  assert.equal(levelsLabel(known), '1 of 3');
});

test('a row says what, where and how it stands in words', () => {
  const at = (index: number, hour: number) => projectAt(PROJECTS[index], hour)!;
  assert.deepEqual([statusLabel(at(0, 148)), statusLabel(at(1, 130)), statusLabel(at(2, 172)), statusLabel(at(3, 148))], ['Built', 'Repairing', 'Removed', 'Left the queue']);
  assert.equal(placeLabel(LOG, PROJECTS[0]), 'Ruhr');
  assert.equal(placeLabel(LOG, PROJECTS[1]), 'State 6');
  assert.equal(placeLabel(LOG, PROJECTS[3]), 'Moscow to Smolensk');
  assert.equal(placeLabel(LOG, project({ state: null, province: 5007 })), 'province 5007');
  const [row] = logRows(LOG, 148, { ...every, types: ['industrial_complex'] }, 'change');
  assert.match(describeRow(LOG, row, 'Germany'), /^Germany, Civilian Factory in Ruhr, built, /);
});

test('the tooltip dates its read, names its levels and cost, and says why an ending is unknown', () => {
  const [full, compact] = projectTooltip(LOG, projectAt(PROJECTS[3], 150)!, 150, 'Italy');
  assert.equal(full.title, 'Railway');
  assert.equal(full.subtitle, 'Italy · Moscow to Smolensk');
  const value = (label: string) => full.rows.find(row => row.label === label)?.value;
  assert.equal(value('Segments built'), '1 of 2');
  assert.equal(value('Factories'), '4');
  assert.equal(value('Cost'), '560 for the route');
  assert.equal(value('Railway level'), '1 → 2');
  assert.match(value('Left the queue') ?? '', /^by /);
  assert.match(full.note ?? '', /Completion unknown/);
  assert.doesNotMatch(full.note ?? '', /read once a day/, 'tooltips carry no read cadence note');
  assert.ok(compact.rows.length < full.rows.length);
  const repair = projectTooltip(LOG, projectAt(PROJECTS[1], 124)!, 124, 'Germany')[0];
  assert.equal(repair.rows.find(row => row.label === 'Levels repaired')?.value, '0 of 2');
  assert.equal(repair.subtitle, 'Germany · State 6 · repair');
  assert.equal(repair.rows.find(row => row.label === 'Cost')?.value, '10,800 per level');
  assert.equal(repair.note, undefined);
});

test('the building types offered are the recording’s own, by name', () => {
  assert.deepEqual(buildingChoices(MANIFEST).map(choice => choice.label), ['Building name unavailable', 'Civilian Factory', 'Railway']);
  assert.deepEqual(buildingChoices(undefined), []);
});

test('a level lost while the building was not recorded stays counted, as a range, never erased', () => {
  // Two levels at 100; at 124 one is left, and whether the other was built is not known.
  const unrecorded = project({ id: '9', runs: [{ firstHour: 100, lastHour: 123, factories: 15, remaining: 2, repairLevels: 0 },
    { firstHour: 124, lastHour: 147, factories: 15, remaining: 1, repairLevels: 0 }], unaccounted: [{ hour: null, knownAt: 124, count: 1, repair: false }],
  built: [{ hour: 144, knownAt: 148, count: 1, repair: false }], end: { kind: 'finished', hour: 144, knownAt: 148, unknown: null } });
  const at = (hour: number) => projectAt(unrecorded, hour)!;
  assert.deepEqual([at(130).built, at(130).unknown, at(130).levels, levelsLabel(at(130))], [0, 1, 2, '0–1 of 2']);
  // Finished, its last level confirmed: the row keeps the range the tooltip gives.
  assert.deepEqual([at(148).status, at(148).built, at(148).unknown, at(148).levels, levelsLabel(at(148))], ['finished', 1, 1, 2, '1–2 of 2']);
  assert.equal(projectTooltip(LOG, at(148), 148, 'Germany')[0].rows.find(item => item.label === 'Levels built')?.value, levelsLabel(at(148)));
  assert.equal(describeRow(LOG, logRows(logOf([unrecorded]), 148, every, 'change')[0], 'Germany'),
    `Germany, Civilian Factory 1–2 of 2 in Ruhr, built, ${formatDateTime(144)}`);
  // Left the queue unknowably, its levels all unaccounted: a range too, never "×2".
  const left = project({ id: '11', runs: [{ firstHour: 100, lastHour: 123, factories: 15, remaining: 2, repairLevels: 0 }],
    unaccounted: [{ hour: null, knownAt: 124, count: 2, repair: false }], end: { kind: 'unknown', hour: null, knownAt: 124, unknown: 'notRecorded' } });
  assert.equal(levelsLabel(projectAt(left, 130)!), '0–2 of 2');
  // A new stage: the row re-enters, dated only by the read.
  const [row] = logRows(logOf([unrecorded]), 130, every, 'change');
  assert.deepEqual([row.key, row.hour, row.by, row.id], ['9:1', 124, true, '9']);
  const tip = projectTooltip(LOG, at(130), 130, 'Germany')[0];
  assert.equal(tip.rows.find(item => item.label === 'Built or dropped, not recorded')?.value, '1 level');
  assert.throws(() => validateConstructionLogWindow(windowOf([project({ unaccounted: [{ hour: null, knownAt: 1, count: 1, repair: false }] })]), MANIFEST.types.length), /malformed/);
});

test('levels built after the last read and before a removal are counted once', () => {
  // Three left at its last read; one built at 144, then removed by 148.
  const removed = project({ id: '10', runs: [{ firstHour: 100, lastHour: 147, factories: 15, remaining: 3, repairLevels: 0 }],
    built: [{ hour: 144, knownAt: 148, count: 1, repair: false }], end: { kind: 'removed', hour: null, knownAt: 148, unknown: null } });
  const at = projectAt(removed, 160)!;
  assert.deepEqual([at.status, at.built, at.levels, levelsLabel(at)], ['removed', 1, 3, '×3']);
  assert.equal(projectTooltip(LOG, at, 160, 'Germany')[0].rows.find(item => item.label === 'Levels built')?.value, '1 of 3');
  // Before the removal was read: three to build, none built yet.
  assert.deepEqual([projectAt(removed, 140)!.built, projectAt(removed, 140)!.levels], [0, 3]);
});

test('of changes one read confirms, the latest dates the row', () => {
  // The read at 124 failed; read 148 confirms levels risen at 120 and 144.
  const both = project({ id: '12', runs: [{ firstHour: 100, lastHour: 147, factories: 15, remaining: 2, repairLevels: 0 }],
    built: [{ hour: 120, knownAt: 148, count: 1, repair: false }, { hour: 144, knownAt: 148, count: 1, repair: false }], end: { kind: 'finished', hour: 144, knownAt: 148, unknown: null } });
  const building = project({ ...both, id: '18', end: null, runs: [{ firstHour: 100, lastHour: 147, factories: 15, remaining: 2, repairLevels: 0 },
    { firstHour: 148, lastHour: 160, factories: 15, remaining: 1, repairLevels: 0 }], built: [{ hour: 144, knownAt: 148, count: 1, repair: false }, { hour: 120, knownAt: 148, count: 1, repair: false }] });
  assert.deepEqual([projectAt(building, 150)!.changed, projectAt(building, 150)!.changedBy], [144, false]);
  // A range from 130 keeps it.
  const log = logOf([both, building]);
  assert.deepEqual(logRows(log, 150, { ...every, from: 130 }, 'change').map(row => `${row.id}@${row.hour}`), ['12@144', '18@144']);
});

test('levels a project may have built unseen before it left are a range, never zero', () => {
  // Two left at 100; hours 101-123 missing; gone at 124, ending unknown.
  const gone = project({ id: '13', runs: [{ firstHour: 100, lastHour: 100, factories: 15, remaining: 2, repairLevels: 0 }],
    unaccounted: [{ hour: null, knownAt: 124, count: 2, repair: false }], end: { kind: 'unknown', hour: null, knownAt: 124, unknown: 'notRecorded' } });
  const at = projectAt(gone, 130)!;
  assert.deepEqual([at.status, at.built, at.unknown, at.levels], ['ended', 0, 2, 2]);
  assert.equal(builtLabel(at), '0–2 of 2');
  assert.equal(projectTooltip(LOG, at, 130, 'Germany')[0].rows.find(row => row.label === 'Levels built')?.value, '0–2 of 2');
  assert.equal(builtLabel(projectAt(PROJECTS[0], 148)!), '1 of 1');
});

test('the Jump to menu keeps the latest change whatever the order', () => {
  const at = projectAt(PROJECTS[2], 130)!;
  // Started 95, a level at 120; not yet ended at 130.
  assert.deepEqual(jumpHours(at, 130), [95, 120]);
  const [row] = logRows(logOf([PROJECTS[2]]), 130, every, 'start');
  assert.equal(row.hour, 95);
  assert.deepEqual(jumpHours(row.item, 130), [95, 120]);
  // Once its removal is read, the ending too.
  assert.deepEqual(jumpHours(projectAt(PROJECTS[2], 172)!, 172), [95, 172]);
  // A project with no change yet offers its start alone.
  assert.deepEqual(jumpHours(projectAt(PROJECTS[1], 124)!, 124), [110]);
});

test('a level built while another was added counts both: built and still to build', () => {
  // One left at 100; one added and one built by 124 (one still left); built again at 144, gone at 148.
  const masked = project({ id: '14', runs: [{ firstHour: 100, lastHour: 123, factories: 15, remaining: 1, repairLevels: 0 },
    { firstHour: 124, lastHour: 147, factories: 15, remaining: 1, repairLevels: 0 }],
  built: [{ hour: 120, knownAt: 124, count: 1, repair: false }, { hour: 144, knownAt: 148, count: 1, repair: false }],
  added: [{ hour: null, knownAt: 124, count: 1, repair: false }], end: { kind: 'finished', hour: 144, knownAt: 148, unknown: null } });
  const at = (hour: number) => projectAt(masked, hour)!;
  assert.deepEqual([at(110).built, at(110).levels, at(110).added], [0, 1, 0]);
  assert.deepEqual([at(130).built, at(130).levels, at(130).added, levelsLabel(at(130))], [1, 2, 1, '1 of 2']);
  assert.deepEqual([at(150).built, at(150).levels, levelsLabel(at(150))], [2, 2, '×2']);
  assert.equal(projectTooltip(LOG, at(130), 130, 'Germany')[0].rows.find(row => row.label === 'Added while under way')?.value, '1 level');
  // Added and built at once before it left: the ending's addition counts.
  const burst = project({ id: '15', runs: [{ firstHour: 100, lastHour: 123, factories: 15, remaining: 1, repairLevels: 0 }],
    built: [{ hour: 110, knownAt: 124, count: 1, repair: false }, { hour: 120, knownAt: 124, count: 1, repair: false }],
    added: [{ hour: null, knownAt: 124, count: 1, repair: false }], end: { kind: 'finished', hour: 120, knownAt: 124, unknown: null } });
  assert.equal(builtLabel(projectAt(burst, 130)!), '2 of 2');
  assert.throws(() => validateConstructionLogWindow(windowOf([project({ added: [{ hour: null, knownAt: 124, count: 1 } as never] })]), MANIFEST.types.length), /malformed/);
});

test('what a line does is classified per stretch: a repair, then new levels', () => {
  // First read repairing one level with one new level queued; repaired at 110; building from 124; built at 140.
  const both = project({ id: '16', repair: true, runs: [{ firstHour: 100, lastHour: 123, factories: 15, remaining: 1, repairLevels: 1 },
    { firstHour: 124, lastHour: 147, factories: 15, remaining: 1, repairLevels: 0 }],
  built: [{ hour: 110, knownAt: 124, count: 1, repair: true }, { hour: 140, knownAt: 148, count: 1, repair: false }],
  end: { kind: 'finished', hour: 140, knownAt: 148, unknown: null } });
  const at = (hour: number) => projectAt(both, hour)!;
  assert.deepEqual([at(110).kind, at(110).repairing, statusLabel(at(110)), at(110).levels], ['repairThenNew', true, 'Repairing', 2]);
  assert.deepEqual([at(130).kind, at(130).repairing, statusLabel(at(130)), builtLabel(at(130))], ['repairThenNew', false, 'Building, after repair', '1 of 2']);
  assert.deepEqual([statusLabel(at(150)), builtLabel(at(150)), kindLabel(at(150).kind)], ['Repaired and built', '2 of 2', 'Repair, then new']);
  assert.equal(projectTooltip(LOG, at(150), 150, 'Germany')[0].subtitle, 'Germany · Ruhr · repair, then new');
  // Its completions follow the stretches: repaired at 110, built at 140, never "Repaired" at the build's date.
  const dates = (hour: number) => projectTooltip(LOG, at(hour), hour, 'Germany')[0].rows
    .filter(row => ['Repaired', 'Built', 'Finished'].includes(row.label)).map(row => `${row.label}: ${row.value}`);
  assert.deepEqual(dates(150), [`Repaired: ${formatDateTime(110)}`, `Built: ${formatDateTime(140)}`]);
  assert.deepEqual(dates(130), [`Repaired: ${formatDateTime(110)}`, 'Built: Not yet']);
  assert.deepEqual(dates(110), ['Repaired: Not yet', 'Built: Not yet']);
  // A plain repair and a plain new building keep their one completion row.
  assert.deepEqual(projectTooltip(LOG, projectAt(PROJECTS[0], 150)!, 150, 'Germany')[0].rows.filter(row => row.label === 'Finished').map(row => row.value),
    [formatDateTime(144)]);
  // Both filters show it; a plain repair only under repairs, a new building only under new.
  const log = logOf([both, PROJECTS[0], PROJECTS[1]]);
  const ids = (projects: 'new' | 'repair') => logRows(log, 150, { ...every, projects }, 'start').map(row => row.id).sort();
  assert.deepEqual(ids('repair'), ['16', '2']);
  assert.deepEqual(ids('new'), ['1', '16']);
  // A line first read building is new, however its flag was set.
  assert.equal(projectAt(project({ repair: true }), 110)!.kind, 'new');
  // Damaged and repaired between two reads that both show nothing to repair: the
  // repair and its added work count, and it shows under repairs from that read.
  const between = project({ id: '17', runs: [{ firstHour: 100, lastHour: 147, factories: 15, remaining: 1, repairLevels: 0 }],
    built: [{ hour: 120, knownAt: 124, count: 1, repair: true }], added: [{ hour: null, knownAt: 124, count: 1, repair: true }] });
  const inBetween = (hour: number) => projectAt(between, hour)!;
  assert.deepEqual([inBetween(110).kind, inBetween(130).kind, builtLabel(inBetween(130))], ['new', 'repairThenNew', '1 of 2']);
  assert.deepEqual(logRows(logOf([between]), 130, { ...every, projects: 'repair' }, 'start').map(row => row.id), ['17']);
});

test('the log keeps exact signed output and distinguishes natural repair from factory IC', () => {
  const p = project({ firstRepairTick: 124 });
  const log = logOf([p], [{ id: '1', runs: [{ ...RUN, constructionRaw: '9007199254741499', repairRaw: '-1', beforeObservation: true }] }], [100]);
  assert.equal(projectAt(p, 100)!.kind, 'new');
  const at = projectAt(p, 124)!;
  assert.equal(at.kind, 'repairThenNew');
  const tip = projectTooltip(log, at, 124, 'Synthetic country')[0];
  assert.equal(tip.rows?.find(row => row.label === 'Construction output')?.value, '90,071,992,547.41499');
  assert.equal(tip.rows?.find(row => row.label === 'Repair output')?.value, '−0.00001');
  assert.equal(tip.rows?.find(row => row.label === 'Output read')?.value, undefined);
  assert.match(tip.note ?? '', /Earlier progress unknown/);
});

// The output table was read at 100, 124, 148 and 172, and was unavailable at 172.
// Project 1's ended row was read at 148; project 3's last row at 124, then its
// final output was missing.
const exact = (from: number, to: number, constructionRaw: string): ConstructionOutputRun => ({ ...RUN, from, to, constructionRaw });
const ABSENT: ConstructionOutputRun = { ...RUN, from: 148, to: 172, constructionRaw: null, repairRaw: null, absent: true };
const RUNS = { 1: [exact(100, 100, '10'), exact(124, 148, '20')], 3: [exact(100, 124, '5'), ABSENT] };
const UNAVAILABLE: ConstructionLogReply['outputUnavailable'] = [[172, 'readFailed']];
// As the product cuts them: each project in the windows of its rows' dates, with the
// runs read in each window and every listed project's last run.
const WINDOWS = new Map([
  [0, windowOf(PROJECTS, [{ id: '1', runs: RUNS[1] }, { id: '3', runs: RUNS[3] }], [100], 0)],
  [1, windowOf([PROJECTS[0], PROJECTS[2], PROJECTS[3]], [{ id: '1', runs: RUNS[1] }, { id: '3', runs: RUNS[3] }], [100, 124, 148], 1)],
  [2, windowOf([PROJECTS[2]], [{ id: '3', runs: [ABSENT] }], [148, 172], 2, UNAVAILABLE)],
]);

test('the windows held show the whole-branch log, back to the oldest held unbroken', () => {
  const whole = mergeWindows(WHOLE, new Map([[0, { ...windowOf(PROJECTS, Object.entries(RUNS).map(([id, runs]) => ({ id, runs })),
    [100, 124, 148, 172], 0, UNAVAILABLE), interval: WHOLE.windows[0] }]]), 0);
  // What each read showed, as the whole-branch reply listed every read.
  const reads: Record<string, [number, string | null, SpendingGap | null][]> = {
    1: [[100, '10', null], [124, '20', null], [148, '20', null]],
    3: [[100, '5', null], [124, '5', null], [148, null, 'finalOutputMissing'], [172, null, 'readFailed']],
  };
  for (let hour = 100; hour <= 200; hour++) {
    const current = hour < 120 ? 0 : hour < 160 ? 1 : 2;
    const rows = logRows(whole, hour, every, 'change');
    for (let first = 0; first <= current; first++) {
      const held = mergeWindows(MANIFEST, new Map([...WINDOWS].filter(([index]) => index >= first && index <= current)), current);
      const shown = logRows(held, hour, every, 'change');
      assert.deepEqual(shown.map(row => row.key), rows.filter(row => row.hour >= MANIFEST.windows[first].firstHour || first === 0).map(row => row.key),
        `${hour} from window ${first}`);
      for (const row of shown) assert.deepEqual(projectTooltip(held, row.item, hour, 'Germany'), projectTooltip(whole, row.item, hour, 'Germany'));
    }
    for (const [id, read] of Object.entries(reads)) {
      const project = PROJECTS.find(item => item.id === id)!;
      const expected = read.filter(([knownAt]) => knownAt <= hour).at(-1) ?? null;
      // The hour's window and any other window listing the project.
      for (const member of [...WINDOWS.values()].filter(reply => reply.projects.some(listed => listed.id === id))) {
        const output = outputAt(mergeWindows(MANIFEST, new Map([[current, WINDOWS.get(current)!], [member.window, member]]), current), project, hour);
        assert.deepEqual(output && [output.knownAt, output.constructionRaw, output.reason], expected, `${id} at ${hour}`);
      }
    }
  }
});

test('the log holds the windows its drawn rows need, reaching back only when short', () => {
  // At first: the hour's window and the one before, and the next during playback.
  assert.deepEqual(constructionWindows(5, 2, true, null), [1, 2, 3]);
  assert.deepEqual(constructionWindows(5, 4, true, null), [3, 4]);
  // A reach asked at this window or the one before holds; after a far jump it starts afresh.
  assert.deepEqual(constructionWindows(5, 3, false, { at: 2, lower: 0 }), [0, 1, 2, 3]);
  assert.deepEqual(constructionWindows(5, 4, false, { at: 2, lower: 0 }), [3, 4]);
  const log = mergeWindows(MANIFEST, new Map([[1, WINDOWS.get(1)!], [2, WINDOWS.get(2)!]]), 2);
  const rows = logRows(log, 172, every, 'change');
  assert.deepEqual(rows.map(row => row.key), ['3:2', '4:2', '1:2']);
  // One drawn: the next row, read in window 1, is held.
  assert.equal(logReach(log, rows, rows.slice(0, 1), null), 1);
  // All drawn: short of as many below, it reaches back twice as far, unless a range starts in what it holds.
  assert.equal(logReach(log, rows, rows, null), 0);
  assert.equal(logReach(log, rows, rows, 130), 1);
});
