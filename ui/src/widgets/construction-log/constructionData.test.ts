import assert from 'node:assert/strict';
import test from 'node:test';
import { deepFreeze } from '../../test-support/frozen.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { resolveCountries } from '../../dashboard/groups.ts';
import { definition } from './definition.ts';
import { builtLabel, constructionPlayback, heldThrough, liveKey, liveProjectTexts, projectText, shapeChanges, constructionWindows, describeRow as datedDescribeRow, factoriesShown, inQueue, joinProject, jumpHours, kindLabel, levelsLabel, logReach, logRows, mergeWindows, outputAt, placeTitle, projectAt, projectTooltip as datedProjectTooltip, rowCountryRequests, rowFactories, statusLabel, statusTone, validateConstructionLogManifest, validateConstructionLogWindow, windowAt } from './constructionData.ts';
import type { ConstructionLog, LogFilter, ProjectAt } from './constructionData';
import type { LogRow } from '../../charts/log/logData';
import { MAX_NAME_HOURS } from '../../dashboard/countryFacts.ts';
import type { ConstructionLogManifestReply, ConstructionLogReply, ConstructionOutputRun, ConstructionProject } from '../../generated/dashboard';

const project = (patch: Partial<ConstructionProject>): ConstructionProject => ({
  id: '1', kind: 'building', source: 'census', queueOrdinal: 0, censusRead: patch.firstRead ?? 100, outputOrigin: null, tag: 'GER', building: 20013, repair: false, state: 5, toState: null, province: null, routeProvinces: null, baseLevel: null,
  targetLevel: null, costRaw: '1080000000', createdHour: 90, firstRead: 100, runs: [{ firstHour: 100, lastHour: 147, factories: 10, remaining: 1, repairLevels: 0 }],
  earlierUnknown: false, built: [], unaccounted: [], possible: [], added: [], end: null, firstRepairTick: null, outputs: [], ...patch,
});

const MANIFEST: ConstructionLogManifestReply = {
  costScale: 100000, windows: [{ firstHour: 0, lastHour: 119 }, { firstHour: 120, lastHour: 159 }, { firstHour: 160, lastHour: 300 }],
  recorded: true, repairIcon: 'data:repair', warnings: [], missing: [],
  types: [{ id: 20013, token: 'industrial_complex', name: 'Civilian Factory', icon: 'data:civ' }, { id: 12203, token: 'infrastructure', name: null, icon: null },
    { id: 19649, token: 'rail_way', name: 'Railway', icon: null }, { id: 7, token: null, name: null, icon: null }],
};
const STATES = [{ id: 5, name: 'Ruhr' }, { id: 6, name: null }, { id: 40, name: 'Moscow' }, { id: 41, name: 'Smolensk' }];
const PROJECTS = [
  project({ built: [{ hour: 144, knownAt: 148, count: 1, repair: false, carried: false }], end: { kind: 'finished', hour: 144, knownAt: 148, unknown: null } }),
  project({ id: '2', building: 12203, repair: true, state: 6, createdHour: 110, firstRead: 124, runs: [{ firstHour: 124, lastHour: 151, factories: 0, remaining: 0, repairLevels: 2 }] }),
  project({ id: '3', createdHour: 95, runs: [{ firstHour: 100, lastHour: 123, factories: 15, remaining: 3, repairLevels: 0 }, { firstHour: 124, lastHour: 151, factories: 12, remaining: 2, repairLevels: 0 }],
    built: [{ hour: 120, knownAt: 124, count: 1, repair: false, carried: false }], end: { kind: 'removed', hour: null, knownAt: 172, unknown: null } }),
  project({ id: '4', kind: 'railway', tag: 'ITA', building: 19649, state: 40, toState: 41, routeProvinces: 3, baseLevel: 1, targetLevel: 2, costRaw: '56000000',
    runs: [{ firstHour: 100, lastHour: 123, factories: 4, remaining: 2, repairLevels: 0 }, { firstHour: 124, lastHour: 147, factories: 4, remaining: 1, repairLevels: 0 }],
    built: [{ hour: null, knownAt: 124, count: 1, repair: false, carried: false }],
    end: { kind: 'unknown', hour: null, knownAt: 148, unknown: 'notRecorded' } }),
];
const windowOf = (projects: ConstructionProject[], index = 0, states = STATES): ConstructionLogReply => ({
  window: index, interval: MANIFEST.windows[index], projects, states, warnings: [] });
const WHOLE: ConstructionLogManifestReply = { ...MANIFEST, windows: [{ firstHour: 0, lastHour: 300 }] };
const logOf = (projects: ConstructionProject[], states = STATES) =>
  mergeWindows(WHOLE, new Map([[0, { ...windowOf(projects, 0, states), interval: WHOLE.windows[0] }]]), 0);
const LOG = logOf(PROJECTS);
const RUN: ConstructionOutputRun = { from: 100, to: 100, constructionRaw: '1', repairRaw: '0', reason: null, beforeObservation: false };
const every: LogFilter = { types: [], projects: 'all', country: () => true, from: null };
const SPLIT: ConstructionProject[] = [
  { ...PROJECTS[2], runs: [{ firstHour: 100, lastHour: 119, factories: 15, remaining: 3, repairLevels: 0 }], built: [], end: null },
  { ...PROJECTS[2], runs: [{ firstHour: 120, lastHour: 123, factories: 15, remaining: 3, repairLevels: 0 }, { firstHour: 124, lastHour: 151, factories: 12, remaining: 2, repairLevels: 0 }], end: null },
  { ...PROJECTS[2], runs: [], built: [{ hour: null, knownAt: 160, count: 1, repair: false, carried: true }] },
];

