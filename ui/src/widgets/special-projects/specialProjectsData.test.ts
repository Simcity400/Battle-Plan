import { test } from 'node:test';
import assert from 'node:assert/strict';
import type { CountryGroupReply, SpecialProjectsReply } from '../../generated/dashboard';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import { historyCard, historyTooltip, liveCards, observedAt, projectCountries, projectCountryHour, projectHeading, projectHistoryExtent, projectLanes, projectTooltip, validateSpecialProjectsReply } from './specialProjectsData.ts';
import { OPTIONS, definition } from './definition.ts';
import { resolveOptions } from '../../dashboard/options.ts';

const fixture = (): SpecialProjectsReply => ({ branchId: '1', tag: 'tag with spaces', unavailable: null, warnings: [],
  identities: [{firstHour:100,lastHour:199,slot:5,countryId:10},{firstHour:220,lastHour:300,slot:5,countryId:10}],
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
  const tip=projectTooltip(data,liveCards(data,120,240)[0]);
  assert.match(tip.rows.find(row=>row.label==='Started')!.value!,/^by /);
  assert.doesNotMatch(tip.note ?? '',/read once a day/);
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
    (data:SpecialProjectsReply)=>{data.identities[1].firstHour=190;},
  ]) {const data=fixture();mutate(data);assert.throws(()=>validateSpecialProjectsReply(data));}
});
test('last assignment is dated and known iterations with missing text are localisation unavailability', () => {
  const data=fixture(); data.presentations[0].scientistAsOf=99;
  data.iterationTexts['2']=null;
  const card=liveCards(data,120,240)[0];
  const tip=projectTooltip(data,card);
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
  assert.equal(projectTooltip(data,liveCards(data,139,240)[0]).rows.find(row=>row.label==='State')?.value,'Paused');
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

const countryGroups = (): CountryGroupReply => ({ branchId: '1', warnings: [], factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [],
  unobserved: { humans: [], factions: [], wars: [] }, humans: [{ tag: 'X', firstHour: 100, lastHour: 109 }, { tag: 'Y', firstHour: 110, lastHour: 114 }],
  countryIdentities: [
    { tag: 'X', identities: [{ firstHour: 100, lastHour: 109, slot: 1, countryId: 11 }, { firstHour: 120, lastHour: 150, slot: 2, countryId: 22 }] },
    { tag: 'Y', identities: [{ firstHour: 110, lastHour: 114, slot: 1, countryId: 11 }, { firstHour: 117, lastHour: 119, slot: 1, countryId: 11 }, { firstHour: 120, lastHour: 150, slot: 3, countryId: 33 }] },
  ],
});

test('manual project requests retain original roots and all their holders without promoting reused aliases', () => {
  const scope = projectCountries({ kind: 'tags', tags: ['X'] }, [], countryGroups(), 100, 150);
  assert.equal(scope.ready, true);
  assert.deepEqual(scope.roots, ['X']);
  assert.deepEqual(scope.tags, ['X', 'Y']);
  assert.deepEqual([...scope.rootsFor], [['X', ['X']], ['Y', ['X']]]);
  assert.equal(scope.match.matches('Y', 110), true);
  assert.equal(scope.match.matches('Y', 115), false, 'identity gaps acquire nobody');
  assert.equal(scope.match.matches('Y', 118), true);
  assert.equal(scope.match.matches('Y', 120), false, 'an unrelated later alias holder stays outside');
  assert.equal(scope.match.matches('X', 120), true, 'the second literal-tag holder remains selected');
  assert.deepEqual(scope.memberships.get('Y')!.map(run => [run.firstHour, run.lastHour]), [[110, 114], [117, 119]]);
  const aliasRoot = projectCountries({ kind: 'tags', tags: ['Y'] }, [], countryGroups(), 100, 150);
  assert.equal(aliasRoot.match.matches('X', 105), true, 'the original choice may select an earlier alias');
  assert.equal(aliasRoot.match.matches('X', 130), false, 'projected X is not another selection root');
  assert.equal(projectCountries({ kind: 'tags', tags: ['X'] }, [], undefined, 100, 150).ready, false);
});

test('smart project membership keeps its dated hour, end and ever rules', () => {
  const groups = countryGroups();
  const scope = (membership: 'hour' | 'end' | 'ever') => projectCountries({ kind: 'group', group: { type: 'humans' }, membership, aggregate: false }, [], groups, 100, 150);
  assert.equal(scope('hour').match.matches('Y', 118), false);
  assert.equal(scope('ever').match.matches('Y', 118), true);
  assert.equal(scope('ever').match.matches('Y', 120), false);
  assert.ok(scope('end').reason, 'the ended human group stays empty at the branch end');
  groups.humans.push({ tag: 'X', firstHour: 130, lastHour: 150 });
  assert.equal(scope('end').match.matches('X', 140), true);
  assert.equal(scope('end').match.matches('X', 105), false, 'end membership does not admit a previous holder');
  groups.unobserved.humans = [{ firstHour: 112, lastHour: 113 }];
  assert.equal(scope('hour').match.matches('Y', 112), false);
});

test('a comparison heading carries complete flag status at the selected holder date', () => {
  const data = fixture();
  assert.equal(projectCountryHour(data, 219), 199);
  assert.equal(projectCountryHour(data, 99), null);
  assert.equal(projectCountryHour(data, 250, [{ firstHour: 110, lastHour: 115 }]), 115);
  const labels: ReturnType<CountryLabel>[] = [{ name: 'Known', flag: 'data:image/png;base64,synthetic', flagPending: true },
    { name: '', flagPending: false, flagReason: 'The supplying flag read refused its native inputs.' }];
  for (const label of labels) {
    const heading = projectHeading('country', label, '');
    assert.equal(heading.identity?.label, label.name, 'known empty names stay empty');
    assert.deepEqual(heading.identity?.countryFlag, { flag: label.flag, flagPending: label.flagPending, flagReason: label.flagReason });
    assert.equal(heading.identity?.icon, undefined);
  }
});

test('membership cannot borrow a later project presentation or completion', () => {
  const data = fixture(), membership = [{ firstHour: 110, lastHour: 115 }];
  data.presentations.push({ ...data.presentations[0], name: 'Unselected later presentation' });
  data.projects[0].runs[1].presentation = 1;
  const lane = projectLanes(data, 199, membership)[0];
  assert.equal(lane.identity?.label, 'Mod project name');
  assert.equal(lane.spans[0].positive, false);
  assert.equal(historyCard(data, 'episode', 199, membership)?.run.state, 'running');
  assert.ok(!historyTooltip(data, 'episode', lane.spans[0], 112, 199, membership)?.rows.some(row => row.label === 'Completed'));
  assert.equal(projectLanes(data, 199, [])[0], undefined);
});

test('clipping a run into future membership cannot reveal its blueprint or menu before the playhead', () => {
  const data = fixture(), membership = [{ firstHour: 100, lastHour: 110 }, { firstHour: 180, lastHour: 190 }];
  data.projects = [{ ...data.projects[0], start: { hour: 140, exact: false }, runs: [{ ...data.projects[0].runs[0], firstHour: 140, lastHour: 200 }] }];
  assert.deepEqual(projectLanes(data, 150, membership), []);
  assert.equal(historyCard(data, 'episode', 150, membership), null);
  const later = projectLanes(data, 185, membership)[0];
  assert.deepEqual(later.spans.map(span => [span.firstHour, span.lastHour]), [[180, 185]]);
  assert.equal(historyCard(data, 'episode', 185, membership)?.run.firstHour, 180);
});

test('hourly group uncertainty cuts every History consumer without changing end or ever membership', () => {
  const data = fixture(), groups = countryGroups();
  groups.countryIdentities = [{ tag: data.tag, identities: data.identities }];
  groups.humans = [{ tag: data.tag, firstHour: 100, lastHour: 199 }];
  groups.unobserved.humans = [{ firstHour: 112, lastHour: 113 }];
  const scope = (membership: 'hour' | 'end' | 'ever') => projectCountries({ kind: 'group', group: { type: 'humans' }, membership, aggregate: false }, [], groups, 100, 199);
  const hourly = scope('hour'), membership = hourly.memberships.get(data.tag)!;
  assert.deepEqual(membership.map(span => [span.firstHour, span.lastHour]), [[100, 111], [114, 199]]);
  assert.deepEqual(scope('ever').memberships.get(data.tag)!.map(span => [span.firstHour, span.lastHour]), [[100, 199]]);
  assert.deepEqual(scope('end').memberships.get(data.tag)!.map(span => [span.firstHour, span.lastHour]), [[100, 199]]);
  assert.equal(projectCountryHour(data, 113, membership), 111);
  assert.equal(historyCard(data, 'episode', 113, membership)?.run.lastHour, 111);
  const lane = projectLanes(data, 120, membership)[0];
  assert.deepEqual(lane.spans.map(span => [span.firstHour, span.lastHour]), [[100, 111], [114, 120]]);
  assert.match(historyTooltip(data, 'episode', null, 112, 120, membership)?.note ?? '', /not recorded at this hour/);
  data.projects = [{ ...data.projects[0], start: { hour: 112, exact: false }, runs: [{ ...data.projects[0].runs[0], firstHour: 112 }] }];
  assert.equal(historyCard(data, 'episode', 113, membership), null);
  assert.deepEqual(projectLanes(data, 113, membership), []);
});

test('History keeps its campaign axis by default and grows only from an available playhead', () => {
  assert.equal(resolveOptions(OPTIONS, { view: 'history' }).grow, false);
  assert.deepEqual(projectHistoryExtent(100, 300, 150, false), [100, 301]);
  assert.deepEqual(projectHistoryExtent(100, 300, 150, true), [100, 150]);
  assert.deepEqual(projectHistoryExtent(100, 300, null, true), [100, 301]);
  assert.deepEqual(projectHistoryExtent(120, 180, null, true), [120, 181]);
  assert.deepEqual(projectHistoryExtent(100, 300, 100, true), [100, 101]);
});

test('5000 project choices reuse the native catalogue without repeating every root in each request', () => {
  const groups = countryGroups();
  groups.countryIdentities = Array.from({ length: 5000 }, (_, index) => ({ tag: `mod-country-${index}`,
    identities: [{ firstHour: 100, lastHour: 300, slot: index, countryId: index + 5000 }] }));
  const roots = groups.countryIdentities.map(history => history.tag);
  const scope = projectCountries({ kind: 'tags', tags: roots }, [], groups, 100, 300);
  assert.equal(scope.tags.length, 5000);
  assert.equal([...scope.rootsFor.values()].reduce((count, roots) => count + roots.length, 0), 5000);
  assert.equal(scope.match.matches('mod-country-4999', 300), true);
});
