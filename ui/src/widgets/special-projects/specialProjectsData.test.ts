import { countryKey } from '../../dashboard/countryKey.ts';
import { panelStructure, panelValues } from '../../draw/panels/views.ts';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deepFreeze } from '../../test-support/frozen.ts';
import type { CountryGroupReply, ProjectEntry, ProjectRuntimeDetails, ProjectStatus, SpecialProjectDetails, SpecialProjectsReply } from '../../generated/dashboard';
import type { CountryLabelValue } from '../../dashboard/countryLabelData.ts';
import { closedLane, completedWhenFirstRecorded, completions, currentKey, currentProjects, earlierBound, heldThrough, historyCard, historyPicture, historyProject, historyStructure, historyUnrecorded, historyTooltip, iterationCount, observedAt, projectCountries, projectCountryHour, projectHeading, projectHistory, projectHistoryExtent, projectStructure, projectTooltip, readLabel, runningValues, runtimeProject, shownCard, statusRead, episodeBound, joinDue, validateSpecialProjectsReply } from './specialProjectsData.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { OPTIONS, definition } from './definition.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import { dayLabel } from '../../charts/draw.ts';
import { effectsStructure } from './projectDetailsData.ts';

test('project effects render again when their game text changes, never only because the hour moved', () => {
  const record = (hour: number, amount: number) => ({ hour, readHour: 12, pending: null, warnings: [], unavailable: null,
    output: { runs: [{ kind: 'text' as const, text: 'Amount ' + amount, tone: null }], refusals: { 'Unknown effect': 1 } } });
  assert.equal(effectsStructure(record(12, 3)), effectsStructure(record(24, 3)));
  assert.notEqual(effectsStructure(record(24, 3)), effectsStructure(record(24, 8)));
});

const marker = (date: number, readHour: number, counted = true): ProjectEntry => ({ ordinal: 3, date, completion: true, readHour, counted, presentation: counted ? 0 : null });

const fixture = (): SpecialProjectsReply => ({ branchId: '1', tag: 'tag with spaces', unavailable: null, warnings: [],
  identities: [{firstHour:100,lastHour:199,slot:5,countryId:10},{firstHour:220,lastHour:300,slot:5,countryId:10}],
  available: [{firstHour:100,lastHour:199},{firstHour:220,lastHour:300}], partial: [], unresolved: [],
  iterationTexts:{2:'Mod iterations: 2',3:'Mod iterations: 3'},
  people:[{assigned:true,id:77,name:'Mod scientist',portrait:null,missing:'Portrait not recorded.'}],
  presentations:[{name:'Mod project name',specialization:'mod_kind',specializationName:'Mod science',icon:null,iconMissing:'Missing mod icon',scientist:0,lastScientist:null}],
  reads:[],
  projects:[{id:'episode',owner:9,local:3,firstRead:100,firstState:'running',statuses:[
    {firstHour:100,lastHour:147,readHour:100,presentation:0,progress:17,state:'running',iterations:2,scientistRead:100,lastScientistRead:null},
    {firstHour:148,lastHour:199,readHour:148,presentation:0,progress:100,state:'completed',iterations:3,scientistRead:100,lastScientistRead:null},
  ],entries:[marker(140,148)],attempts:[]}],
});

test('a dated project packet reveals its own rows and warnings', () => {
  const data = fixture();
  assert.notEqual(panelStructure('special-projects', data, 147), panelStructure('special-projects', data, 148));
  const before = panelStructure('special-projects', data, 147);
  const renamed = fixture();
  renamed.presentations[0].name = 'New name in another seed';
  assert.notEqual(panelStructure('special-projects', renamed, 147), before);
  data.warnings.push('Unknown project state');
  assert.notEqual(panelStructure('special-projects', data, 147), before);
});

test('an exact project read derives its structure from the sampled hour', () => {
  const data = fixture();
  const stale = currentKey(data, 147);
  const values = panelValues('special-projects', data, null, 148, stale);
  assert.equal(values?.get('episode'), 'Mod project name');
});

