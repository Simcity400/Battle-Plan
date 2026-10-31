import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';
import type { FuelRead, FuelReply } from '../../generated/dashboard';
import type { FuelSummaries } from './preparedFuel';
import { wireSummary } from '../../charts/chartSummary.fixture.ts';

test('prepared fuel keeps exact reached integers and supplying dates separate from summaries', async()=>{
  const server=await createServer({configFile:false,optimizeDeps:{noDiscovery:true},server:{middlewareMode:true,watch:null,hmr:false,ws:false},appType:'custom'});
  try {
    const {validateFuelStates,reachedFuel,validateFuelSummaries,fuelSummaryRuns,fuelSummaryStrip}=await server.ssrLoadModule('/ui/src/widgets/fuel/preparedFuel.ts');
    const label={key:'synthetic',template:null,arguments:['NUM'],reason:'fixture'};
    const metadata:FuelReply={branchId:'1',tag:'mod/tag',quantityScale:32768,gainScale:100000,dayHours:24,detailHours:720,
      consumerNames:[null,null,null],gainLabels:[label,label,label,label],transferLabels:[label,label,label],reads:[],warnings:[]};
    const read:FuelRead={country:'7',countryInstance:null,continuityStartHour:null,firstHour:100,lastHour:123,startsSegment:true,
      fuelRaw:'9223372036854775807',capacityRaw:'9223372036854775807',lendLeaseRaw:'0',day:null,leftOut:'ownershipNotRecorded',awardsReason:'fixture'};
    const states=validateFuelStates({branchId:'1',replies:[metadata],states:[{tag:metadata.tag,hour:105,recorded:true,fuel:read,consumption:null,consumptionSegment:null},
      {tag:metadata.tag,hour:120,recorded:true,fuel:read,consumption:null,consumptionSegment:null}]});
    assert.deepEqual(reachedFuel(states)[0].reads,[read]);
    assert.equal(reachedFuel(states)[0].reads[0].fuelRaw,'9223372036854775807');
    assert.throws(()=>validateFuelStates({...states,states:[{...states.states[0],hour:124}]}),/cover/);
    const summary={from:100,to:123,width:30,buckets:[],runs:[{firstHour:100,lastHour:123,value:32768,breakBefore:true}]};
    assert.deepEqual(validateFuelSummaries({branchId:'1',replies:[metadata],series:[{tag:metadata.tag,measure:'fuel',summary:wireSummary(summary)}]}).series[0].summary,summary);
    assert.deepEqual(fuelSummaryRuns(summary,32768),[{firstHour:100,lastHour:123,value:1,breakBefore:true}]);
    const gain={...summary,runs:[{firstHour:100,lastHour:100,value:150000,breakBefore:true},{firstHour:124,lastHour:124,value:200000,breakBefore:false}]};
    const burn={...summary,runs:[{firstHour:100,lastHour:100,value:32768,breakBefore:true}]};
    assert.deepEqual(fuelSummaryStrip(gain,burn,metadata),[{firstHour:76,lastHour:100,up:1.5,down:1},{firstHour:100,lastHour:124,up:2,down:0}]);
  }finally{await server.close();}
});

test('dense fuel geometry leaves missing coverage disconnected and keeps both strip peak dates',async()=>{
  const server=await createServer({configFile:false,optimizeDeps:{noDiscovery:true},server:{middlewareMode:true,watch:null,hmr:false,ws:false},appType:'custom'});
  try {
    const {fuelSummaryRuns,fuelSummaryStrip}=await server.ssrLoadModule('/ui/src/widgets/fuel/preparedFuel.ts');
    const bucket={firstHour:100,lastHour:200,firstValue:10,lastValue:20,lastValueHour:199,minHour:101,minValue:2,maxHour:198,maxValue:70,firstBreak:true,hasBreak:true};
    const summary={from:100,to:200,width:1,buckets:[bucket]};
    const runs=fuelSummaryRuns(summary,1);
    assert.ok(runs.every((run:{firstHour:number;lastHour:number;breakBefore:boolean})=>run.firstHour===run.lastHour && run.breakBefore));
    const burn={...summary,buckets:[{...bucket,maxHour:170,maxValue:90}]};
    const bars=fuelSummaryStrip(summary,burn,{dayHours:24,gainScale:1,quantityScale:1});
    assert.deepEqual(bars.map((bar:{lastHour:number;up:number;down:number})=>[bar.lastHour,bar.up,bar.down]),[[170,0,90],[198,70,0]]);
  }finally{await server.close();}
});

