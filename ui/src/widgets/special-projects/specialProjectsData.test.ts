import { test } from 'node:test';
import assert from 'node:assert/strict';
import type { SpecialProjectsReply } from '../../generated/dashboard';
import { historyTooltip, liveCards, observedAt, projectLanes, projectTooltip, validateSpecialProjectsReply } from './specialProjectsData.ts';
import { OPTIONS, definition } from './definition.ts';
import { resolveOptions } from '../../dashboard/options.ts';

const fixture = (): SpecialProjectsReply => ({ branchId: '1', tag: 'tag with spaces', unavailable: null, warnings: [],
  available: [{firstHour:100,lastHour:199},{firstHour:220,lastHour:300}], readHours:[100,124,148,172,196,220,244,268,292],
  iterationTexts:{2:'Mod iterations: 2',3:'Mod iterations: 3'},
  people:[{assigned:true,id:77,name:'Mod scientist',portrait:null,missing:'Portrait not recorded.'}],
  presentations:[{name:'Mod project name',specialization:'mod_kind',specializationName:'Mod science',icon:null,iconMissing:'Missing mod icon',scientist:0,scientistAsOf:null}],
  projects:[{id:'episode',start:{hour:100,exact:false},runs:[
    {firstHour:100,lastHour:147,presentation:0,progress:17,state:'running',cancellation:null,completion:null,iterations:2},
    {firstHour:148,lastHour:199,presentation:0,progress:100,state:'completed',cancellation:null,completion:{hour:140,exact:true},iterations:3},
  ]}],
});

test('daily reads step without interpolation and future completion stays hidden', () => {
  const data = fixture();
  assert.equal(liveCards(data,147,240)[0].run.progress,17);
  assert.equal(liveCards(data,147,240)[0].run.completion,null);
  assert.equal(liveCards(data,148,240)[0].run.completion?.hour,140);
  assert.equal(projectLanes(data,147)[0].spans[0].positive,false);
  assert.equal(projectLanes(data,148)[0].spans[0].positive,true);
});
test('missing hours are unavailable and completed retention is fixed game days', () => {
  const data = fixture();
  assert.equal(observedAt(data,200),false);
  assert.equal(liveCards(data,200,240).length,0);
  assert.equal(liveCards(data,148,0).length,0);
  assert.equal(liveCards(data,163,1).length,1);
  assert.equal(liveCards(data,164,1).length,0);
});
test('history caps at the completion event and leaves unrecorded intervals empty', () => {
  const data = fixture();
  const lane = projectLanes(data,199)[0];
  assert.equal(lane.identity?.tile,'blueprint');
  assert.equal(lane.label,'Mod scientist');
  assert.equal(lane.spans[0].lastHour,139);
  assert.equal(lane.spans[0].endLabel,'✓');
  data.projects[0].runs[1].state='running'; data.projects[0].runs[1].completion=null;
  data.projects[0].runs.push({...data.projects[0].runs[0],firstHour:220,lastHour:230});
  assert.deepEqual(projectLanes(data,300)[0].spans.map(span=>[span.firstHour,span.lastHour]),[[100,199],[220,230]]);
});
test('tooltips date reads and expose unknown art instead of assuming zero', () => {
  const data=fixture(); data.projects[0].runs[0].iterations=null; data.iterationTexts['2']=null;
  const tip=projectTooltip(data,liveCards(data,120,240)[0],120);
  assert.match(tip.rows.find(row=>row.label==='Started')!.value!,/^by /);
  assert.match(tip.note!,/read once a day/);
  assert.match(tip.note!,/iterations unknown/);
  assert.match(tip.note!,/Missing mod icon/);
});
test('validator rejects bad references, overlaps, missing and invented percentages', () => {
  assert.equal(validateSpecialProjectsReply(fixture()).tag,'tag with spaces');
  for(const mutate of [
    (data:SpecialProjectsReply)=>{data.presentations[0].scientist=1;},
    (data:SpecialProjectsReply)=>{data.projects[0].runs[1].firstHour=100;},
    (data:SpecialProjectsReply)=>{data.projects[0].runs[0].progress=101;},
    (data:SpecialProjectsReply)=>{data.readHours.push(100);},
    (data:SpecialProjectsReply)=>{data.projects[0].runs[1].completion!.hour=200;},
    (data:SpecialProjectsReply)=>{data.projects[0].runs[1].state='running';},
  ]) {const data=fixture();mutate(data);assert.throws(()=>validateSpecialProjectsReply(data));}
});
test('last assignment is dated and known iterations with missing text are localisation unavailability', () => {
  const data=fixture(); data.presentations[0].scientistAsOf=99;
  data.iterationTexts['2']=null;
  const card=liveCards(data,120,240)[0];
  const tip=projectTooltip(data,card,120);
  assert.match(tip.rows.find(row=>row.label==='Scientist')!.value!,/^Last assigned/);
  assert.match(tip.note!,/Last assignment recorded/);
  assert.match(tip.note!,/Iteration localisation unavailable/);
  assert.doesNotMatch(tip.note!,/iterations unknown/);
});
test('presentation has no content count cap at 5000 projects and defaults are saved options', () => {
  const data=fixture(); data.projects=Array.from({length:5000},(_,id)=>({...data.projects[0],id:`mod-${id}`}));
  assert.equal(validateSpecialProjectsReply(data).projects.length,5000);
  assert.equal(liveCards(data,120,240).length,5000);
  assert.equal(projectLanes(data,120).length,5000);
  assert.equal(resolveOptions(OPTIONS,{}).completedRetentionDays,240);
  assert.equal(resolveOptions(OPTIONS,{view:'history'}).view,'history');
  assert.equal(definition.defaultTime,'campaign');
});