test("statuses step without interpolation, and a completion's date is a history entry, never the status's", () => {
  const data = fixture();
  assert.equal(currentProjects(data,147).running[0].run.progress,17);
  assert.equal(currentProjects(data,148).completed[0].run.state,'completed');
  const done = currentProjects(data,148).completed[0].run;
  assert.equal(projectTooltip(data,currentProjects(data,148).completed[0],148).rows.find(row=>row.label==='State')?.value,'Completed', 'K4: the status says Completed and carries no date');
  assert.ok(!('completion' in done));
  assert.deepEqual(completions(data.projects[0],147).map(entry=>entry.date),[], 'a completion first listed at 148 is not shown at 147');
  assert.deepEqual(completions(data.projects[0],148).map(entry=>entry.date),[140], 'its dated history entry is');
  assert.equal(projectHistory(fixture(),147).lanes[0].spans[0].positive,false);
  assert.equal(projectHistory(fixture(),148).lanes[0].spans.at(-1)?.positive,true);
});
test('missing hours are unavailable and completed projects stay listed without a retention period', () => {
  const data = fixture();
  assert.equal(observedAt(data,200),false);
  assert.deepEqual(currentProjects(data,200),{ running: [], completed: [], earlier: [] });
  assert.equal(currentProjects(data,148).completed.length,1);
  assert.equal(currentProjects(data,199).completed.length,1, 'a completion long past still lists');
});
test('history lanes split at progress steps and leave unrecorded intervals empty', () => {
  const data = fixture();
  const lane = projectHistory(data,199).lanes[0];
  assert.equal(lane.identity?.tile,'blueprint');
  assert.equal(lane.label,'Mod scientist');
  assert.equal(lane.names,true);
  assert.deepEqual(lane.spans.map(span=>[span.firstHour,span.lastHour,span.label,span.positive]),[[100,147,'17%',false],[148,199,'100%',true]]);
  assert.equal(lane.spans[1].shade,undefined, 'a completed bar keeps the positive fill');
  data.projects[0].statuses.push({...data.projects[0].statuses[0],firstHour:220,lastHour:230});
  const later = projectHistory(data,300).lanes[0].spans;
  assert.deepEqual(later.map(span=>[span.firstHour,span.lastHour,span.label,span.shade]),[[100,147,'17%',0.17],[148,199,'100%',undefined],[220,230,'17%',0.17]]);
});
test('tooltips keep the first read and progress without artwork or table diagnostics', () => {
  const data=fixture(); data.projects[0].statuses[0].iterations=null; data.iterationTexts['2']=null;
  const tip=projectTooltip(data,currentProjects(data,120).running[0],120);
  assert.equal(tip.rows.find(row=>row.label==='First recorded')!.value, dayLabel(100));
  assert.equal(tip.note, undefined);
  assert.equal(tip.description, undefined);
  assert.equal(tip.subtitle, 'Mod science');
  assert.equal(tip.rows.find(row=>row.label==='Progress')?.value, '17%');
});
test('validator rejects bad references, overlaps, missing and invented percentages', () => {
  assert.equal(validateSpecialProjectsReply(fixture()).tag,'tag with spaces');
  for(const mutate of [
    (data:SpecialProjectsReply)=>{data.presentations[0].scientist=1;},
    (data:SpecialProjectsReply)=>{data.presentations[0].lastScientist=0;},
    (data:SpecialProjectsReply)=>{data.projects[0].statuses[1].firstHour=100;},
    (data:SpecialProjectsReply)=>{data.projects[0].statuses[0].progress=101;},
    (data:SpecialProjectsReply)=>{data.available[1].firstHour=150;},
    (data:SpecialProjectsReply)=>{data.projects[0].statuses[1].progress=90;},
    (data:SpecialProjectsReply)=>{data.projects[0].statuses[1].readHour=150;},
    (data:SpecialProjectsReply)=>{data.projects[0].firstRead=120;},
    (data:SpecialProjectsReply)=>{data.projects[0].entries[0].date=200;},
    (data:SpecialProjectsReply)=>{data.projects[0].entries[0].completion=false;},
    (data:SpecialProjectsReply)=>{data.identities[1].firstHour=190;},
    (data:SpecialProjectsReply)=>{data.reads=[{owner:9,firstHour:100,lastHour:120,readHour:101}];},
    (data:SpecialProjectsReply)=>{data.reads=[{owner:9,firstHour:100,lastHour:120,readHour:100},{owner:9,firstHour:110,lastHour:130,readHour:110}];},
  ]) {const data=fixture();mutate(data);assert.throws(()=>validateSpecialProjectsReply(data));}
});
test('the scientist shows its own program read, and the last observed one apart from no one or unknown', () => {
  const data=fixture();
  data.people.push({assigned:null,id:null,name:null,portrait:null,missing:'Program 7 was not read in the day up to this hour.'});
  data.presentations[0]={...data.presentations[0],scientist:1,lastScientist:0};
  data.projects[0].statuses=data.projects[0].statuses.map(run=>({...run,scientistRead:null,lastScientistRead:99}));
  const card=currentProjects(data,120).running[0];
  const tip=projectTooltip(data,card,120);
  assert.equal(tip.rows.find(row=>row.label==='Scientist')!.value, 'Scientist not recorded');
  assert.equal(tip.rows.find(row=>row.label==='Last observed')!.value, `Mod scientist (read ${formatDateTime(99)})`);
  assert.equal(tip.description, 'Mod iterations: 2');
  assert.equal(iterationCount(card.run.iterations), '2 iterations');
  assert.equal(iterationCount(1), '1 iteration');
  assert.equal(iterationCount(null), 'Iterations not recorded');
  assert.equal(projectTooltip(data,currentProjects(data,160).completed[0],160).rows.find(row=>row.label==='State')?.value, 'Completed');
});
test('presentation has no content count cap at 5000 projects and defaults are saved options', () => {
  const data=fixture(); data.projects=Array.from({length:5000},(_,id)=>({...data.projects[0],id:`mod-${id}`}));
  assert.equal(validateSpecialProjectsReply(data).projects.length,5000);
  assert.equal(currentProjects(data,120).running.length,5000);
  assert.equal(projectHistory(data,120).lanes.length,5000);
  assert.equal('completedRetentionDays' in resolveOptions(OPTIONS,{}),false);
  assert.equal(resolveOptions(OPTIONS,{view:'history'}).view,'history');
  assert.equal(definition.defaultTime, 'follow');
});

test('paused cards are named, and cancelled projects leave Current, undated', () => {
  const data = fixture();
  data.projects[0].statuses = [{...data.projects[0].statuses[0],state:'paused',lastHour:139}, {...data.projects[0].statuses[0],firstHour:140,lastHour:199,readHour:140,state:'cancelled'}];
  data.projects[0].entries = [];
  assert.equal(currentProjects(data,139).running.length,1);
  assert.equal(projectTooltip(data,currentProjects(data,139).running[0],139).rows.find(row=>row.label==='State')?.value,'Paused');
  const cancelled = currentProjects(data,140);
  assert.equal(cancelled.running.length + cancelled.completed.length + cancelled.earlier.length, 0, 'a cancelled project is not listed in Current');
  const lane=projectHistory(data,199).lanes[0];
  assert.equal(lane.muted,true);
  assert.equal(lane.spans.at(-1)?.positive,false);
  assert.equal(lane.spans[0].shade,undefined, 'a paused stretch keeps the neutral fill');
  const tip=historyTooltip(data,'episode',lane.spans.at(-1)!,150,199);
  assert.equal(tip?.rows.find(row=>row.label==='State')?.value, 'Cancelled', 'a cancellation is the state read, never dated');
  assert.ok(!tip?.rows.some(row=>row.label==='Completed'||row.label==='Cancelled'));
});
test('history tooltip resolves the hovered status bounded by the playhead; gaps stay unknown', () => {
  const data=fixture();
  const span=projectHistory(data,199).lanes[0].spans[0];
  assert.equal(historyTooltip(data,'episode',span,120,199)?.rows.find(row=>row.label==='Progress')?.value,'17%');
  assert.equal(historyTooltip(data,'episode',{...span,lastHour:199},170,130)?.rows.find(row=>row.label==='Progress')?.value,'17%');
  assert.match(historyTooltip(data,'episode',null,205,300)?.note ?? '',/not recorded at this hour/);
  assert.equal(historyTooltip(data,'episode',null,170,199)?.rows.find(row=>row.label==='State')?.value, 'Completed');
});
test('group membership clips timeline bars and unassigned is an explicit flag', () => {
  const data=fixture();
  assert.deepEqual(projectHistory(data,199,[{firstHour:110,lastHour:115}]).lanes[0].spans.map(span=>[span.firstHour,span.lastHour]),[[110,115]]);
  data.people[0]={assigned:false,id:null,name:null,portrait:null,missing:'A translated message'};
  assert.equal(projectHistory(data,120).lanes[0].label,'No scientist assigned');
  data.people[0]={...data.people[0],assigned:null};
  assert.equal(projectHistory(data,120).lanes[0].label,'Scientist not recorded');
});