test('progressive fuel amounts keep exact integers and leave completed-day accounting pending', async()=>{
  const server=await createServer({configFile:false,optimizeDeps:{noDiscovery:true},server:{middlewareMode:true,watch:null,hmr:false,ws:false},appType:'custom'});
  try {
    const {validateFuelStates,scaledFuelSummary}=await server.ssrLoadModule('/ui/src/widgets/fuel/preparedFuel.ts');
    const {fuelTooltips,formatFuel}=await server.ssrLoadModule('/ui/src/widgets/fuel/fuelData.ts');
    const label={key:'fixture',template:null,arguments:['NUM'],reason:'fixture'};
    const reply={branchId:'1',tag:'tail only',quantityScale:32768,gainScale:100000,dayHours:24,detailHours:720,
      consumerNames:[null,null,null],gainLabels:[label,label,label,label],transferLabels:[label,label,label],reads:[],warnings:[]};
    const state={tag:reply.tag,hour:29,recorded:true,approximate:false,fuel:null,consumption:null,consumptionSegment:null,
      fuelRaw:'9223372036854775807',capacityRaw:'9223372036854775807'};
    const data=validateFuelStates({branchId:'1',replies:[reply],states:[state]});
    assert.equal(data.states[0].fuelRaw,state.fuelRaw);
    assert.equal(data.states[0].fuel,null,'a scalar sample must never manufacture a completed-day read');
    const tip=fuelTooltips(reply,[],29,'Tail only',{fuel:'red',capacity:'grey',gained:'green',burned:'blue'},5,state)[0];
    assert.equal(tip.rows.find((row:{label:string})=>row.label==='Fuel').value,formatFuel(state.fuelRaw,32768,5));
    assert.equal(tip.rows.find((row:{label:string})=>row.label==='Gained and burned').value,'Still being prepared');
    const summary={from:0,to:29,width:30,buckets:[],runs:[],approximate:[{firstHour:2,lastHour:29}]};
    assert.deepEqual(scaledFuelSummary(summary,32768).approximate,summary.approximate,'shared drawing receives the original approximate spans');
    assert.throws(()=>validateFuelStates({branchId:'1',replies:[reply],states:[{...state,fuelRaw:123}]}),/amount/);
  } finally {await server.close();}
});

test('5000 distinct country endpoints stay local and use one bounded exact query', async () => {
  const server = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false }, appType: 'custom' });
  try {
    const { fuelReadRequests, fuelStateIndex, validateFuelStates } = await server.ssrLoadModule('/ui/src/widgets/fuel/preparedFuel.ts');
    const { preparedSeriesQuery } = await server.ssrLoadModule('/ui/src/dashboard/preparedSeries.ts');
    const summaries: FuelSummaries = { branchId: '1', replies: [], series: [] };
    for (let index = 0; index < 5000; index++) {
      for (const measure of ['fuel', 'capacity']) {
        const hour = 10_000 + 2 * index + (measure === 'capacity' ? 1 : 0);
        summaries.series.push({ tag: `mod/country/${index}`, measure, summary: { from: 0, to: 30_000, width: 1, buckets: [{
          firstHour: hour, lastHour: hour, firstValue: 1, lastValue: 1, lastValueHour: hour,
          minHour: hour, minValue: 1, maxHour: hour, maxValue: 1, firstBreak: true, hasBreak: false,
        }] } });
      }
    }
    const two = fuelReadRequests(['mod/country/0', 'mod/country/1'], [30_000, 30_000], summaries);
    assert.deepEqual(two, [{ tag: 'mod/country/0', hour: 10_000 }, { tag: 'mod/country/0', hour: 10_001 }, { tag: 'mod/country/0', hour: 30_000 },
      { tag: 'mod/country/1', hour: 10_002 }, { tag: 'mod/country/1', hour: 10_003 }, { tag: 'mod/country/1', hour: 30_000 }]);
    const tags = Array.from({ length: 5000 }, (_, index) => `mod/country/${index}`);
    const reads = fuelReadRequests(tags, [30_000, 30_000], summaries);
    assert.equal(reads.length, 3 * tags.length, 'one reached date and two local endpoints per country');
    const label = { key: 'fixture', template: null, arguments: ['NUM'], reason: 'fixture' };
    let calls = 0;
    const client = { async preparedQuery(_source: unknown, product: string, params: { reads: typeof reads }) {
      assert.equal(++calls, 1, 'a non-null exact reply ends the real shared query');
      assert.equal(product, 'series-state'); assert.equal(params.reads.length, 15_000);
      return { branchId: '1', replies: tags.map(tag => ({ branchId: '1', tag, quantityScale: 32768, gainScale: 100000,
        dayHours: 24, detailHours: 720, consumerNames: [null, null, null], gainLabels: [label, label, label, label],
        transferLabels: [label, label, label], reads: [], warnings: [] })),
      states: params.reads.map((read: { tag: string; hour: number }) => ({ ...read, recorded: false, fuel: null, consumption: null, consumptionSegment: null })) };
    } };
    const data = validateFuelStates(await preparedSeriesQuery(client, { id: 'source', revision: '1' }, 'series-state',
      { branchId: '1', family: { kind: 'country-fuel' }, reads }, new AbortController().signal));
    assert.equal(fuelStateIndex(data, reads).size, 5000); assert.equal(calls, 1);
    assert.throws(() => fuelStateIndex({ ...data, states: [] }, reads), /did not advance/);
    assert.throws(() => fuelStateIndex(data, [{ tag: tags[0], hour: 30_001 }]), /did not advance/, 'a stale reply cannot advance the next parity page');
    assert.throws(() => fuelStateIndex({ ...data, states: [data.states[0], data.states[0]] }, [reads[0]]), /repeats/);
  } finally { await server.close(); }
});