test('construction playback retains rows between native changes and matches uncached rows across seeks', () => {
  const log = deepFreeze(logOf(structuredClone(PROJECTS))), filter = deepFreeze({ ...every, types: [] });
  for (const order of ['change', 'start'] as const) {
    const playback = constructionPlayback(log, filter, order);
    for (const hour of [99, 100, 119, 124, 130, 147, 148, 151, 152, 172, 180, 125, 100])
      assert.deepEqual(playback.at(hour), logRows(log, hour, filter, order));
    const settled = playback.at(125);
    assert.equal(playback.at(126), settled);
    assert.equal(playback.at(130), settled);
    assert.notEqual(playback.at(148), settled);
  }
});
const describeRow = (log: ConstructionLog, row: LogRow<ProjectAt>, country: string) => datedDescribeRow(log, row, country);
const projectTooltip = (log: ConstructionLog, at: ProjectAt, hour: number, country: string) => datedProjectTooltip(log, at, hour, country);

test('output-only projects show their evidence without inventing census fields', () => {
  const output = project({ source: 'outputAlone', queueOrdinal: null, censusRead: null,
    outputOrigin: { tag: 'GER', building: 20013, state: 5, toState: null, province: null },
    repair: null, costRaw: null, createdHour: null, firstRead: 124, runs: [], earlierUnknown: true,
    end: { kind: 'finished', hour: 120, knownAt: 124, unknown: null } });
  validateConstructionLogWindow(windowOf([output]));
  const at = projectAt(output, 124)!;
  assert.equal(at.project.source, 'outputAlone');
  assert.equal(statusLabel(at), 'Finished');
  assert.equal(factoriesShown(at), null);
  assert.equal(levelsLabel(at), null);
  const rows = projectTooltip(logOf([output]), at, 124, 'Germany')[0].rows ?? [];
  assert.ok(rows.some(row => row.label === 'Source' && row.value === 'Known from output'));
  assert.ok(!rows.some(row => /Started|Factories|Cost/.test(row.label)));
  assert.equal(projectAt(output, 123), null);
  for (const projects of ['new', 'repair'] as const)
    assert.equal(logRows(logOf([output]), 124, { ...every, projects }, 'change').length, 0);
});

test('a later census changes the source without backfilling earlier playback', () => {
  const enriched = project({ source: 'census', queueOrdinal: 0, censusRead: 148, firstRead: 124, createdHour: 110,
    outputOrigin: { tag: 'GER', building: 20013, state: 5, toState: null, province: null },
    runs: [{ firstHour: 148, lastHour: 180, factories: 7, remaining: 1, repairLevels: 0 }] });
  validateConstructionLogWindow(windowOf([enriched]));
  const before = projectAt(enriched, 147)!;
  assert.equal(before.project.source, 'outputAlone');
  assert.equal(before.project.createdHour, null);
  assert.equal(before.project.costRaw, null);
  assert.equal(before.project.queueOrdinal, null);
  assert.equal(before.run, null);
  assert.equal(before.last, null);
  assert.equal(before.changed, 124);
  const after = projectAt(enriched, 148)!;
  assert.equal(after.project.source, 'census');
  assert.equal(after.project.firstRead, 124);
  assert.equal(after.project.createdHour, 110);
  assert.equal(after.run?.factories, 7);
  const log = logOf([enriched]);
  assert.equal(logRows(log, 147, every, 'start')[0].hour, 124);
  assert.equal(logRows(log, 148, every, 'start')[0].hour, 110);
});

test('construction selection accepts every recorded country', () => {
  assert.equal(definition.countries.max, Number.MAX_SAFE_INTEGER);
  const tags = Array.from({ length: 5000 }, (_, index) => `MOD${index}`);
  assert.deepEqual(resolveCountries({ kind: 'tags', tags }, [], undefined, 172, definition.countries.max).tags, tags);
});


test('every drawn row is named at its own first read, however many hours they span', () => {
  const rows = (count: number) => Array.from({ length: count }, (_, index) => {
    const at = projectAt(project({ id: String(index), tag: `MOD ${index % 3}`, firstRead: 1000 + index, createdHour: 900,
      runs: [{ firstHour: 1000 + index, lastHour: 5000, factories: 1, remaining: 1, repairLevels: 0 }] }), 5000)!;
    return { key: at.project.id, hour: 5000 - index, item: at };
  });
  const many = rowCountryRequests(rows(MAX_NAME_HOURS + 36));
  assert.equal(many.wanted.length, MAX_NAME_HOURS + 36);
  assert.equal(many.maxHours, MAX_NAME_HOURS + 36, 'the newest rows keep their names past the default hour limit');
  assert.deepEqual(many.wanted[0], { hour: 1000, tag: 'MOD 0' });
  const few = rowCountryRequests(rows(3));
  assert.deepEqual([few.wanted.length, few.maxHours], [3, MAX_NAME_HOURS]);
  assert.deepEqual(rowCountryRequests([]), { wanted: [], maxHours: MAX_NAME_HOURS });
});

test('extra possible completions widen both counts only from their read, including the row and tooltip', () => {
  const uncertain = project({ runs: [
    { firstHour: 100, lastHour: 147, factories: 15, remaining: 2, repairLevels: 0 },
    { firstHour: 148, lastHour: 171, factories: 15, remaining: 1, repairLevels: 0 },
  ], built: [{ hour: null, knownAt: 148, count: 1, repair: false, carried: false }],
  possible: [{ hour: null, knownAt: 148, count: 1, repair: false, carried: false }] });
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
  const ended = project({ ...uncertain, built: [...uncertain.built, { hour: 168, knownAt: 172, count: 1, repair: false, carried: false }],
    end: { kind: 'finished', hour: 168, knownAt: 172, unknown: null } });
  assert.equal(levelsLabel(projectAt(ended, 172)!), '2–3 of 2–3');
});