const countryGroupReply = (overrides: Partial<CountryGroupReply>): CountryGroupReply => ({ branchId: '1', warnings: [], warStarts: [], capitulations: [],
  identityChanges: [], factions: [], wars: [], humans: [], countryIdentities: [], unobserved: { humans: [], factions: [], wars: [] }, ...overrides });

const countryGroups = (): CountryGroupReply => countryGroupReply({
  humans: [{ tag: 'X', firstHour: 100, lastHour: 109 }, { tag: 'Y', firstHour: 110, lastHour: 114 }],
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

test('the dashboard country chosen by slot asks by each tag it held, which its replies name, never a tag another country held', () => {
  const scope = projectCountries({ kind: 'selected' }, [countryKey(1)], countryGroups(), 100, 150);
  assert.equal(scope.ready, true);
  assert.deepEqual([...scope.tags].sort(), ['X', 'Y'], 'asked by tag, never by the slot key a reply cannot name');
  assert.equal(scope.match.matches('X', 105), true);
  assert.equal(scope.match.matches('Y', 112), true);
  assert.equal(scope.match.matches('X', 130), false, 'X held by another country later is not the choice');
  assert.equal(scope.match.matches('Y', 130), false);
  assert.deepEqual(scope.memberships.get('Y')!.map(run => [run.firstHour, run.lastHour]), [[110, 114], [117, 119]]);
});

test('smart project membership keeps its dated hour, end and ever rules', () => {
  const groups = deepFreeze(countryGroups());
  const scope = (membership: 'hour' | 'end' | 'ever', reply = groups) => projectCountries({ kind: 'group', group: { type: 'humans' }, membership, aggregate: false }, [], reply, 100, 150);
  assert.equal(scope('hour').match.matches('Y', 118), false);
  assert.equal(scope('ever').match.matches('Y', 118), true);
  assert.equal(scope('ever').match.matches('Y', 120), false);
  assert.ok(scope('end').reason, 'the ended human group stays empty at the branch end');
  const end = deepFreeze({ ...groups, humans: [...groups.humans, { tag: 'X', firstHour: 130, lastHour: 150 }] });
  assert.equal(scope('end', end).match.matches('X', 140), true);
  assert.equal(scope('end', end).match.matches('X', 105), false, 'end membership does not admit a previous holder');
  const gap = deepFreeze({ ...end, unobserved: { ...end.unobserved, humans: [{ firstHour: 112, lastHour: 113 }] } });
  assert.equal(scope('hour', gap).match.matches('Y', 112), false);
});

test('a comparison heading carries complete flag status at the selected holder date', () => {
  const data = fixture();
  assert.equal(projectCountryHour(data, 219), 199);
  assert.equal(projectCountryHour(data, 99), null);
  assert.equal(projectCountryHour(data, 250, [{ firstHour: 110, lastHour: 115 }]), 115);
  const labels: CountryLabelValue[] = [{ name: 'Known', flag: 'http://artwork.localhost/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png', flagPending: true },
    { name: '', flagPending: false, flagReason: 'The supplying flag read refused its native inputs.' }];
  for (const label of labels) {
    const heading = projectHeading('country', label, '');
    assert.equal(heading.identity?.label, label.name, 'known empty names stay empty');
    assert.deepEqual(heading.identity?.countryFlag, { flag: label.flag, flagPending: label.flagPending, flagReason: label.flagReason });
    assert.equal(heading.identity?.icon, undefined);
  }
});

test('membership cannot borrow a later project presentation or status', () => {
  const data = fixture(), membership = [{ firstHour: 110, lastHour: 115 }];
  data.presentations.push({ ...data.presentations[0], name: 'Unselected later presentation' });
  data.projects[0].statuses[1].presentation = 1;
  const lane = projectHistory(data, 199, membership).lanes[0];
  assert.equal(lane.identity?.label, 'Mod project name');
  assert.equal(lane.spans[0].positive, false);
  assert.equal(historyCard(data, 'episode', 199, membership)?.run.state, 'running');
  assert.ok(!historyTooltip(data, 'episode', lane.spans[0], 112, 199, membership)?.rows.some(row => row.value === 'Completed'));
  assert.equal(projectHistory(data, 199, []).lanes[0], undefined);
});

test('clipping a status into future membership cannot reveal its blueprint or menu before the playhead', () => {
  const data = fixture(), membership = [{ firstHour: 100, lastHour: 110 }, { firstHour: 180, lastHour: 190 }];
  data.projects = [{ ...data.projects[0], firstRead: 140, statuses: [{ ...data.projects[0].statuses[0], firstHour: 140, lastHour: 200, readHour: 140 }] }];
  assert.deepEqual(projectHistory(data, 150, membership).lanes, []);
  assert.equal(historyCard(data, 'episode', 150, membership), null);
  const later = projectHistory(data, 185, membership).lanes[0];
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
  const lane = projectHistory(data, 120, membership).lanes[0];
  assert.deepEqual(lane.spans.map(span => [span.firstHour, span.lastHour]), [[100, 111], [114, 120]]);
  assert.match(historyTooltip(data, 'episode', null, 112, 120, membership)?.note ?? '', /not recorded at this hour/);
  data.projects = [{ ...data.projects[0], firstRead: 112, statuses: [{ ...data.projects[0].statuses[0], firstHour: 112, readHour: 112 }] }];
  assert.equal(historyCard(data, 'episode', 113, membership), null);
  assert.deepEqual(projectHistory(data, 113, membership).lanes, []);
});

test('History keeps its campaign axis by default and grows only from an available playhead', () => {
  assert.equal(resolveOptions(OPTIONS, { view: 'history' }).grow, false);
  assert.deepEqual(projectHistoryExtent(100, 300, 150, false), [100, 301]);
  assert.deepEqual(projectHistoryExtent(100, 300, 150, true), [100, 150]);
  assert.deepEqual(projectHistoryExtent(100, 300, null, true), [100, 301]);
  assert.deepEqual(projectHistoryExtent(120, 180, null, true), [120, 181]);
  assert.deepEqual(projectHistoryExtent(100, 300, 100, true), [100, 101]);
});

test('5000 project choices resolve through the native catalogue', () => {
  const groups = countryGroups();
  groups.countryIdentities = Array.from({ length: 5000 }, (_, index) => ({ tag: `mod-country-${index}`,
    identities: [{ firstHour: 100, lastHour: 300, slot: index, countryId: index + 5000 }] }));
  const roots = groups.countryIdentities.map(history => history.tag);
  const scope = projectCountries({ kind: 'tags', tags: roots }, [], groups, 100, 300);
  assert.equal(scope.tags.length, 5000);
  assert.equal(scope.match.matches('mod-country-4999', 300), true);
});

test('View is also a setting, and saved layouts drop the retired completed-project retention', () => {
  const view = OPTIONS.find(field => field.key === 'view')!;
  assert.equal(view.visibleWhen, undefined, 'View stays reachable when the header is narrow');
  for (const saved of ['live', 'history']) {
    const options = resolveOptions(OPTIONS, { view: saved, completedRetentionDays: 12, grow: true });
    assert.equal(options.view, saved);
    assert.equal('completedRetentionDays' in options, false);
    assert.equal(options.grow, true);
    assert.equal(OPTIONS.find(field => field.key === 'grow')?.visibleWhen?.(options), saved === 'history');
  }
  assert.equal(OPTIONS.some(field => field.key === 'completedRetentionDays'), false);
  assert.equal(definition.version, 2);
  const stored = (options?: Record<string, string | number | boolean>) => ({ id: 'projects', kind: 'special-projects', configVersion: 1, title: 'Special projects', trackIds: ['main'],
    countries: { kind: 'selected' }, time: { mode: 'campaign' }, ...(options ? { options } : {}) }) as unknown as Parameters<typeof definition.migrate>[1];
  assert.deepEqual(definition.migrate(1, stored({ completedRetentionDays: 12, view: 'history', grow: true }))?.options, { view: 'history', grow: true });
  assert.equal('options' in definition.migrate(1, stored({ completedRetentionDays: 30 }))!, false, 'a widget left with no options stores none');
  assert.equal(definition.migrate(1, stored())?.title, 'Special projects');
});

function projects(): SpecialProjectsReply {
  const data = fixture();
  data.iterationTexts = { 0: 'Mod iterations: 0', 1: 'Mod iterations: 1', 2: 'Mod iterations: 2', 3: 'Mod iterations: 3' };
  data.presentations.push(...['Alpha early', 'Beta running', 'Gamma running', 'Delta cancelled'].map(name => ({ ...data.presentations[0], name })));
  const run = (firstHour: number, lastHour: number, presentation: number, progress: number, state: ProjectStatus['state'], iterations = 0): ProjectStatus =>
    ({ firstHour, lastHour, readHour: firstHour, presentation, progress, state, iterations, scientistRead: null, lastScientistRead: null });
  const project = (id: string, local: number, firstRead: number, firstState: ProjectStatus['state'], statuses: ProjectStatus[], entries: ProjectEntry[] = []) =>
    ({ id, owner: 9, local, firstRead, firstState, statuses, entries, attempts: [] });
  data.projects = [
    project('done', 1, 100, 'running', [run(100, 159, 0, 40, 'running'), run(160, 199, 0, 100, 'completed')], [marker(160, 160)]),
    project('early', 2, 100, 'completed', [run(100, 199, 1, 100, 'completed')]),
    project('beta', 3, 120, 'running', [run(120, 149, 2, 0, 'running'), run(150, 172, 2, 30, 'running'), run(173, 199, 2, 30, 'running', 1)]),
    project('gamma', 4, 110, 'running', [run(110, 199, 3, 55, 'running')]),
    project('delta', 5, 100, 'cancelled', [run(100, 199, 4, 65, 'cancelled')]),
  ];
  return validateSpecialProjectsReply(data);
}

test('Current lists running projects closest to completion first, then completed and the folded first-recorded group', () => {
  const ids = (cards: readonly { project: { id: string } }[]) => cards.map(card => card.project.id);
  const late = currentProjects(projects(), 199);
  assert.deepEqual(ids(late.running), ['gamma', 'beta']);
  assert.deepEqual(ids(late.completed), ['done']);
  assert.deepEqual(ids(late.earlier), ['early']);
  assert.ok(![...late.running, ...late.completed, ...late.earlier].some(card => card.project.id === 'delta'), 'the cancelled project is not listed');
  assert.equal(earlierBound(late.earlier), 100);
  const before = currentProjects(projects(), 159);
  assert.deepEqual(ids(before.running), ['gamma', 'done', 'beta'], 'progress orders running projects, not their ids');
  assert.deepEqual(ids(before.completed), []);
  const data = projects();
  assert.equal(completedWhenFirstRecorded(data.projects[0]), false, 'a completion seen after it ran lists on its own row');
  assert.equal(completedWhenFirstRecorded(data.projects[1]), true, 'already complete at its first state read');
  const tie = projects();
  tie.projects[2].statuses = tie.projects[2].statuses.map(run => ({ ...run, progress: 55 }));
  assert.deepEqual(ids(currentProjects(tie, 199).running), ['gamma', 'beta'], 'equal progress keeps the earlier first read first');
});

test('History lanes run in first-read order, split at each progress step and fold projects that ended before the range into one line', () => {
  const history = projectHistory(projects(), 199);
  assert.deepEqual(history.lanes.map(lane => lane.id), ['early', 'delta', 'done', 'gamma', 'beta'], 'first-read order, then name');
  const beta = history.lanes.find(lane => lane.id === 'beta')!;
  assert.deepEqual(beta.spans.map(span => [span.firstHour, span.lastHour, span.label, span.shade]), [[120, 149, '0%', 0], [150, 199, '30%', 0.3]],
    'an iteration count change at equal progress keeps one segment');
  const ranged = projectHistory(projects(), 150, undefined, 165);
  assert.deepEqual(ranged.lanes.map(lane => lane.id).sort(), ['beta', 'done', 'gamma'], 'ended projects whose last status lies before the range fold');
  assert.deepEqual([ranged.completed, ranged.cancelled, ranged.by], [1, 1, 150]);
  assert.equal(closedLane('closed', ranged)?.label, `1 completed, 1 cancelled by ${dayLabel(150)}`);
  assert.equal(closedLane('closed', ranged)?.compact, true);
  assert.equal(closedLane('closed', projectHistory(fixture(), 199)), null);
});

test('Current renders again only when a section, order, name, scientist, state or date changes, never for progress alone', () => {
  const data = projects();
  assert.equal(currentKey(data, 149), currentKey(data, 150));
  assert.notEqual(currentKey(data, 172), currentKey(data, 173), 'a new iteration count renders');
  assert.notEqual(currentKey(data, 159), currentKey(data, 160));
  const overtaking = projects();
  overtaking.projects[2].statuses[2] = { ...overtaking.projects[2].statuses[2], progress: 60 };
  assert.notEqual(currentKey(overtaking, 172), currentKey(overtaking, 173));
  assert.equal(currentProjects(data, 200).running.length, 0, 'an hour not recorded shows no cards');
  const inPart = { ...projects(), partial: [{ firstHour: 200, lastHour: 200 }] };
  assert.notEqual(currentKey(inPart, 200), currentKey(inPart, 201), 'an hour read in part and a following unrecorded hour name different reasons');
  const dated = projects();
  dated.projects[0].entries = [marker(90, 160), marker(165, 170)];
  assert.notEqual(historyPicture([dated], 170, () => undefined), historyPicture([data], 170, () => undefined), 'History renders again when a completion outside its status is listed');
  const counted = projects();
  counted.projects[2].statuses[2] = { ...counted.projects[2].statuses[2], iterations: 2 };
  assert.notEqual(currentKey(counted, 180), currentKey(projects(), 180), 'U-S4: a recorded iteration count renders the open pop-up again');
});

test('History names an interval read only in part or not at all, never as no projects, though a later read is complete or only part was read', () => {
  const later = { ...fixture(), projects: [], available: [{ firstHour: 220, lastHour: 300 }], partial: [{ firstHour: 150, lastHour: 150 }] };
  assert.equal(historyUnrecorded([later], 140, 160), 'Projects were read only in part over this time.');
  assert.equal(historyUnrecorded([later], 100, 140), 'Projects were not recorded over this time.');
  assert.equal(historyUnrecorded([later], 220, 230), null, 'complete reads over the whole interval prove no projects');
  const mixed = 'No projects in complete reads. Other hours are unknown.';
  assert.equal(historyUnrecorded([later], 140, 230), mixed, 'one complete read does not prove no projects over hours never read whole');
  assert.equal(historyUnrecorded([{ ...later, partial: [] }], 140, 230), mixed);
  assert.equal(historyUnrecorded([later], 140, 230, () => [{ firstHour: 220, lastHour: 300 }]), null, 'hours outside the membership are no gap');
  assert.equal(historyUnrecorded([later], 140, 230, () => [{ firstHour: 140, lastHour: 160 }]), 'Projects were read only in part over this time.',
    'a complete read outside the membership proves nothing for the member hours');
  const unknownHolder = { ...later, identities: [{ firstHour: 100, lastHour: 100, slot: 5, countryId: 10 }, { firstHour: 102, lastHour: 102, slot: 5, countryId: 10 }],
    available: [{ firstHour: 100, lastHour: 100 }, { firstHour: 102, lastHour: 102 }], partial: [], unresolved: [{ firstHour: 101, lastHour: 101 }] };
  const held = () => [{ firstHour: 100, lastHour: 100 }, { firstHour: 102, lastHour: 102 }];
  assert.equal(historyUnrecorded([validateSpecialProjectsReply(unknownHolder)], 100, 102, held), mixed, 'an hour whose holder is unknown is not an hour without projects, though its membership is unknown too');
  const empty = { ...later, available: [{ firstHour: 100, lastHour: 150 }], partial: [] };
  assert.notEqual(historyPicture([empty], 150, () => undefined, 100), historyPicture([empty], 160, () => undefined, 100), 'with no projects, a change in coverage renders');
  assert.equal(historyPicture([empty], 160, () => undefined, 100), historyPicture([empty], 170, () => undefined, 100));
});

test('a running card paints its progress, bar and iterations at the hour, and nothing where the hour holds no status', () => {
  const data = projects();
  const beta = data.projects[2];
  assert.deepEqual(runningValues(beta, 149), { progress: '0%', fill: 0, iterations: '×0' });
  assert.deepEqual(runningValues(beta, 180), { progress: '30%', fill: 0.3, iterations: '×1' });
  assert.equal(runningValues(beta, 100), null);
  const backwards = { ...beta, statuses: [{ ...beta.statuses[0], progress: -20 }] };
  assert.equal(runningValues(backwards, 130)?.fill, 0, 'a negative prototype progress draws an empty bar, its percent as recorded');
  assert.equal(runningValues(backwards, 130)?.progress, '-20%');
});

test('History bounded by the playhead shows no status past it, and renders again only at a start, an end or a new scientist', () => {
  const data = projects();
  assert.deepEqual(projectHistory(data, 159).lanes.find(lane => lane.id === 'done')!.spans.map(span => [span.firstHour, span.lastHour, span.positive]), [[100, 159, false]],
    'a completion after the hour is not shown yet');
  assert.deepEqual(projectHistory(data, 159).lanes.find(lane => lane.id === 'beta')!.spans.map(span => span.lastHour), [149, 159]);
  const picture = (hour: number) => historyPicture([data], hour, () => undefined, -Infinity, 199);
  assert.equal(picture(151), picture(159), 'within one status drawn open to the head nothing renders');
  assert.notEqual(picture(149), picture(150), 'U-S5, X-S8: a progress-only step renders, so the lane keeps drawing');
  assert.notEqual(picture(159), picture(160), 'a completion renders');
  assert.notEqual(picture(119), picture(120), 'a project first recorded renders');
});



test('a project window joins its runtime project on the owner and local id, read within its statuses and not after the hour', () => {
  const data=fixture(); data.projects[0]={ ...data.projects[0], owner: 9, local: 3 };
  const card=shownCard(data,120,data.projects[0].id)!;
  const runtime=(owner:number,id:number,name:string|null,readHour=120):ProjectRuntimeDetails=>({owner,id,label:{token:'mod_project',name},readHour,pendingKnown:true,pending:null,seed:null,variables:null,choices:null,historyReadHour:null});
  const details=(projects:ProjectRuntimeDetails[]|null,hour=120):SpecialProjectDetails=>({hour,scientists:null,programs:null,breakthroughs:null,projects,unavailable:[],warnings:[]});
  const joined=(project:ProjectRuntimeDetails)=>({project,choices:project.choices});
  assert.deepEqual(runtimeProject(details([runtime(9,4,'Mod project name'),runtime(9,3,'Other')]),card,120), joined(runtime(9,3,'Other')), 'the name is not the join');
  assert.match(runtimeProject(details([runtime(10,3,'Mod project name')]),card,120) as string, /not recorded/, 'another owner with the same local id is not this project');
  assert.match(runtimeProject(details([runtime(9,3,'Mod project name',99)],106),card,120) as string, /before this project was first recorded/);
  assert.deepEqual(runtimeProject(details([runtime(9,3,'Mod project name',100)],106),card,120), joined(runtime(9,3,'Mod project name',100)), 'a row read at the first read is its own');
  const ended={...card,project:{...card.project,statuses:[card.project.statuses[0]]}};
  assert.match(runtimeProject(details([runtime(9,3,'Mod project name',148)],148),ended,150) as string, /after this project was last recorded/);
  assert.equal(runtimeProject(details([runtime(9,3,'Mod project name')],121),card,120), null, 'a reply from after the displayed hour waits');
  assert.match(runtimeProject(details([]),card,120) as string, /not recorded/);
  assert.match(runtimeProject(details(null),card,120) as string, /not read/);
  const open={...data,available:[{firstHour:100,lastHour:147}],projects:[{...data.projects[0],statuses:[data.projects[0].statuses[0]],entries:[]}]};
  const running=shownCard(open,147,open.projects[0].id)!;
  for (const through of [null, 147]) {
    const shown = runtimeProject(details([runtime(9,3,'Mod project name',160)],160),running,160,episodeBound(open,running.project,through));
    assert.equal(shown, `Project rewards were read ${dayLabel(160)}, after the project list is known current (${dayLabel(147)}).`,
      'a running project read after the list stands is not yet joined');
  }
  assert.deepEqual(runtimeProject(details([runtime(9,3,'Mod project name',160)],160),running,160,episodeBound(open,running.project,170)), joined(runtime(9,3,'Mod project name',160)),
    'a running project the list still covers takes its later read');
  assert.deepEqual(episodeBound(data,card.project,170), { through: 199, open: false }, 'a project no longer listed keeps its last status');
});

test('the live pop-up names a read past the list it was joined on, and wakes when the list reaches it', () => {
  const data=fixture(); data.projects[0]={ ...data.projects[0], owner: 9, local: 3 };
  const open={...data,available:[{firstHour:100,lastHour:147}],projects:[{...data.projects[0],statuses:[data.projects[0].statuses[0]],entries:[]}]};
  const running=shownCard(open,147,open.projects[0].id)!;
  const read:ProjectRuntimeDetails={owner:9,id:3,label:{token:'mod_project',name:null},readHour:160,pendingKnown:true,pending:null,seed:null,variables:null,choices:null,historyReadHour:null};
  const details:SpecialProjectDetails={hour:160,scientists:null,programs:null,breakthroughs:null,projects:[read],unavailable:[],warnings:[]};
  const listed=147, shown=Math.max(147, listed), bound=episodeBound(open,running.project,listed);
  assert.equal(runtimeProject(details,running,shown,bound), `Project rewards were read ${dayLabel(160)}, after the project list is known current (${dayLabel(147)}).`);
  assert.deepEqual(joinDue(details,running,shown,bound), { hour: 160, list: true }, 'it wakes on the list reaching the read, not on the frozen hour');
  assert.deepEqual(joinDue({...details,projects:[{...read,readHour:140}]},running,shown,bound), { hour: 160, list: false }, 'a reply ahead of the shown hour waits on the clock');
  assert.equal(joinDue({...details,hour:147,projects:[{...read,readHour:140}]},running,shown,bound), null);
});

test('a pop-up joins only reads and choices made while the project was recorded', () => {
  const data=fixture();
  const gapped={...data,projects:[{...data.projects[0],owner:9,local:3,firstRead:110,entries:[],statuses:[{...data.projects[0].statuses[0],firstHour:110,lastHour:123,readHour:110},{...data.projects[0].statuses[0],firstHour:140,lastHour:145,readHour:140}]}]};
  const card={project:gapped.projects[0],run:gapped.projects[0].statuses[1],info:gapped.presentations[0]};
  const choice={ordinal:0,date:100,earlier:false,reward:{token:'mod_reward',name:null},option:{token:'mod_option',name:null},province:0,progress:0,characterReference:'0',country:null,scientist:null,state:null};
  const read=(readHour:number,historyReadHour:number|null):ProjectRuntimeDetails=>({owner:9,id:3,label:{token:'mod_project',name:null},readHour,pendingKnown:true,pending:null,seed:null,variables:null,
    choices:historyReadHour===null?null:[choice],historyReadHour});
  const details=(project:ProjectRuntimeDetails):SpecialProjectDetails=>({hour:145,scientists:null,programs:null,breakthroughs:null,projects:[project],unavailable:[],warnings:[]});
  assert.equal(runtimeProject(details(read(130,130)),card,145), 'Project rewards and output were read while this project was not recorded.');
  const before=runtimeProject(details(read(141,100)),card,145);
  assert.ok(before && typeof before==='object');
  assert.equal(before.choices, 'Iterations were last read before this project was first recorded.');
  assert.equal((runtimeProject(details(read(141,130)),card,145) as {choices:unknown}).choices, 'Iterations were read while this project was not recorded.');
  assert.deepEqual((runtimeProject(details(read(141,142)),card,145) as {choices:unknown}).choices, [choice]);
  assert.equal((runtimeProject(details(read(141,null)),card,145) as {choices:unknown}).choices, null);
});

test('U-S1: a status and its scientist name the read they come from when it is older than the hour shown', () => {
  const data = fixture();
  data.reads = [{ owner: 9, firstHour: 100, lastHour: 123, readHour: 100 }, { owner: 9, firstHour: 124, lastHour: 147, readHour: 124 }];
  const project = data.projects[0];
  assert.equal(statusRead(data, project, 120), 100);
  assert.equal(statusRead(data, project, 130), 124, 'the owner read current at the hour, never the status run\'s first read');
  assert.equal(statusRead(data, project, 150), null);
  assert.equal(readLabel(100, 100), null, 'a read at the hour shown is current');
  const tip = projectTooltip(data, currentProjects(data, 130).running[0], 130);
  assert.equal(tip.rows.find(row => row.label === 'Status')?.value, `read ${formatDateTime(124)}`);
  assert.equal(tip.rows.find(row => row.label === 'Scientist')?.value, `Mod scientist (read ${formatDateTime(100)})`);
  assert.notEqual(currentKey(data, 123), currentKey(data, 124), 'Current renders again when the read it names changes');
  assert.notEqual(currentKey(data, 124), currentKey(data, 125), 'K6: and when its label first shows, one hour after the read');
});

test('X-S9: the pop-up joins the Projects read that supplied its card, though it precedes the published hours', () => {
  const data = fixture();
  data.projects[0] = { ...data.projects[0], statuses: [{ ...data.projects[0].statuses[0], firstHour: 101, readHour: 100 }], firstRead: 100 };
  data.available = [{ firstHour: 101, lastHour: 199 }];
  const card = shownCard(data, 120, 'episode')!;
  const read: ProjectRuntimeDetails = { owner: 9, id: 3, label: { token: 'mod_project', name: null }, readHour: 100, pendingKnown: true, pending: null, seed: null, variables: null, choices: null, historyReadHour: null };
  const joined = runtimeProject({ hour: 120, scientists: null, programs: null, breakthroughs: null, projects: [read], unavailable: [], warnings: [] }, card, 120);
  assert.ok(joined && typeof joined === 'object', `${joined}`);
});

test('U-S5: with the playhead live, the latest bounded status draws on to the head; earlier ones stay cut at their end', () => {
  const data = projects();
  const beta = (openTo?: number) => projectHistory(data, 160, undefined, -Infinity, openTo).lanes.find(lane => lane.id === 'beta')!.spans.map(span => span.lastHour);
  assert.deepEqual(beta(), [149, 160]);
  assert.deepEqual(beta(199), [149, 199], 'the timeline clips the open status at the live head, so the bar grows without another request');
});

test('R3-S5: a project seen only in history shows its dated completions with status unknown', () => {
  const data = fixture();
  data.projects.push({ id: 'history-only', owner: 9, local: 4, firstRead: 120, firstState: 'unknown', statuses: [], entries: [marker(118, 120)], attempts: [] });
  const lane = projectHistory(data, 130).lanes.find(lane => lane.id === 'history-only');
  assert.equal(lane?.label, 'Status not recorded');
  assert.deepEqual(lane?.spans.map(span => [span.firstHour, span.lastHour, span.label]), [[118, 118, 'Completed']]);
  assert.equal(projectHistory(data, 119).lanes.find(lane => lane.id === 'history-only'), undefined, 'not before the read that first listed it');
});

test('R5-S1: a status that ends before a later read stops where it ends, and the picture changes when that read arrives', () => {
  const data = fixture();
  const ending = { ...data, available: [{ firstHour: 100, lastHour: 131 }],
    projects: [{ ...data.projects[0], statuses: [{ ...data.projects[0].statuses[0], firstHour: 120, lastHour: 130, state: 'running' as const }] }] };
  const drawn = (hour: number) => projectHistory(ending, hour, undefined, -Infinity, 199).lanes[0]?.spans.map(span => span.lastHour);
  assert.deepEqual(drawn(130), [199], 'the status read at the head draws on to the head');
  assert.deepEqual(drawn(131), [130], 'once a later read lacks it, the bar ends at its last status');
  assert.notEqual(historyPicture([ending], 130, () => undefined, -Infinity, 199), historyPicture([ending], 131, () => undefined, -Infinity, 199));
});

test('R5-S1, R5-S5: the followed History installs again when a status ends or a read repeats, never as a running status grows, and holds open runs to the head', () => {
  const data = fixture();
  const status = { ...data.projects[0].statuses[0], firstHour: 120, lastHour: 130, readHour: 120, state: 'running' as const };
  const at = (head: number, lastStatus: number, reads = [{ owner: 9, firstHour: 120, lastHour: head, readHour: 120 }]) => ({ ...data, available: [{ firstHour: 100, lastHour: head }],
    partial: [], unresolved: [], reads, projects: [{ ...data.projects[0], statuses: [{ ...status, lastHour: lastStatus }] }] });
  assert.equal(historyStructure(at(131, 131), 131), historyStructure(at(130, 130), 130), 'a running status that grows needs no install');
  assert.notEqual(historyStructure(at(131, 130), 131), historyStructure(at(130, 130), 130), 'R5-S1: a status that ends before the head installs');
  const reread = at(131, 131, [{ owner: 9, firstHour: 120, lastHour: 130, readHour: 120 }, { owner: 9, firstHour: 131, lastHour: 131, readHour: 131 }]);
  assert.notEqual(historyStructure(reread, 131), historyStructure(at(130, 130), 130), 'R5-S5: a repeated read installs, so tooltips name it');
  const held = heldThrough(at(131, 130), 131, 140);
  assert.deepEqual([held.available[0].lastHour, held.reads[0].lastHour, held.projects[0].statuses[0].lastHour], [140, 140, 130], 'open runs hold to the head; an ended status stays ended');
});

test('F-S1: a missing read while the country stays identified ends the status there, though no coverage reaches the hour', () => {
  const data = fixture();
  const status = { ...data.projects[0].statuses[0], firstHour: 120, lastHour: 130, readHour: 120, state: 'running' as const };
  const unread = { ...data, identities: [{ firstHour: 100, lastHour: 131, slot: 5, countryId: 10 }], available: [{ firstHour: 100, lastHour: 130 }], partial: [], unresolved: [],
    reads: [{ owner: 9, firstHour: 120, lastHour: 130, readHour: 120 }], projects: [{ ...data.projects[0], statuses: [status] }] };
  const read = { ...unread, identities: [{ firstHour: 100, lastHour: 130, slot: 5, countryId: 10 }] };
  assert.notEqual(historyStructure(unread, 131), historyStructure(read, 130), 'the hour without a read installs');
  const held = heldThrough(unread, 131, 140);
  assert.deepEqual([held.identities[0].lastHour, held.available[0].lastHour, held.projects[0].statuses[0].lastHour], [140, 130, 130],
    'only runs open at the evaluated hour are held to the head');
});

test('F-S6, F-S7: a kept completion hovers as its counted entry, and warnings change the picture', () => {
  const data = fixture();
  data.projects.push({ id: 'history-only', owner: 9, local: 4, firstRead: 120, firstState: 'unknown', entries: [marker(118, 120)], attempts: [],
    statuses: [{ ...data.projects[0].statuses[0], firstHour: 140, lastHour: 150, state: 'running', progress: 5 }] });
  const span = projectHistory(data, 150).lanes.find(lane => lane.id === 'history-only')!.spans[0];
  const tip = historyTooltip(data, 'history-only', span, 118, 150);
  assert.deepEqual(tip?.rows, [{ label: 'Completed', value: dayLabel(118) }], `the completion bar names its completion: ${JSON.stringify(tip)}`);
  assert.notEqual(historyPicture([{ ...data, warnings: ['a new warning'] }], 150, () => undefined), historyPicture([data], 150, () => undefined));
});

test('F-S2: a newly counted completion changes the Current structure, so its menu offers the jump', () => {
  const data = projects();
  const counted = projects();
  counted.projects[2].entries = [marker(170, 175)];
  assert.notEqual(projectStructure(counted, 180), projectStructure(data, 180));
});

test('R3-S5: a completion dated outside the project status stays drawn once the project gains a status', () => {
  const data = fixture();
  data.projects.push({ id: 'history-only', owner: 9, local: 4, firstRead: 120, firstState: 'unknown', statuses: [], entries: [marker(118, 120)], attempts: [] });
  const before = projectHistory(data, 130).lanes.find(lane => lane.id === 'history-only');
  assert.deepEqual(before?.spans.map(span => [span.firstHour, span.label]), [[118, 'Completed']]);
  const restarted = data.projects.at(-1)!;
  restarted.statuses = [{ ...data.projects[0].statuses[0], firstHour: 140, lastHour: 150, state: 'running', progress: 5 }];
  const after = projectHistory(data, 150).lanes.find(lane => lane.id === 'history-only');
  assert.deepEqual(after?.spans.map(span => [span.firstHour, span.label]), [[118, 'Completed'], [140, '5%']], 'the later status keeps the earlier completion bar');
});

test('R5-S3: a project seen only in history has a tooltip with status unknown and a menu to jump to its completion', () => {
  const data = fixture();
  data.projects.push({ id: 'history-only', owner: 9, local: 4, firstRead: 120, firstState: 'unknown', statuses: [], entries: [marker(118, 120)], attempts: [] });
  const tip = historyTooltip(data, 'history-only', { id: 'x', firstHour: 118, lastHour: 118, label: 'Completed' }, 118, 130);
  assert.deepEqual(tip?.rows, [{ label: 'Completed', value: dayLabel(118) }, { label: 'State', value: 'Not recorded' }]);
  assert.match(tip?.note ?? '', /status is unknown/);
  const project = historyProject(data, 'history-only', 130);
  assert.equal(project?.id, 'history-only');
  assert.deepEqual(completions(project!, 130).map(entry => entry.date), [118], 'the menu lists its completion to jump to');
  assert.equal(historyProject(data, 'history-only', 119), null, 'no menu before the read that first listed it');
  assert.equal(historyTooltip(data, 'history-only', null, 118, 119), null);
});

