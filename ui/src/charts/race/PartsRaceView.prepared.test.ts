import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { NativePartsRaceReply } from './partsRaceDrill';

test('prepared force pages align part keys and preserve unknown parts and all 5000 countries', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'force-private-helpers', enforce: 'pre',
      transform(source, id) { if (id.replaceAll('\\', '/').endsWith('/charts/race/PartsRaceView.tsx'))
        return source.replace('function mergePreparedParts(', 'export function mergePreparedParts(')
          .replace('function preparedPartsGeometry(', 'export function preparedPartsGeometry('); },
    }] });
  try {
    const { mergePreparedParts } = await vite.ssrLoadModule('/ui/src/charts/race/PartsRaceView.tsx');
    const make = (part: string, firstHour: number, value: number | null): NativePartsRaceReply => ({ branchId: '1',
      parts: [{ key: part, name: part, unresolved: value === null }], warnings: [],
      series: Array.from({ length: 5000 }, (_, id) => ({ tag: `opaque / ${id}`, identities: [{ slot: id, countryId: id, firstHour: 0, lastHour: 100 }],
        segments: [{ firstHour: 0, lastHour: 100 }], membership: null,
        runs: [{ firstHour, lastHour: firstHour, startsSegment: false, total: 10, values: [value], referenceTotal: null }] })) });
    const result = mergePreparedParts([make('z', 0, null), make('a', 1, 10)]) as NativePartsRaceReply;
    assert.equal(result.series.length, 5000);
    assert.deepEqual(result.parts.map(part => part.key), ['a', 'z']);
    assert.deepEqual(result.series[4999].runs.map(run => run.values), [[0, null], [10, 0]]);
    assert.equal(result.series[4999].runs[0].referenceTotal, null);
    assert.equal(result.series[4999].identities[0].slot, 4999);
    const earlier = make('a', 0, 10), later = make('a', 8, 10);
    earlier.series[0].membership = [{ firstHour: 0, lastHour: 1 }]; later.series[0].membership = [{ firstHour: 8, lastHour: 9 }];
    const reached = mergePreparedParts([earlier, later]) as NativePartsRaceReply;
    assert.deepEqual(reached.series[0].membership, [{ firstHour: 0, lastHour: 1 }, { firstHour: 8, lastHour: 9 }]);

  } finally { await vite.close(); }
});

test('force dense stack geometry uses bounded known coverage while exact windows keep their original runs', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'force-stack-helper', enforce: 'pre',
      transform(source, id) { if (id.replaceAll('\\', '/').endsWith('/charts/race/PartsRaceView.tsx'))
        return source.replace('function preparedPartsGeometry(', 'export function preparedPartsGeometry('); },
    }] });
  try {
    const { preparedPartsGeometry } = await vite.ssrLoadModule('/ui/src/charts/race/PartsRaceView.tsx');
    const reply = { branchId: '1', parts: [{ key: 'mod class', name: 'Mod class', unresolved: false }], warnings: [],
      series: [{ tag: 'a', identities: [], segments: [{ firstHour: 0, lastHour: 9 }], membership: null,
        runs: [{ firstHour: 0, lastHour: 0, startsSegment: true, total: 2, values: [2] },
          { firstHour: 8, lastHour: 8, startsSegment: false, total: 8, values: [8] }] }],
      summaries: [{ tag: 'a', summary: { from: 0, to: 9, width: 1, buckets: [{ firstHour: 0, lastHour: 9,
        firstValue: 2, lastValue: 8, lastValueHour: 8, minValue: 2, minHour: 0, maxValue: 8, maxHour: 8,
        firstBreak: true, hasBreak: false }] }, parts: [] }] };
    const dense = preparedPartsGeometry(reply);
    assert.deepEqual(dense.series[0].runs.map((run: { firstHour: number; lastHour: number }) => [run.firstHour, run.lastHour]), [[0, 7], [8, 9]]);
    assert.deepEqual(reply.series[0].runs.map(run => run.lastHour), [0, 8], 'geometry must not mutate exact amounts');
    const broken = structuredClone(reply); broken.summaries[0].summary.buckets[0].hasBreak = true;
    assert.deepEqual(preparedPartsGeometry(broken).series[0].runs.map((run: { lastHour: number }) => run.lastHour), [0, 8]);
    const exact = { ...reply, summaries: [{ ...reply.summaries[0], summary: { ...reply.summaries[0].summary, runs: [] } }] };
    assert.equal(preparedPartsGeometry(exact).series[0], exact.series[0]);
  } finally { await vite.close(); }
});

test('the shared prepared race hook accepts nuclear native page merging without converting its counters', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { mergeNukes, nukesGeometry } = await vite.ssrLoadModule('/ui/src/widgets/nuclear-arsenal/preparedNukes.ts');
    const { usePreparedRace } = await vite.ssrLoadModule('/ui/src/charts/race/PartsRaceView.tsx');
    assert.equal(typeof usePreparedRace, 'function');
    const first = { branchId: '1', readHours: [1], emptyReadHours: [], types: [{ ordinal: 7, token: null, name: null, supported: false }], warnings: [],
      series: [{ tag: 'mod', membership: [{ firstHour: 1, lastHour: 4 }], segments: [{ firstHour: 1, lastHour: 4 }],
        runs: [{ firstHour: 1, lastHour: 2, startsSegment: true, total: null, counts: [{ ordinal: 7, value: -12 }] }] }] };
    const reached = structuredClone(first); reached.readHours = [2]; reached.series[0].runs[0].firstHour = 2;
    reached.series[0].runs[0].lastHour = 3; reached.series[0].runs[0].startsSegment = false;
    const joined = mergeNukes([first, reached]);
    assert.deepEqual(joined.readHours, [1, 2]); assert.equal(joined.series[0].runs.length, 1);
    assert.equal(joined.series[0].runs[0].total, null); assert.equal(joined.series[0].runs[0].counts[0].value, -12);
    assert.equal(joined.series[0].runs[0].startsSegment, true); assert.equal(joined.series[0].runs[0].lastHour, 3);
    assert.deepEqual(nukesGeometry(joined), joined);
  } finally { await vite.close(); }
});