test('possible completions combine with unaccounted levels and earlier unknown work', () => {
  const uncertain = project({ earlierUnknown: true, runs: [
    { firstHour: 100, lastHour: 123, factories: 15, remaining: 3, repairLevels: 0 },
    { firstHour: 124, lastHour: 147, factories: 15, remaining: 2, repairLevels: 0 },
    { firstHour: 148, lastHour: 171, factories: 15, remaining: 2, repairLevels: 0 },
  ], unaccounted: [{ hour: null, knownAt: 124, count: 1, repair: false, carried: false }],
  built: [{ hour: null, knownAt: 148, count: 1, repair: false, carried: false }],
  added: [{ hour: null, knownAt: 148, count: 1, repair: false, carried: false }],
  possible: [{ hour: null, knownAt: 148, count: 1, repair: false, carried: false }] });
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
  const uncertain = project({ possible: [{ hour: null, knownAt: 124, count: 1, repair: false, carried: false }] });
  assert.equal(projectAt(uncertain, 123)!.possible, 0);
  const at = projectAt(uncertain, 124)!;
  assert.deepEqual([at.built, at.added, at.levels, at.changed, at.changedBy, at.stage], [0, 0, 1, 124, true, 1]);
  assert.equal(levelsLabel(at), '0–1 of 1–2');
});

test('work before the first read stays unknown, including on a single level', () => {
  const opening = project({ createdHour: 110, firstRead: 124, earlierUnknown: true,
    runs: [{ firstHour: 124, lastHour: 147, factories: 10, remaining: 1, repairLevels: 0 }],
    built: [{ hour: 144, knownAt: 148, count: 1, repair: false, carried: false }], end: { kind: 'finished', hour: 144, knownAt: 148, unknown: null } });
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
    unaccounted: [{ hour: null, knownAt: 124, count: 1, repair: false, carried: false }] });
  assert.equal(builtLabel(projectAt(unseen, 124)!), '0–1+ of 2+');
  assert.equal(levelsLabel(projectAt(unseen, 124)!), '0–1+ of 2+');
  assert.equal(builtLabel(projectAt(project({}), 100)!), '0 of 1');
  assert.equal(levelsLabel(projectAt(project({}), 100)!), null);
});

test('a single uncertain level stays visible on the row and in its tooltip', () => {
  const uncertain = project({
    runs: [{ firstHour: 100, lastHour: 123, factories: 10, remaining: 1, repairLevels: 0 }],
    unaccounted: [{ hour: null, knownAt: 124, count: 1, repair: false, carried: false }],
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
    built: [], unaccounted: [], added: [{ hour: null, knownAt: 124, count: 1, repair: true, carried: false }] });
  assert.equal(projectAt(merged, 100)!.levels, 1);
  assert.equal(projectAt(merged, 124)!.levels, 1);
});

