import assert from 'node:assert/strict';
import test from 'node:test';
import type { CountryFactoriesReply } from '../../generated/dashboard';
import { createServer } from 'vite';

test('prepared factory pages preserve signed fields, supplying reads and 5000 native country identities', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { mergeFactories, factoriesGeometry } = await vite.ssrLoadModule('/ui/src/widgets/factories/preparedFactories.ts');
    const { validateFactoriesReply, factoryModel, factoryReadAt } = await vite.ssrLoadModule('/ui/src/widgets/factories/factoriesData.ts');
    const make = (hour: number) => ({ branchId: '1', groups: [], fields: [{ group: 'civilian', part: 'total' }], warnings: [],
      series: Array.from({ length: 5000 }, (_, id) => ({ tag: `mod ${id}`, identities: [{ slot: id, countryId: id + 1, firstHour: 0, lastHour: 9 }],
        membership: null, segments: [{ firstHour: 0, lastHour: 9 }], reads: [{ firstHour: hour, lastHour: hour, readHour: hour, hourly: true }],
        runs: [{ firstHour: hour, lastHour: hour, startsSegment: hour === 0, values: [-1] }] })) });
    const joined = validateFactoriesReply(mergeFactories([make(0), make(1)])) as CountryFactoriesReply;
    assert.equal(joined.series.length, 5000); assert.deepEqual(joined.series[4999].runs.map(run => run.values), [[-1]]);
    const model=factoryModel(joined);
    assert.equal(factoryReadAt(model,'mod 4999',0),0);assert.equal(factoryReadAt(model,'mod 4999',1),1,'coalesced hourly provenance preserves every supplying date');
    assert.equal(joined.series[4999].identities?.[0].countryId, 5000);
    assert.deepEqual(factoriesGeometry(joined), joined);
  } finally { await vite.close(); }
});

test('exact factory reads replace coarse values and holder evidence without overlapping lookups',async()=>{
  const vite=await createServer({configFile:false,optimizeDeps:{noDiscovery:true},server:{middlewareMode:true,watch:null,hmr:false,ws:false}});
  try {
    const {mergeFactories}=await vite.ssrLoadModule('/ui/src/widgets/factories/preparedFactories.ts');
    const {validateFactoriesReply,factoryModel,factoryReadAt}=await vite.ssrLoadModule('/ui/src/widgets/factories/factoriesData.ts');
    const make=(firstHour:number,lastHour:number,value:number,id:number)=>({branchId:'1',groups:[],fields:[{group:'civilian',part:'total'}],warnings:[],series:[{
      tag:'mod holder',identities:[{slot:0,countryId:id,firstHour,lastHour}],segments:[{firstHour,lastHour}],membership:null,
      reads:[{firstHour,lastHour,readHour:firstHour,hourly:firstHour===lastHour}],runs:[{firstHour,lastHour,startsSegment:true,values:[value]}]}]});
    const coarse=make(0,9,1,100),exact=make(5,5,99,200);
    const merged=validateFactoriesReply(mergeFactories([coarse,exact]));
    assert.deepEqual(merged.series[0].runs.map((run:{firstHour:number;lastHour:number;values:number[]})=>[run.firstHour,run.lastHour,run.values[0]]),[[0,4,1],[5,5,99],[6,9,1]]);
    assert.deepEqual(merged.series[0].identities.map((run:{countryId:number;firstHour:number;lastHour:number})=>[run.firstHour,run.lastHour,run.countryId]),[[0,4,100],[5,5,200],[6,9,100]]);
    assert.equal(factoryReadAt(factoryModel(merged),'mod holder',5),5);
    assert.equal(factoryReadAt(factoryModel(merged),'mod holder',6),0);
    assert.equal(coarse.series[0].runs[0].lastHour,9,'joining never mutates a retained reply');
    const missing={...make(5,5,99,200),summaries:[{tag:'mod holder',summary:{from:5,to:5,width:1,buckets:[null],runs:[]}}]};
    missing.series[0].runs=[];missing.series[0].reads=[];
    const without=validateFactoriesReply(mergeFactories([coarse,missing]));
    assert.equal(factoryReadAt(factoryModel(without),'mod holder',5),null,'an exact missing read cannot reuse its rough estimate');
  } finally {await vite.close();}
});

test('paged factory summaries join as factory replies, each country and its summary line once', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    await vite.ssrLoadModule('/ui/src/widgets/factories/preparedFactories.ts');
    const { pageJoin } = await vite.ssrLoadModule('/ui/src/dashboard/pageJoins.ts');
    const { validateFactoriesReply } = await vite.ssrLoadModule('/ui/src/widgets/factories/factoriesData.ts');
    const summary = { from: 0, to: 9, width: 1, buckets: [null] };
    const page = (tags: string[], cursor: string | null, last: number) => ({ branchId: '1', groups: [], fields: [{ group: 'civilian', part: 'total' }], warnings: [`w ${last}`],
      coverage: { lastHour: last }, cursor, summaries: tags.map(tag => ({ tag, summary, parts: [] })),
      series: tags.map(tag => ({ tag, identities: [{ slot: tag.length, countryId: tag.length + 1, firstHour: 0, lastHour: 9 }], membership: null, segments: [{ firstHour: 0, lastHour: 9 }],
        reads: [{ firstHour: 0, lastHour: 9, readHour: 0, hourly: false }], runs: [{ firstHour: 0, lastHour: 9, startsSegment: true, values: [tag.length] }] })) });
    const join = pageJoin('country-factories');
    const joined = join([page(['mod a', 'mod b'], '2', 5), page(['mod ccc'], null, 9)]);
    const reply = validateFactoriesReply(joined) as CountryFactoriesReply;
    assert.deepEqual(reply.series.map(series => series.tag), ['mod a', 'mod b', 'mod ccc']);
    assert.deepEqual(reply.series[2].runs[0].values, [7]);
    assert.deepEqual(reply.fields, [{ group: 'civilian', part: 'total' }]);
    assert.deepEqual(joined.summaries.map((line: { tag: string }) => line.tag), ['mod a', 'mod b', 'mod ccc']);
    assert.deepEqual(joined.coverage, { lastHour: 9 }, 'the last page read is the joined preparation');
    assert.equal(joined.cursor, null);
    assert.deepEqual(join([page(['mod a'], '1', 5), page(['mod a'], null, 5)]).summaries.length, 1);
  } finally { await vite.close(); }
});
