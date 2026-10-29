import assert from 'node:assert/strict';
import test from 'node:test';
import { mergePreparedAutonomy, overlayReachedAutonomy, validatePreparedAutonomy } from './preparedAutonomy.ts';
import { autonomyViews, subjectLane } from './autonomyData.ts';
import { countryIdentityIndex } from '../../dashboard/countryIdentity.ts';
import { preparedWindowQuery } from '../../dashboard/preparedSeries.ts';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import type { PreparedAutonomy } from './preparedAutonomy';

const band = { firstHour: 0, lastHour: 1000, startsSegment: true, level: 0, overlord: null, lowest: true, highest: true };
const reply = { branchId: '1', scoreScale: 100000, levels: [{ id: 'mod/level', name: '', defined: true }],
  subjects: [{ tag: 'A', runs: [{ ...band, firstHour: 500, lastHour: 501, startsSegment: false,
    scoreRaw: '-9223372036854775808', lowerRaw: '-10', upperRaw: '20', ratioRaw: null }] }],
  identities: [{ tag: 'A', identities: [{ firstHour: 0, lastHour: 1000, slot: 1, countryId: 10 }] }],
  bands: [{ tag: 'A', runs: [band] }], recorded: [{ firstHour: 0, lastHour: 1000 }], unidentified: [], warnings: [], exactHours: [500], summaries: {} };

test('summary pages retain every page marker and the least prepared coverage', () => {
  const approximate = [{ firstHour: 500, lastHour: 1000 }];
  const summary = { from: 0, to: 1000, width: 1, buckets: [], approximate,
    runs: [{ firstHour: 0, lastHour: 1000, value: 1, breakBefore: true }] };
  const exact = validatePreparedAutonomy({ ...reply, levels: [...reply.levels, { id: 'mod/new-level', name: '', defined: true }],
    coverage: { firstHour: 0, lastHour: 1000, complete: true }, approximate: [], pending: [], exactHours: [],
    bands: [{ tag: 'A', runs: [{ ...band, lastHour: 749 }, { ...band, firstHour: 750, level: 1 }] }] });
  const rough = validatePreparedAutonomy({ ...reply, coverage: { firstHour: 0, lastHour: 499, complete: false },
    approximate, pending: [], exactHours: [], summaries: { B: summary } });
  for (const [a, b] of [[exact, rough], [rough, exact]]) {
    const merged = mergePreparedAutonomy(a, b);
    assert.deepEqual(merged.approximate, approximate, 'another page cannot make sampled geometry exact');
    assert.deepEqual(merged.pending, []);
    assert.deepEqual(merged.coverage, rough.coverage);
    assert.deepEqual(merged.summaries.B, summary, 'numeric geometry keeps its original sampling evidence');
  }
  const merged = mergePreparedAutonomy(exact, rough);
  assert.equal(merged.bands[0].runs.find(run => run.firstHour <= 750 && 750 <= run.lastHour)!.level, 0,
    'the older sampled page missed the change at 750, so its band must stay approximate');
  const waiting = validatePreparedAutonomy({ ...rough, approximate: [], pending: approximate, summaries: {} });
  for (const [a, b] of [[exact, waiting], [waiting, exact]]) {
    const combined = mergePreparedAutonomy(a, b);
    assert.deepEqual(combined.pending, approximate, 'pending is a union across pages too');
    assert.deepEqual(combined.coverage, waiting.coverage);
  }
  const complete = mergePreparedAutonomy(exact, exact);
  assert.deepEqual(complete.approximate, []);
  assert.deepEqual(complete.pending, []);
  assert.equal(complete.coverage?.complete, true);
});

test('transport-split summaries stay approximate while only one page refreshes to completion', async () => {
  const approximate = [{ firstHour: 500, lastHour: 1000 }];
  const partial = { ...reply, exactHours: [], coverage: { firstHour: 0, lastHour: 499, complete: false }, approximate, pending: [] };
  const exact = { ...partial, coverage: { firstHour: 0, lastHour: 1000, complete: true }, approximate: [] };
  const asks = new Map<string, number>();
  const statuses: (() => void)[] = [];
  const controller = new AbortController();
  const published: PreparedAutonomy[] = [];
  const client = {
    preparedQuery: async (_source: unknown, product: string, params: { within: Record<string, unknown> }) => {
      assert.equal(product, 'series-summary');
      const ids = Object.keys(params.within);
      if (ids.length > 1) throw new Error('Autonomy summary exceeds the transport reply budget.');
      const id = ids[0], count = asks.get(id) ?? 0;
      asks.set(id, count + 1);
      return count === 0 ? partial : exact;
    },
    preparationStatus: (request: object, signal: AbortSignal) => new Promise((resolve, reject) => {
      signal.addEventListener('abort', () => reject(new DOMException('cancelled', 'AbortError')), { once: true });
      statuses.push(() => resolve({ ...request, state: 'ready', coverage: { '1': exact.coverage }, families: {},
        sections: [], backgroundEnabled: true, bytes: '0', error: null }));
    }),
  } as unknown as DashboardClient;
  const query = preparedWindowQuery(client, { id: 'autonomy-pages', revision: '1' } as DashboardSource, 'autonomy', { branchId: '1' },
    { from: 0, to: 1000, width: 1, series: ['A', 'B'].map(id => ({ id, spans: [{ ...band, tag: id }] })) }, controller.signal,
    validatePreparedAutonomy, mergePreparedAutonomy, false, value => published.push(value));
  try {
    await new Promise<void>(resolve => setImmediate(resolve));
    assert.equal(statuses.length, 2);
    statuses[0]();
    await new Promise<void>(resolve => setImmediate(resolve));
    const unequal = published.at(-1)!;
    assert.deepEqual(unequal.coverage, partial.coverage);
    assert.deepEqual(unequal.approximate, approximate, 'a refreshed page cannot clear its sibling sampling mark');
    assert.deepEqual([...asks.values()], [2, 1]);
    statuses[1]();
    const complete = await query;
    assert.deepEqual(complete.coverage, exact.coverage);
    assert.deepEqual(complete.approximate, []);
    assert.deepEqual(complete.pending, []);
  } finally { controller.abort(); await query.catch(() => {}); }
});

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
    { approximate: [{ firstHour: 5, lastHour: 4 }] },
    { pending: [{ firstHour: 5, lastHour: 4 }] },
    { bands: [{ tag: 'A', runs: [{ ...band, lastHour: -1 }] }] },
    { bands: [{ tag: 'A', runs: [{ ...band, startsSegment: undefined }] }] },
    { summaries: { A: { from: 0, to: 1000, width: 1, buckets: [null, null, null, null, null] } } },
    { summaries: { A: { from: 0, to: 1000, width: 1, buckets: [{ firstHour: 0, lastHour: 5, maxHour: 6 }] } } },
    { exactHours: [NaN] },
  ]) assert.throws(() => validatePreparedAutonomy({ ...reply, ...changes }));
});