test('a project shows from its first read, and its ending only from the read that shows it', () => {
  const [built] = PROJECTS;
  assert.equal(projectAt(built, 99), null);
  const before = projectAt(built, 146)!;
  assert.deepEqual([before.status, before.built, before.levels, before.changed, before.stage], ['building', 0, 1, 90, 0]);
  const after = projectAt(built, 148)!;
  assert.deepEqual([after.status, after.built, after.levels, after.changed, after.changedBy, after.stage], ['finished', 1, 1, 144, false, 2]);
  assert.equal(factoriesShown(after), 10);
  assert.deepEqual([rowFactories(before), rowFactories(after)], [{ factories: 10 }, null]);
  assert.deepEqual([statusTone(before), statusTone(after)], ['accent', 'positive']);
  const removed = projectAt(PROJECTS[2], 160)!;
  assert.deepEqual([removed.status, removed.built, removed.levels, factoriesShown(removed)], ['unread', 1, 3, null]);
  const gone = projectAt(PROJECTS[2], 172)!;
  assert.deepEqual([gone.status, gone.levels, gone.changed, gone.changedBy, factoriesShown(gone)], ['removed', 3, 172, true, 12]);
  assert.deepEqual([inQueue(removed), rowFactories(removed), statusTone(removed)], [true, { factories: null }, 'neutral']);
  assert.deepEqual([inQueue(gone), rowFactories(gone), statusTone(gone)], [false, null, 'neutral']);
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
  assert.deepEqual(keys(172, { ...every, types: ['infrastructure'] }), ['2:0@110']);
  assert.deepEqual(keys(172, { ...every, projects: 'repair' }), ['2:0@110']);
  assert.deepEqual(keys(172, { ...every, projects: 'new' }), ['3:2@172?', '4:2@148?', '1:2@144']);
  assert.deepEqual(keys(172, { ...every, country: (tag, hour) => tag === 'ITA' && hour === 100 }), ['4:2@148?']);
  assert.deepEqual(keys(172, { ...every, from: 145 }), ['3:2@172?', '4:2@148?']);
  assert.deepEqual(keys(172, { ...every, queue: true }), ['2:0@110']);
  assert.deepEqual(logRows(LOG, 148, { ...every, queue: true }, 'change').map(row => row.id), ['3', '2']);
  assert.deepEqual(logRows(LOG, 124, { ...every, queue: true }, 'start').map(row => row.id).sort(), ['1', '2', '3', '4']);
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
          assert.equal(tip.rows.find(row => row.label === 'Factories')?.value, 'Not recorded');
          assert.deepEqual(rowFactories(at), { factories: null });
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


test('the tooltip dates its read, names its levels and cost, and says why an ending is unknown', () => {
  const [full, compact] = projectTooltip(LOG, projectAt(PROJECTS[3], 150)!, 150, 'Italy');
  assert.equal(full.title, 'Railway');
  assert.equal(full.subtitle, 'Italy · Moscow to Smolensk');
  const value = (label: string) => full.rows.find(row => row.label === label)?.value;
  assert.equal(value('Segments built'), '1 of 2');
  assert.equal(value('Factories'), undefined);
  assert.equal(value('Factories at its last read'), '4');
  assert.equal(value('Cost'), undefined);
  assert.equal(value('Route cost'), '560');
  assert.equal(value('Railway level'), '1 → 2');
  assert.match(value('Left the queue') ?? '', /^by /);
  assert.match(full.note ?? '', /Completion unknown/);
  assert.doesNotMatch(full.note ?? '', /read once a day/, 'tooltips carry no read cadence note');
  assert.ok(compact.rows.length < full.rows.length);
  const repair = projectTooltip(LOG, projectAt(PROJECTS[1], 124)!, 124, 'Germany')[0];
  assert.equal(repair.rows.find(row => row.label === 'Levels repaired')?.value, '0 of 2');
  assert.equal(repair.subtitle, 'Germany · State 6 · repair');
  assert.equal(repair.rows.find(row => row.label === 'Cost per level')?.value, '10,800');
  assert.equal(repair.rows.find(row => row.label === 'Factories')?.value, '0');
  assert.equal(repair.note, 'State 6: no name in the recorded content');
});


test('a level lost while the building was not recorded stays counted, as a range, never erased', () => {
  const unrecorded = project({ id: '9', runs: [{ firstHour: 100, lastHour: 123, factories: 15, remaining: 2, repairLevels: 0 },
    { firstHour: 124, lastHour: 147, factories: 15, remaining: 1, repairLevels: 0 }], unaccounted: [{ hour: null, knownAt: 124, count: 1, repair: false, carried: false }],
  built: [{ hour: 144, knownAt: 148, count: 1, repair: false, carried: false }], end: { kind: 'finished', hour: 144, knownAt: 148, unknown: null } });
  const at = (hour: number) => projectAt(unrecorded, hour)!;
  assert.deepEqual([at(130).built, at(130).unknown, at(130).levels, levelsLabel(at(130))], [0, 1, 2, '0–1 of 2']);
  assert.deepEqual([at(148).status, at(148).built, at(148).unknown, at(148).levels, levelsLabel(at(148))], ['finished', 1, 1, 2, '1–2 of 2']);
  assert.equal(projectTooltip(LOG, at(148), 148, 'Germany')[0].rows.find(item => item.label === 'Levels built')?.value, levelsLabel(at(148)));
  assert.equal(describeRow(LOG, logRows(logOf([unrecorded]), 148, every, 'change')[0], 'Germany'),
    `Germany, Civilian Factory 1–2 of 2 in Ruhr, built, ${formatDateTime(144)}`);
  const left = project({ id: '11', runs: [{ firstHour: 100, lastHour: 123, factories: 15, remaining: 2, repairLevels: 0 }],
    unaccounted: [{ hour: null, knownAt: 124, count: 2, repair: false, carried: false }], end: { kind: 'unknown', hour: null, knownAt: 124, unknown: 'notRecorded' } });
  assert.equal(levelsLabel(projectAt(left, 130)!), '0–2 of 2');
  const [row] = logRows(logOf([unrecorded]), 130, every, 'change');
  assert.deepEqual([row.key, row.hour, row.by, row.id], ['9:1', 124, true, '9']);
  const tip = projectTooltip(LOG, at(130), 130, 'Germany')[0];
  assert.equal(tip.rows.find(item => item.label === 'Built or dropped, not recorded')?.value, '1 level');
});

test('levels built after the last read and before a removal are counted once', () => {
  const removed = project({ id: '10', runs: [{ firstHour: 100, lastHour: 147, factories: 15, remaining: 3, repairLevels: 0 }],
    built: [{ hour: 144, knownAt: 148, count: 1, repair: false, carried: false }], end: { kind: 'removed', hour: null, knownAt: 148, unknown: null } });
  const at = projectAt(removed, 160)!;
  assert.deepEqual([at.status, at.built, at.levels, levelsLabel(at)], ['removed', 1, 3, '×3']);
  assert.equal(projectTooltip(LOG, at, 160, 'Germany')[0].rows.find(item => item.label === 'Levels built')?.value, '1 of 3');
  assert.deepEqual([projectAt(removed, 140)!.built, projectAt(removed, 140)!.levels], [0, 3]);
});

test('of changes one read confirms, the latest dates the row', () => {
  const both = project({ id: '12', runs: [{ firstHour: 100, lastHour: 147, factories: 15, remaining: 2, repairLevels: 0 }],
    built: [{ hour: 120, knownAt: 148, count: 1, repair: false, carried: false }, { hour: 144, knownAt: 148, count: 1, repair: false, carried: false }], end: { kind: 'finished', hour: 144, knownAt: 148, unknown: null } });
  const building = project({ ...both, id: '18', end: null, runs: [{ firstHour: 100, lastHour: 147, factories: 15, remaining: 2, repairLevels: 0 },
    { firstHour: 148, lastHour: 160, factories: 15, remaining: 1, repairLevels: 0 }], built: [{ hour: 144, knownAt: 148, count: 1, repair: false, carried: false }, { hour: 120, knownAt: 148, count: 1, repair: false, carried: false }] });
  assert.deepEqual([projectAt(building, 150)!.changed, projectAt(building, 150)!.changedBy], [144, false]);
  const log = logOf([both, building]);
  assert.deepEqual(logRows(log, 150, { ...every, from: 130 }, 'change').map(row => `${row.id}@${row.hour}`), ['12@144', '18@144']);
});

test('levels a project may have built unseen before it left are a range, never zero', () => {
  const gone = project({ id: '13', runs: [{ firstHour: 100, lastHour: 100, factories: 15, remaining: 2, repairLevels: 0 }],
    unaccounted: [{ hour: null, knownAt: 124, count: 2, repair: false, carried: false }], end: { kind: 'unknown', hour: null, knownAt: 124, unknown: 'notRecorded' } });
  const at = projectAt(gone, 130)!;
  assert.deepEqual([at.status, at.built, at.unknown, at.levels], ['ended', 0, 2, 2]);
  assert.equal(builtLabel(at), '0–2 of 2');
  assert.equal(projectTooltip(LOG, at, 130, 'Germany')[0].rows.find(row => row.label === 'Levels built')?.value, '0–2 of 2');
  assert.equal(builtLabel(projectAt(PROJECTS[0], 148)!), '1 of 1');
});

test('the Jump to menu keeps the latest change whatever the order', () => {
  const at = projectAt(PROJECTS[2], 130)!;
  assert.deepEqual(jumpHours(at, 130), [95, 120]);
  const [row] = logRows(logOf([PROJECTS[2]]), 130, every, 'start');
  assert.equal(row.hour, 95);
  assert.deepEqual(jumpHours(row.item, 130), [95, 120]);
  assert.deepEqual(jumpHours(projectAt(PROJECTS[2], 172)!, 172), [95, 172]);
  assert.deepEqual(jumpHours(projectAt(PROJECTS[1], 124)!, 124), [110]);
});

test('a level built while another was added counts both: built and still to build', () => {
  const masked = project({ id: '14', runs: [{ firstHour: 100, lastHour: 123, factories: 15, remaining: 1, repairLevels: 0 },
    { firstHour: 124, lastHour: 147, factories: 15, remaining: 1, repairLevels: 0 }],
  built: [{ hour: 120, knownAt: 124, count: 1, repair: false, carried: false }, { hour: 144, knownAt: 148, count: 1, repair: false, carried: false }],
  added: [{ hour: null, knownAt: 124, count: 1, repair: false, carried: false }], end: { kind: 'finished', hour: 144, knownAt: 148, unknown: null } });
  const at = (hour: number) => projectAt(masked, hour)!;
  assert.deepEqual([at(110).built, at(110).levels, at(110).added], [0, 1, 0]);
  assert.deepEqual([at(130).built, at(130).levels, at(130).added, levelsLabel(at(130))], [1, 2, 1, '1 of 2']);
  assert.deepEqual([at(150).built, at(150).levels, levelsLabel(at(150))], [2, 2, '×2']);
  assert.equal(projectTooltip(LOG, at(130), 130, 'Germany')[0].rows.find(row => row.label === 'Added while under way')?.value, '1 level');
  const burst = project({ id: '15', runs: [{ firstHour: 100, lastHour: 123, factories: 15, remaining: 1, repairLevels: 0 }],
    built: [{ hour: 110, knownAt: 124, count: 1, repair: false, carried: false }, { hour: 120, knownAt: 124, count: 1, repair: false, carried: false }],
    added: [{ hour: null, knownAt: 124, count: 1, repair: false, carried: false }], end: { kind: 'finished', hour: 120, knownAt: 124, unknown: null } });
  assert.equal(builtLabel(projectAt(burst, 130)!), '2 of 2');
  assert.throws(() => validateConstructionLogWindow(windowOf([project({ added: [{ hour: null, knownAt: 124, count: 1 } as never] })])), /malformed/);
});

test('what a line does is classified per stretch: a repair, then new levels', () => {
  const both = project({ id: '16', repair: true, runs: [{ firstHour: 100, lastHour: 123, factories: 15, remaining: 1, repairLevels: 1 },
    { firstHour: 124, lastHour: 147, factories: 15, remaining: 1, repairLevels: 0 }],
  built: [{ hour: 110, knownAt: 124, count: 1, repair: true, carried: false }, { hour: 140, knownAt: 148, count: 1, repair: false, carried: false }],
  end: { kind: 'finished', hour: 140, knownAt: 148, unknown: null } });
  const at = (hour: number) => projectAt(both, hour)!;
  assert.deepEqual([at(110).kind, at(110).repairing, statusLabel(at(110)), at(110).levels], ['repairThenNew', true, 'Repairing', 2]);
  assert.deepEqual([at(130).kind, at(130).repairing, statusLabel(at(130)), builtLabel(at(130))], ['repairThenNew', false, 'Building, after repair', '1 of 2']);
  assert.deepEqual([statusLabel(at(150)), builtLabel(at(150)), kindLabel(at(150).kind)], ['Repaired and built', '2 of 2', 'Repair, then new']);
  assert.equal(projectTooltip(LOG, at(150), 150, 'Germany')[0].subtitle, 'Germany · Ruhr · repair, then new');
  const dates = (hour: number) => projectTooltip(LOG, at(hour), hour, 'Germany')[0].rows
    .filter(row => ['Repaired', 'Built', 'Finished'].includes(row.label)).map(row => `${row.label}: ${row.value}`);
  assert.deepEqual(dates(150), [`Repaired: ${formatDateTime(110)}`, `Built: ${formatDateTime(140)}`]);
  assert.deepEqual(dates(130), [`Repaired: ${formatDateTime(110)}`, 'Built: Not yet']);
  assert.deepEqual(dates(110), ['Repaired: Not yet', 'Built: Not yet']);
  assert.deepEqual(projectTooltip(LOG, projectAt(PROJECTS[0], 150)!, 150, 'Germany')[0].rows.filter(row => row.label === 'Finished').map(row => row.value),
    [formatDateTime(144)]);
  const log = logOf([both, PROJECTS[0], PROJECTS[1]]);
  const ids = (projects: 'new' | 'repair') => logRows(log, 150, { ...every, projects }, 'start').map(row => row.id).sort();
  assert.deepEqual(ids('repair'), ['16', '2']);
  assert.deepEqual(ids('new'), ['1', '16']);
  assert.equal(projectAt(project({ repair: true }), 110)!.kind, 'new');
  const between = project({ id: '17', runs: [{ firstHour: 100, lastHour: 147, factories: 15, remaining: 1, repairLevels: 0 }],
    built: [{ hour: 120, knownAt: 124, count: 1, repair: true, carried: false }], added: [{ hour: null, knownAt: 124, count: 1, repair: true, carried: false }] });
  const inBetween = (hour: number) => projectAt(between, hour)!;
  assert.deepEqual([inBetween(110).kind, inBetween(130).kind, builtLabel(inBetween(130))], ['new', 'repairThenNew', '1 of 2']);
  assert.deepEqual(logRows(logOf([between]), 130, { ...every, projects: 'repair' }, 'start').map(row => row.id), ['17']);
});


test('the manifest and its windows are checked before use', () => {
  assert.equal(validateConstructionLogManifest(MANIFEST), MANIFEST);
  assert.throws(() => validateConstructionLogManifest({ ...MANIFEST, costScale: 0 }), /malformed/);
  assert.throws(() => validateConstructionLogManifest({ ...MANIFEST, windows: [...MANIFEST.windows].reverse() }), /malformed/);
  assert.throws(() => validateConstructionLogManifest({ ...MANIFEST, missing: undefined as never }), /malformed/);
  const reply = windowOf([...PROJECTS, project({ id: '5', outputs: [RUN] })]);
  assert.equal(validateConstructionLogWindow(reply), reply);
  const bad = (patch: Partial<ConstructionLogReply>) => assert.throws(() => validateConstructionLogWindow({ ...reply, ...patch }), /malformed/);
  bad({ projects: [project({ runs: [{ firstHour: 101, lastHour: 147, factories: 1, remaining: 1, repairLevels: 0 }, { firstHour: 120, lastHour: 150, factories: 1, remaining: 1, repairLevels: 0 }] })] });
  bad({ projects: [project({ end: { kind: 'gone' as 'finished', hour: null, knownAt: 148, unknown: null } })] });
  bad({ projects: [project({ earlierUnknown: undefined as never })] });
  bad({ projects: [project({ possible: undefined as never })] });
  bad({ projects: [project({ possible: [{ hour: null, knownAt: 99, count: 1, repair: false } as never] })] });
  bad({ projects: [project({ outputs: [{ ...RUN, constructionRaw: null }] })] });
  bad({ projects: [project({ outputs: [RUN, RUN] })] });
  bad({ states: undefined as never });
});

test('a place on its own line keeps the game’s names and capitalises only what it says instead', () => {
  const log = logOf(PROJECTS, [...STATES, { id: 7, name: 'île de Test' }]);
  assert.equal(placeTitle(PROJECTS[0], log.states), 'Ruhr');
  assert.equal(placeTitle(project({ state: 7 }), log.states), 'île de Test');
  assert.equal(placeTitle(PROJECTS[3], log.states), 'Moscow to Smolensk');
  assert.equal(placeTitle(project({ state: null, province: 5007 }), log.states), 'Province 5007');
  assert.equal(placeTitle(project({ state: null }), log.states), 'An unknown place');
  assert.equal(placeTitle(project({ kind: 'railway', state: null, toState: null }), log.states), 'An unknown route');
  const unnamed = logOf([PROJECTS[0]], []);
  const tip = datedProjectTooltip(unnamed, projectAt(PROJECTS[0], 124)!, 124, 'Country')[0];
  assert.match(tip.subtitle ?? '', /State 5/);
  assert.match(tip.note ?? '', /State 5: not named by the reply/);
});

test('the log keeps exact signed output, writes it in whole IC and distinguishes natural repair from factory IC', () => {
  const p = project({ firstRepairTick: 124, outputs: [{ ...RUN, constructionRaw: '9007199254741499', repairRaw: '-1', beforeObservation: true }] });
  const log = logOf([p]);
  assert.equal(projectAt(p, 100)!.kind, 'new');
  const at = projectAt(p, 124)!;
  assert.equal(at.kind, 'repairThenNew');
  const tip = projectTooltip(log, at, 124, 'Synthetic country')[0];
  assert.equal(tip.rows?.find(row => row.label === 'Construction output')?.value, '90,071,992,547');
  assert.equal(tip.rows?.find(row => row.label === 'Repair output')?.value, '0');
  assert.deepEqual([outputAt(p, 124)?.constructionRaw, outputAt(p, 124)?.repairRaw, outputAt(p, 124)?.knownAt], ['9007199254741499', '-1', 100]);
  assert.equal(outputAt(p, 99), null);
  assert.match(tip.note ?? '', /Earlier progress unknown/);
  const linked = project({ outputs: [{ ...RUN, constructionRaw: null, repairRaw: null, reason: 'linkedOutput' }] });
  const unknown = projectTooltip(logOf([linked]), projectAt(linked, 124)!, 124, 'Germany')[0];
  assert.equal(unknown.rows?.find(row => row.label === 'Construction output')?.value, 'Not recorded');
  assert.match(unknown.note ?? '', /Output unavailable/);
});

test('a line several windows list is one line: runs and output joined, counts carried only from the oldest held', () => {
  assert.equal(windowAt(MANIFEST.windows, 50), 0);
  assert.equal(windowAt(MANIFEST.windows, 159), 1);
  assert.equal(windowAt(MANIFEST.windows, 400), 2);
  const joined = joinProject(joinProject(SPLIT[0], SPLIT[1]), SPLIT[2]);
  assert.deepEqual(joined.runs, PROJECTS[2].runs);
  assert.deepEqual(joined.built, PROJECTS[2].built);
  assert.deepEqual(joined.end, PROJECTS[2].end);
  const outputs = joinProject({ ...SPLIT[0], outputs: [{ ...RUN, from: 100, to: 119 }] },
    { ...SPLIT[1], outputs: [{ ...RUN, from: 120, to: 140 }, { ...RUN, from: 141, to: 151, constructionRaw: '2' }] }).outputs;
  assert.deepEqual(outputs.map(run => [run.from, run.to, run.constructionRaw]), [[100, 140, '1'], [141, 151, '2']]);
  const windows = new Map(SPLIT.map((item, index) => [index, windowOf([item], index)]));
  for (const hour of [110, 124, 130, 160, 172, 200]) {
    const current = windowAt(MANIFEST.windows, hour);
    const whole = projectAt(PROJECTS[2], hour)!;
    for (let first = 0; first <= current; first++) {
      const held = mergeWindows(MANIFEST, new Map([...windows].filter(([index]) => index >= first && index <= current)), current);
      const at = projectAt(held.projects[0], hour)!;
      const since = (changed: number) => changed >= MANIFEST.windows[first].firstHour ? changed : null;
      assert.deepEqual([at.status, at.built, at.stage, since(at.changed)], [whole.status, whole.built, whole.stage, since(whole.changed)], `${hour} from window ${first}`);
    }
  }
});

test('the log holds the windows its drawn rows need, reaching back only when short', () => {
  assert.deepEqual(constructionWindows(5, 2, true, null), [1, 2, 3]);
  assert.deepEqual(constructionWindows(5, 4, true, null), [3, 4]);
  assert.deepEqual(constructionWindows(0, 0, true, null), []);
  assert.deepEqual(constructionWindows(5, 3, false, { at: 2, lower: 0 }), [0, 1, 2, 3]);
  assert.deepEqual(constructionWindows(5, 4, false, { at: 2, lower: 0 }), [3, 4]);
  const log = mergeWindows(MANIFEST, new Map([[1, windowOf(PROJECTS, 1)], [2, windowOf([SPLIT[2]], 2)]]), 2);
  const rows = logRows(log, 172, every, 'change');
  assert.deepEqual(rows.map(row => row.key), ['3:2', '4:2', '1:2']);
  assert.equal(logReach(log, rows, rows.slice(0, 1), null), 1);
  assert.equal(logReach(log, rows, rows, null), 0);
  assert.equal(logReach(log, rows, rows, 130), 1);
});

test('rows change shape only at gaps and where repair or new work starts or stops; factories and levels between are painted', () => {
  const runs = [{ firstHour: 100, lastHour: 111, factories: 5, remaining: 3, repairLevels: 0 }, { firstHour: 112, lastHour: 119, factories: 8, remaining: 2, repairLevels: 0 },
    { firstHour: 120, lastHour: 129, factories: 8, remaining: 2, repairLevels: 1 }, { firstHour: 140, lastHour: 149, factories: 3, remaining: 2, repairLevels: 1 }];
  assert.deepEqual([...shapeChanges(runs)], [100, 120, 130, 140, 150]);
  const line = project({ runs, built: [{ hour: 106, knownAt: 106, count: 1, repair: false, carried: false }] });
  const log = logOf([line]);
  const playback = constructionPlayback(log, every, 'start');
  assert.equal(playback.at(115), playback.at(107), 'eight factories from 112 make no new rows');
  assert.equal(projectText(line, 'factories', 107), '5');
  assert.equal(projectText(line, 'factories', 115), '8');
  assert.equal(projectText(line, 'levels', 107), '1 of 4');
  assert.equal(projectText(line, 'levels', 115), '1 of 3');
  assert.equal(projectText(line, 'factories', 135), '—', 'a gap is not recorded, never zero');
  assert.equal(projectText(line, 'factories', 99), '');
  let hour = 107;
  const clock = { subscribeFrame: () => () => {}, subscribe: () => () => {} };
  const live = liveProjectTexts(clock, () => hour, new Map([[line.id, line]]));
  assert.equal(live.text(liveKey(line.id, 'factories')), '5');
  hour = 112;
  assert.equal(live.text(liveKey(line.id, 'factories')), '8');
  assert.equal(live.text(liveKey('unknown', 'factories')), '');
  assert.equal(heldThrough(log), 300);
  const split = mergeWindows(MANIFEST, new Map([[0, windowOf([], 0)], [1, windowOf([], 1)]]), 0);
  assert.equal(heldThrough(split), 159, 'held through the next window, not past it');
});

test('over a block the log holds each row\'s states, at every hour exactly that hour\'s rows in its order, keyed alike in the next block', () => {
  const base = (key: string) => key.slice(0, key.lastIndexOf('#'));
  for (const order of ['change', 'start'] as const) for (const filter of [every, { ...every, queue: true }, { ...every, projects: 'repair' as const }]) {
    const playback = constructionPlayback(LOG, filter, order);
    const states = playback.states(100, 180);
    for (let hour = 100; hour <= 180; hour++) {
      const shown = states.filter(row => Math.max(row.hour, row.revealedHour ?? row.hour) <= hour && (row.leftHour ?? Infinity) > hour);
      assert.deepEqual(shown.map(row => [base(row.key), row.hour, row.item.status]), logRows(LOG, hour, filter, order).map(row => [row.key, row.hour, row.item.status]),
        `${order}, ${JSON.stringify(filter)} at ${hour}`);
    }
    assert.equal(new Set(states.map(row => row.key)).size, states.length);
    const keys = new Set(states.map(row => row.key));
    for (const row of playback.states(130, 130)) assert.ok(keys.has(row.key), `${row.key} keeps its key in the block from 130`);
  }
});

test('unread built counters stay per-hour and unknown kind or source stay missing and never read as zero or known from output', () => {
  const unreadBuilt = project({ id: '20', runs: [{ firstHour: 100, lastHour: 147, factories: 10, remaining: 2, repairLevels: 0 }], builtUnread: [[110, 125]] });
  validateConstructionLogWindow(windowOf([unreadBuilt]));
  assert.equal(builtLabel(projectAt(unreadBuilt, 105)!), '0 of 2', 'recorded zero before unread span stays zero');
  const atUnread = projectAt(unreadBuilt, 120)!;
  assert.equal(builtLabel(atUnread), 'Not recorded');
  assert.equal(levelsLabel(atUnread), 'Not recorded');
  const tipUnread = projectTooltip(logOf([unreadBuilt]), atUnread, 120, 'Germany')[0];
  assert.deepEqual(tipUnread.rows.find(row => row.label === 'Levels built'), { label: 'Levels built', value: 'Not recorded', tone: 'muted' });
  assert.equal(builtLabel(projectAt(unreadBuilt, 130)!), '0 of 2', 'recorded zero after unread span stays zero');
  const playback = constructionPlayback(logOf([unreadBuilt]), every, 'start');
  assert.equal(playback.at(105)[0].item.countersUnread, false);
  assert.equal(playback.at(120)[0].item.countersUnread, true);
  assert.equal(playback.at(130)[0].item.countersUnread, false);

  const censusMissingCreated = project({ id: '21', source: 'census', createdHour: null, firstRead: 100, runs: [{ firstHour: 100, lastHour: 147, factories: 10, remaining: 2, repairLevels: 0 }] });
  const tipCensus = projectTooltip(logOf([censusMissingCreated]), projectAt(censusMissingCreated, 120)!, 120, 'Germany')[0];
  assert.deepEqual(tipCensus.rows.find(row => row.label === 'Started'), { label: 'Started', value: 'Not recorded', tone: 'muted' });
  assert.ok(!tipCensus.rows.some(row => row.value === 'Known from output'), 'a census project with missing createdHour never claims Known from output');

  const unknown = project({ id: '22', kind: 'unknown', source: 'unknown', censusRead: null, repair: null, createdHour: null, runs: [] });
  validateConstructionLogWindow(windowOf([unknown]));
  const atUnknown = projectAt(unknown, 120)!;
  assert.deepEqual([atUnknown.status, atUnknown.kind, builtLabel(atUnknown), levelsLabel(atUnknown)], ['unread', 'unknown', 'Not recorded', null]);
  const tipUnknown = projectTooltip(logOf([unknown]), atUnknown, 120, 'Germany')[0];
  assert.equal(tipUnknown.rows.find(row => row.label === 'Source')?.value, 'Not recorded');
  assert.equal(tipUnknown.rows.find(row => row.label === 'Progress')?.value, 'Not recorded');

  const unknownSourceWithBuilds = project({
    id: '25', source: 'unknown', censusRead: 100, createdHour: 90, runs: [{ firstHour: 100, lastHour: 109, factories: 5, remaining: 2, repairLevels: 0 }],
    built: [{ hour: 108, knownAt: 110, count: 2, repair: false, carried: false }], end: { kind: 'finished', hour: 108, knownAt: 110, unknown: null },
  });
  const atUnknownSource = projectAt(unknownSourceWithBuilds, 110)!;
  assert.equal(builtLabel(atUnknownSource), '2 of 2', 'an unread source flag still shows recorded completions from counter evidence');
  assert.equal(levelsLabel(atUnknownSource), '×2');
  const tipUnknownSource = projectTooltip(logOf([unknownSourceWithBuilds]), atUnknownSource, 110, 'Germany')[0];
  assert.deepEqual(tipUnknownSource.rows.find(row => row.label === 'Source'), { label: 'Source', value: 'Not recorded', tone: 'muted' });
  assert.equal(tipUnknownSource.rows.find(row => row.label === 'Levels built')?.value, '2 of 2');
  assert.equal(tipUnknownSource.rows.find(row => row.label === 'Factories at its last read')?.value, '5');
  assert.match(describeRow(logOf([unknownSourceWithBuilds]), logRows(logOf([unknownSourceWithBuilds]), 110, every, 'change')[0], 'Germany'), /×2.*source not recorded/);

  const recovered = project({
    id: '26', runs: [{ firstHour: 100, lastHour: 119, factories: 5, remaining: 1, repairLevels: 0 }],
    built: [{ hour: 108, knownAt: 110, count: 1, repair: false, carried: false }], builtUnread: [[100, 109]],
  });
  const atRecovered = projectAt(recovered, 110)!;
  assert.deepEqual([atRecovered.changed, atRecovered.changedBy, atRecovered.countersUnread], [108, false, false]);
  assert.ok(jumpHours(atRecovered, 110).includes(108), 'a dated rise after an unread step offers the 108 jump');

  const truncatedOmission = project({
    id: '27', runs: [{ firstHour: 100, lastHour: 105, factories: 5, remaining: 2, repairLevels: 0 }],
    built: [], builtUnread: [[104, 105]],
  });
  const atTruncated = projectAt(truncatedOmission, 106)!;
  assert.equal(atTruncated.countersUnread, false);
  assert.equal(builtLabel(atTruncated), '0 of 2', 'a truncated omission keeps main count display at 106');

  const boundaryWin0 = project({ id: '28', runs: [{ firstHour: 100, lastHour: 119, factories: 5, remaining: 2, repairLevels: 0 }],
    built: [{ hour: 110, knownAt: 110, count: 1, repair: false, carried: false }] });
  const boundaryWin1 = project({ id: '28', runs: [{ firstHour: 120, lastHour: 140, factories: 5, remaining: 1, repairLevels: 0 }],
    built: [{ hour: null, knownAt: 120, count: 2, repair: false, carried: true }] });
  assert.equal(projectAt(joinProject(boundaryWin0, boundaryWin1), 120)!.built, 2, 'a rise from 1 to 2 at a window boundary shows 2 instead of dropping incoming carry');

  const joined = joinProject(unreadBuilt, { ...unknown, id: '20', builtUnread: [[126, 130]] });
  assert.deepEqual(joined.builtUnread, [[110, 130]]);
  assert.equal(joined.kind, 'building');
  assert.equal(joined.source, 'census', 'joinProject keeps an earlier witnessed source when a later window has unknown source');

  const unreadWin0 = project({ id: '23', runs: [{ firstHour: 100, lastHour: 110, factories: 10, remaining: 3, repairLevels: 0 }], built: [], builtUnread: [[100, 110]] });
  const win1Carried = project({ id: '23', runs: [{ firstHour: 111, lastHour: 120, factories: 10, remaining: 1, repairLevels: 0 }],
    built: [{ hour: null, knownAt: 112, count: 2, repair: false, carried: true }] });
  const joinedCarried = joinProject(unreadWin0, win1Carried);
  assert.equal(projectAt(joinedCarried, 105)!.built, 0);
  assert.equal(projectAt(joinedCarried, 105)!.countersUnread, true);
  assert.equal(projectAt(joinedCarried, 112)!.built, 2, 'carried built count after an unread window is never dropped at the join');
  assert.equal(projectAt(joinedCarried, 112)!.countersUnread, false);

  const partialWin0 = project({ id: '24', runs: [{ firstHour: 100, lastHour: 110, factories: 10, remaining: 3, repairLevels: 0 }],
    built: [{ hour: 102, knownAt: 102, count: 1, repair: false, carried: false }], builtUnread: [[105, 110]] });
  const win1Risen = project({ id: '24', runs: [{ firstHour: 111, lastHour: 120, factories: 10, remaining: 0, repairLevels: 0 }],
    built: [{ hour: null, knownAt: 112, count: 3, repair: false, carried: true }] });
  const joinedDelta = joinProject(partialWin0, win1Risen);
  assert.equal(projectAt(joinedDelta, 104)!.built, 1);
  assert.equal(projectAt(joinedDelta, 112)!.built, 3, 'carried cumulative count adds only the unseen delta above window 0');
});
