import assert from 'node:assert/strict';
import test from 'node:test';
import { mergePreparedAutonomy, validatePreparedAutonomy } from './preparedAutonomy.ts';
import { autonomyViews, subjectLane } from './autonomyData.ts';
import { countryIdentityIndex } from '../../dashboard/countryIdentity.ts';

const band = { firstHour: 0, lastHour: 1000, startsSegment: true, level: 0, overlord: null, lowest: true, highest: true };
const reply = { branchId: '1', scoreScale: 100000, levels: [{ id: 'mod/level', name: '', defined: true }],
  subjects: [{ tag: 'A', runs: [{ ...band, firstHour: 500, lastHour: 501, startsSegment: false,
    scoreRaw: '-9223372036854775808', lowerRaw: '-10', upperRaw: '20', ratioRaw: null }] }],
  identities: [{ tag: 'A', identities: [{ firstHour: 0, lastHour: 1000, slot: 1, countryId: 10 }] }],
  bands: [{ tag: 'A', runs: [band] }], recorded: [{ firstHour: 0, lastHour: 1000 }], unidentified: [], warnings: [], exactHours: [500], summaries: {} };

test('prepared categorical spans retain holder boundaries while exact reached values stay separate', () => {
  const data = validatePreparedAutonomy(reply);
  assert.equal(data.subjects[0].runs[0].scoreRaw, '-9223372036854775808');
  assert.deepEqual(data.metadata.subjects[0].runs.map(run => [run.firstHour, run.lastHour]), [[0, 1000]]);
  const identities = countryIdentityIndex([{ tag: 'A', identities: [
    { firstHour: 0, lastHour: 499, slot: 1, countryId: 10 }, { firstHour: 500, lastHour: 1000, slot: 1, countryId: 20 },
  ] }]);
  assert.deepEqual(autonomyViews(data.metadata, identities).map(subject => subjectLane(subject, data.levels).stretches),
    [[{ firstHour: 0, lastHour: 499 }], [{ firstHour: 500, lastHour: 1000 }]]);
  assert.deepEqual(autonomyViews(data, identities).map(subject => subject.runs[0].scoreRaw), ['-9223372036854775808']);
});

test('malformed bands and unbounded or invalid summary geometry are refused', () => {
  for (const changes of [
    { bands: [{ tag: 'A', runs: [{ ...band, lastHour: -1 }] }] },
    { bands: [{ tag: 'A', runs: [{ ...band, startsSegment: undefined }] }] },
    { summaries: { A: { from: 0, to: 1000, width: 1, buckets: [null, null, null, null, null] } } },
    { summaries: { A: { from: 0, to: 1000, width: 1, buckets: [{ firstHour: 0, lastHour: 5, maxHour: 6 }] } } },
    { exactHours: [NaN] },
  ]) assert.throws(() => validatePreparedAutonomy({ ...reply, ...changes }));
});


test('exact reached replies union cohorts, remap levels and replace rough holder and numeric evidence', () => {
  const rough = validatePreparedAutonomy({ ...reply, exactHours: [], subjects: [{ tag: 'A', runs: [{ ...reply.subjects[0].runs[0], firstHour: 0, lastHour: 1000 }] }] });
  const exact = validatePreparedAutonomy({ ...reply, levels: [{ id: 'new/mod-level', name: null, defined: false }, ...reply.levels],
    subjects: [{ tag: 'A', runs: [{ ...reply.subjects[0].runs[0], firstHour: 500, lastHour: 500, level: 1, scoreRaw: '123' }] },
      { tag: 'new subject', runs: [{ ...reply.subjects[0].runs[0], firstHour: 500, lastHour: 500, level: 0 }] }],
    bands: [{ tag: 'A', runs: [{ ...band, firstHour: 500, lastHour: 500, level: 1 }] },
      { tag: 'new subject', runs: [{ ...band, firstHour: 500, lastHour: 500, level: 0 }] }],
    identities: [{ tag: 'A', identities: [{ firstHour: 500, lastHour: 500, slot: 1, countryId: 20 }] },
      { tag: 'new subject', identities: [{ firstHour: 500, lastHour: 500, slot: 2, countryId: 21 }] }] });
  const merged = mergePreparedAutonomy(rough, exact);
  assert.deepEqual(merged.levels.map(level => level.id), ['mod/level', 'new/mod-level']);
  assert.deepEqual(merged.subjects.map(subject => subject.tag), ['A', 'new subject']);
  assert.deepEqual(merged.subjects[0].runs.map(run => [run.firstHour, run.lastHour, run.scoreRaw]),
    [[0, 499, '-9223372036854775808'], [500, 500, '123'], [501, 1000, '-9223372036854775808']]);
  assert.equal(merged.subjects[1].runs[0].level, 1);
  assert.equal(merged.metadata.subjects[1].runs[0].level, 1);
  assert.equal(countryIdentityIndex(merged.identities).keyAt('A', 500), JSON.stringify([1, 20]));
  assert.equal(countryIdentityIndex(merged.identities).keyAt('A', 501), JSON.stringify([1, 10]));
  assert.equal(rough.subjects[0].runs[0].lastHour, 1000);
});

test('an exact unknown clears estimated values and categorical geometry at the reached hour', () => {
  const rough = validatePreparedAutonomy({ ...reply, exactHours: [], subjects: [{ tag: 'A', runs: [{ ...reply.subjects[0].runs[0], firstHour: 0, lastHour: 1000 }] }] });
  const exact = validatePreparedAutonomy({ ...reply, subjects: [], bands: [], identities: [], exactHours: [500] });
  const merged = mergePreparedAutonomy(rough, exact);
  assert.deepEqual(merged.subjects[0].runs.map(run => [run.firstHour, run.lastHour]), [[0, 499], [501, 1000]]);
  assert.deepEqual(merged.bands[0].runs.map(run => [run.firstHour, run.lastHour]), [[0, 499], [501, 1000]]);
  assert.deepEqual(merged.exactHours, [500]);
});

test('reached state at 5000 countries uses one request and preserves every subject', () => {
  const subjects = Array.from({ length: 5000 }, (_, index) => ({ tag: `mod-${index}`, runs: [{ ...reply.subjects[0].runs[0], firstHour: 0, lastHour: 1000 }] }));
  const rough = validatePreparedAutonomy({ ...reply, subjects, bands: subjects.map(subject => ({ tag: subject.tag, runs: [band] })),
    identities: subjects.map((subject, slot) => ({ tag: subject.tag, identities: [{ firstHour: 0, lastHour: 1000, slot, countryId: slot }] })), exactHours: [] });
  let requests = 0;
  const read = () => { requests++; return validatePreparedAutonomy({ ...rough, subjects: subjects.map(subject => ({ ...subject, runs: [{ ...subject.runs[0], firstHour: 500, lastHour: 500, scoreRaw: '500' }] })),
    bands: [], exactHours: [500], identities: [] }); };
  const merged = mergePreparedAutonomy(rough, read());
  assert.equal(requests, 1);
  assert.equal(merged.subjects.length, 5000);
  assert.equal(merged.subjects.every(subject => subject.runs[1].scoreRaw === '500'), true);
});