test('paused cards are named and cancelled episodes end by observation without a completion tick', () => {
  const data = fixture();
  data.projects[0].runs = [{...data.projects[0].runs[0],state:'paused',lastHour:139}, {...data.projects[0].runs[0],firstHour:140,lastHour:199,state:'cancelled',cancellation:{hour:140,exact:false}}];
  assert.equal(liveCards(data,139,240).length,1);
  assert.equal(projectTooltip(data,liveCards(data,139,240)[0],139).rows.find(row=>row.label==='State')?.value,'Paused');
  assert.equal(liveCards(data,140,240).length,0);
  const lane=projectLanes(data,199)[0];
  assert.equal(lane.muted,true);
  assert.equal(lane.spans.at(-1)?.lastHour,139);
  assert.equal(lane.spans.at(-1)?.positive,false);
  assert.equal(lane.spans.at(-1)?.endLabel,undefined);
});
test('history tooltip resolves hovered observation and span, bounded by playhead; gaps stay unknown', () => {
  const data=fixture();
  const span=projectLanes(data,199)[0].spans[0];
  assert.equal(historyTooltip(data,'episode',span,120,199)?.rows.find(row=>row.label==='Progress')?.value,'17%');
  assert.ok(historyTooltip(data,'episode',span,120,199)?.rows.some(row=>row.label==='Completed'));
  assert.equal(historyTooltip(data,'episode',{...span,lastHour:199},170,130)?.rows.find(row=>row.label==='Progress')?.value,'17%');
  assert.match(historyTooltip(data,'episode',null,205,300)?.note ?? '',/not recorded at this hour/);
});
test('group membership clips timeline bars and unassigned is an explicit flag', () => {
  const data=fixture();
  assert.deepEqual(projectLanes(data,199,[{firstHour:110,lastHour:115}])[0].spans.map(span=>[span.firstHour,span.lastHour]),[[110,115]]);
  data.people[0]={assigned:false,id:null,name:null,portrait:null,missing:'A translated message'};
  assert.equal(projectLanes(data,120)[0].label,'Unassigned');
});