test('root sampling survives empty numeric summaries and a reached absence at the last hour', () => {
  const approximate = [{ firstHour: 0, lastHour: 499 }, { firstHour: 500, lastHour: 1000 }];
  const rough = validatePreparedAutonomy({ ...reply, subjects: [], bands: [], summaries: {}, exactHours: [], approximate });
  assert.deepEqual(rough.approximate, approximate);
  const exact = validatePreparedAutonomy({ ...reply, subjects: [], bands: [], summaries: {}, exactHours: [1000] });
  const merged = overlayReachedAutonomy(rough, exact);
  assert.deepEqual(merged.approximate, [{ firstHour: 0, lastHour: 1000 }]);
  assert.deepEqual(merged.summaries, {});
  assert.deepEqual(merged.metadata.subjects, []);
});

test('history pending in reached replies yields to sampled geometry and its exact prepared prefix', () => {
  const cold = { firstHour: 0, lastHour: -1, complete: false };
  const rough = validatePreparedAutonomy({ ...reply, coverage: cold, approximate: [{ firstHour: 0, lastHour: 999 }],
    pending: [{ firstHour: 1000, lastHour: 1000 }] });
  const reached = validatePreparedAutonomy({ ...reply, coverage: cold, exactHours: [1000],
    pending: [{ firstHour: 0, lastHour: 1000 }] });
  const merged = overlayReachedAutonomy(rough, reached);
  assert.deepEqual(merged.approximate, rough.approximate);
  assert.deepEqual(merged.pending, rough.pending);
  const exact = validatePreparedAutonomy({ ...reply, coverage: { firstHour: 0, lastHour: 1000, complete: true },
    approximate: [], pending: [] });
  const complete = overlayReachedAutonomy(exact, reached);
  assert.deepEqual(complete.approximate, []);
  assert.deepEqual(complete.pending, []);
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
  const merged = overlayReachedAutonomy(rough, exact);
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
  const merged = overlayReachedAutonomy(rough, exact);
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
  const merged = overlayReachedAutonomy(rough, read());
  assert.equal(requests, 1);
  assert.equal(merged.subjects.length, 5000);
  assert.equal(merged.subjects.every(subject => subject.runs[1].scoreRaw === '500'), true);
});

test('an exact read that recorded no autonomy at its hour leaves that hour unrecorded, not the rough block covered', () => {
  const rough = validatePreparedAutonomy({ ...reply, exactHours: [], coverage: { firstHour: 0, lastHour: 1000, complete: false },
    approximate: [{ firstHour: 0, lastHour: 1000 }], pending: [], unidentified: [{ firstHour: 590, lastHour: 610 }] });
  const missing = validatePreparedAutonomy({ ...reply, subjects: [], bands: [], recorded: [], unidentified: [], exactHours: [600] });
  const reached = overlayReachedAutonomy(rough, missing);
  assert.deepEqual(reached.recorded, [{ firstHour: 0, lastHour: 599 }, { firstHour: 601, lastHour: 1000 }]);
  assert.deepEqual(reached.unidentified, [{ firstHour: 590, lastHour: 599 }, { firstHour: 601, lastHour: 610 }]);
  assert.equal(reached.bands[0].runs.some(run => run.firstHour <= 600 && 600 <= run.lastHour), false, 'its subject run is cut as before');
  const recorded = validatePreparedAutonomy({ ...missing, recorded: [{ firstHour: 600, lastHour: 600 }] });
  assert.deepEqual(overlayReachedAutonomy(rough, recorded).recorded, [{ firstHour: 0, lastHour: 1000 }], 'a recorded exact hour joins its neighbours again');
});
