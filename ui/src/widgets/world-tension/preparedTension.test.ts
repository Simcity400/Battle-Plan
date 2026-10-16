import assert from 'node:assert/strict';
import test from 'node:test';
import type { ChartBucket, ChartSummary } from '../../charts/chartSummary';
import { validatePreparedTension } from './preparedTension.ts';
import { countryKey, tensionModel } from './tensionData.ts';
import { tensionLookAt } from './tensionDrill.ts';

const bucket: ChartBucket = { firstHour: 0,lastHour: 31,firstValue: 2,lastValue: 3,lastValueHour: 30,
  minHour: 9,minValue: -7,maxHour: 21,maxValue: 11,firstBreak: false,hasBreak: false };
const summary = (bucket: ChartBucket | null): ChartSummary => ({ from: 0,to: 31,width: 1,buckets: [bucket] });
const total = summary({ ...bucket,firstValue: 5,lastValue: 5,minValue: 5,maxValue: 5 });

test('prepared exact values and original holder provenance stay separate from dense geometry', () => {
  const data = validatePreparedTension({ branchId:'1',tensionScale:1000,contributionScale:100000,ceilingRaw:'10000000',warnings:[],events:[],available:true,
    reads:[{firstHour:20,lastHour:20,startsSegment:false,tensionRaw:'1000',sumRaw:'9007199254740993',unnamedRaw:'0'}],
    countries:[{tag:'A',slot:1,runs:[{firstHour:20,lastHour:20,startsSegment:false,valueRaw:'9007199254740993'}],identities:[
      {firstHour:0,lastHour:9,slot:1,countryId:10},{firstHour:20,lastHour:30,slot:1,countryId:20}]}],
    catalogue:[{key:'0',tag:'A',slot:1,firstHour:0}],exactHours:[20],summaries:{world:total,sum:total} });
  assert.equal(data.countries[0].runs[0].valueRaw,'9007199254740993');
  const model = tensionModel(data,undefined,[],new Map(data.catalogue.map(country => [countryKey(country),country.firstHour])));
  assert.equal(tensionLookAt(model,countryKey(data.countries[0]),20,{firstHour:0,lastHour:30,gaps:[]}),null,'a later reached read cannot relabel the original holder');
  assert.throws(() => validatePreparedTension({...data,summaries:{world:{...total,width:0}}}));
});
