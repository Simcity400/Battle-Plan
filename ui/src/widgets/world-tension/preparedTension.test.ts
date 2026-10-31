import assert from 'node:assert/strict';
import test from 'node:test';
import type { ChartBucket, ChartSummary } from '../../charts/chartSummary';
import { mergePreparedTension, validatePreparedTension } from './preparedTension.ts';
import { contributionAt, countryKey, tensionModel } from './tensionData.ts';
import { tensionDrillAt, tensionLookAt } from './tensionDrill.ts';
import { wireSummary } from '../../charts/chartSummary.fixture.ts';

const bucket: ChartBucket = { firstHour: 0,lastHour: 31,firstValue: 2,lastValue: 3,lastValueHour: 30,
  minHour: 9,minValue: -7,maxHour: 21,maxValue: 11,firstBreak: false,hasBreak: false };
const summary = (bucket: ChartBucket | null): ChartSummary => ({ from: 0,to: 31,width: 1,buckets: [bucket] });
const total = summary({ ...bucket,firstValue: 5,lastValue: 5,minValue: 5,maxValue: 5 });

test('prepared exact values and original holder provenance stay separate from dense geometry', () => {
  const data = validatePreparedTension({ branchId:'1',tensionScale:1000,contributionScale:100000,ceilingRaw:'10000000',warnings:[],events:[],available:true,
    reads:[{firstHour:20,lastHour:20,startsSegment:false,tensionRaw:'1000',sumRaw:'9007199254740993',unnamedRaw:'0'}],
    countries:[{tag:'A',slot:1,runs:[{firstHour:20,lastHour:20,startsSegment:false,valueRaw:'9007199254740993'}],identities:[
      {firstHour:0,lastHour:9,slot:1,countryId:10},{firstHour:20,lastHour:30,slot:1,countryId:20}]}],
    catalogue:[{key:'0',tag:'A',slot:1,firstHour:0}],exactHours:[20],summaries:{world:wireSummary(total),sum:wireSummary(total)} });
  assert.equal(data.countries[0].runs[0].valueRaw,'9007199254740993');
  const model = tensionModel(data,undefined,[],new Map(data.catalogue.map(country => [countryKey(country),country.firstHour])));
  assert.equal(tensionLookAt(model,countryKey(data.countries[0]),20,{firstHour:0,lastHour:30,gaps:[]}),null,'a later reached read cannot relabel the original holder');
  assert.deepEqual(data.summaries.world, { ...total, buckets: [{ ...total.buckets[0], absoluteTotal: '0' }] });
  assert.throws(() => validatePreparedTension({...data,summaries:{world:{...wireSummary(total),width:0}}}));
});

test('frame contributors join a changing catalogue without inventing historical zeroes or unnamed attribution', () => {
  const base = { branchId: '1', tensionScale: 1000, contributionScale: 100000, ceilingRaw: '10000000', warnings: [], events: [],
    available: false, countries: [], catalogue: [], summaries: {}, exactHours: [], reads: [], coverage: { firstHour: 0, lastHour: -1, complete: false }, pending: [{ firstHour: 0, lastHour: 20 }] };
  const current = validatePreparedTension({ ...base, available: true, preparing: ['events', 'historicalContributors'],
    reads: [{ firstHour: 20, lastHour: 20, startsSegment: true, tensionRaw: '100', sumRaw: '10000', unnamedRaw: null,
      preparing: ['historicalContributors', 'unnamedRaw', 'startsSegment'] }],
    countries: [{ tag: 'A', slot: 1, runs: [{ firstHour: 20, lastHour: 20, startsSegment: true, valueRaw: '6000' }], identities: [{ firstHour: 20, lastHour: 20, slot: 1, countryId: 10 }] },
      { tag: 'Old', slot: 9, runs: [], identities: [] }],
    catalogue: [{ key: JSON.stringify(['A', 1]), tag: 'A', slot: 1, firstHour: 20 }, { key: '0', tag: 'Old', slot: 9, firstHour: 0 }],
    preparingContributors: [{ hour: 20, slot: 9, valueRaw: '4000' }] });
  const merged = mergePreparedTension(validatePreparedTension(base), current);
  const model = tensionModel(merged);
  assert.equal(contributionAt(model, countryKey({ tag: 'Old', slot: 9 }), 20), null);
  const at = tensionDrillAt(model, 20)!;
  assert.equal(at.read.unnamedRaw, null);
  assert.equal(at.parts.find(part => part.preparing)?.raw, '4000');
  assert.equal(at.remainder, '0');
  const preparedRead = { ...current.reads[0], unnamedRaw: '0' };
  delete preparedRead.preparing;
  const complete = validatePreparedTension({ ...current, coverage: { firstHour: 0, lastHour: 20, complete: true }, pending: [], preparing: [], preparingContributors: [],
    reads: [preparedRead],
    countries: [current.countries[0], { ...current.countries[1], runs: [{ firstHour: 20, lastHour: 20, startsSegment: false, valueRaw: '4000' }] }] });
  const exact = mergePreparedTension(complete, current);
  assert.deepEqual(exact.reads, complete.reads);
  assert.deepEqual(exact.preparingContributors, []);
  assert.deepEqual(exact.preparing, []);
  assert.throws(() => validatePreparedTension({ ...current, reads: [{ ...current.reads[0], preparing: [] }] }));
});
