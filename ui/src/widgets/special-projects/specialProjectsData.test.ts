import { countryKey } from '../../dashboard/countryKey.ts';
import { panelStructure, panelValues } from '../../draw/panels/views.ts';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deepFreeze } from '../../testFrozen.ts';
import type { CountryGroupReply, SpecialProjectRun, SpecialProjectsReply } from '../../generated/dashboard';
import type { CountryLabelValue } from '../../dashboard/countryLabelData.ts';
import { closedLane, completedWhenFirstRecorded, currentKey, currentProjects, earlierBound, historyCard, historyKey, historyTooltip, iterationCount, observedAt, projectCountries, projectCountryHour, projectHeading, projectHistory, projectHistoryExtent, projectTooltip, runningValues, validateSpecialProjectsReply } from './specialProjectsData.ts';
import { OPTIONS, definition } from './definition.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import { dayLabel } from '../../charts/draw.ts';
import { effectsStructure, effectTexts } from './projectDetailsData.ts';

test('an open effect keeps its layout when its dated line or refusal count changes', () => {
  const record = (hour: number, count: number) => ({ hour, options: [], warnings: [], unavailable: null,
    output: { text: 'Modifier\nAmount ' + count, refusals: { 'Unknown effect': count } } });
  assert.equal(effectsStructure(record(12, 3)), effectsStructure(record(24, 8)));
  assert.equal(effectTexts(record(24, 8)).get('output:1'), 'Amount 8');
  assert.equal(effectTexts(record(24, 8)).get('output:reason:Unknown effect'), 'Unknown effect (8)');
});

const fixture = (): SpecialProjectsReply => ({ branchId: '1', tag: 'tag with spaces', unavailable: null, warnings: [],
  identities: [{firstHour:100,lastHour:199,slot:5,countryId:10},{firstHour:220,lastHour:300,slot:5,countryId:10}],
  available: [{firstHour:100,lastHour:199},{firstHour:220,lastHour:300}],
  iterationTexts:{2:'Mod iterations: 2',3:'Mod iterations: 3'},
  people:[{assigned:true,id:77,name:'Mod scientist',portrait:null,missing:'Portrait not recorded.'}],
  presentations:[{name:'Mod project name',specialization:'mod_kind',specializationName:'Mod science',icon:null,iconMissing:'Missing mod icon',scientist:0,scientistAsOf:null}],
  projects:[{id:'episode',start:{hour:100,exact:false},runs:[
    {firstHour:100,lastHour:147,presentation:0,progress:17,state:'running',cancellation:null,completion:null,iterations:2},
    {firstHour:148,lastHour:199,presentation:0,progress:100,state:'completed',cancellation:null,completion:{hour:140,exact:true},iterations:3},
  ]}],
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

test('daily reads step without interpolation and future completion stays hidden', () => {
  const data = fixture();
  assert.equal(currentProjects(data,147).running[0].run.progress,17);
  assert.equal(currentProjects(data,147).running[0].run.completion,null);
  assert.equal(currentProjects(data,148).completed[0].run.completion?.hour,140);
  assert.equal(projectHistory(data,147).lanes[0].spans[0].positive,false);
  assert.equal(projectHistory(data,148).lanes[0].spans[0].positive,true);
});
test('missing hours are unavailable and completed projects stay listed without a retention period', () => {
  const data = fixture();
  assert.equal(observedAt(data,200),false);
  assert.deepEqual(currentProjects(data,200),{ running: [], completed: [], earlier: [], cancelled: [] });
  assert.equal(currentProjects(data,148).completed.length,1);
  assert.equal(currentProjects(data,199).completed.length,1, 'a completion long past still lists');
});
test('history caps at the completion event, splits at progress steps and leaves unrecorded intervals empty', () => {
  const data = fixture();
  const lane = projectHistory(data,199).lanes[0];
  assert.equal(lane.identity?.tile,'blueprint');
  assert.equal(lane.label,'Mod scientist');
  assert.equal(lane.names,true);
  assert.equal(lane.spans[0].lastHour,139);
  assert.equal(lane.spans[0].label,'17%');
  assert.equal(lane.spans[0].endLabel,'✓');
  assert.equal(lane.spans[0].shade,undefined, 'a completed bar keeps the positive fill');
  data.projects[0].runs[1].state='running'; data.projects[0].runs[1].completion=null;
  data.projects[0].runs.push({...data.projects[0].runs[0],firstHour:220,lastHour:230});
  const running = projectHistory(data,300).lanes[0].spans;
  assert.deepEqual(running.map(span=>[span.firstHour,span.lastHour,span.label,span.shade]),[[100,147,'17%',0.17],[148,199,'100%',1],[220,230,'17%',0.17]]);
});
test('tooltips keep bounded starts and progress without artwork or table diagnostics', () => {
  const data=fixture(); data.projects[0].runs[0].iterations=null; data.iterationTexts['2']=null;
  const tip=projectTooltip(data,currentProjects(data,120).running[0]);
  assert.match(tip.rows.find(row=>row.label==='Started')!.value!,/^by /);
  assert.doesNotMatch(tip.note ?? '',/read once a day/);
  assert.equal(tip.note, undefined);
  assert.equal(tip.description, undefined);
  assert.equal(tip.subtitle, 'Mod science');
  assert.equal(tip.rows.find(row=>row.label==='Progress')?.value, '17%');
});
test('validator rejects bad references, overlaps, missing and invented percentages', () => {
  assert.equal(validateSpecialProjectsReply(fixture()).tag,'tag with spaces');
  for(const mutate of [
    (data:SpecialProjectsReply)=>{data.presentations[0].scientist=1;},
    (data:SpecialProjectsReply)=>{data.projects[0].runs[1].firstHour=100;},
    (data:SpecialProjectsReply)=>{data.projects[0].runs[0].progress=101;},
    (data:SpecialProjectsReply)=>{data.available[1].firstHour=150;},
    (data:SpecialProjectsReply)=>{data.projects[0].runs[1].completion!.hour=200;},
    (data:SpecialProjectsReply)=>{data.projects[0].runs[1].state='running';},
    (data:SpecialProjectsReply)=>{data.identities[1].firstHour=190;},
  ]) {const data=fixture();mutate(data);assert.throws(()=>validateSpecialProjectsReply(data));}
});
test('last assignment keeps its date in its own tooltip row and the game text closes the tooltip', () => {
  const data=fixture(); data.presentations[0].scientistAsOf=99;
  const card=currentProjects(data,120).running[0];
  const tip=projectTooltip(data,card);
  assert.equal(tip.rows.find(row=>row.label==='Scientist')!.value, 'Mod scientist');
  assert.equal(tip.rows.find(row=>row.label==='Last assigned')!.value, dayLabel(99));
  assert.equal(tip.description, 'Mod iterations: 2');
  assert.equal(tip.note, undefined);
  assert.equal(iterationCount(card.run.iterations), '2 iterations');
  assert.equal(iterationCount(1), '1 iteration');
  assert.equal(iterationCount(null), 'Iterations unknown');
  const done=currentProjects(data,160).completed[0];
  data.presentations[0].scientistAsOf=done.run.completion!.hour;
  assert.ok(!projectTooltip(data,done).rows.some(row=>row.label==='Last assigned'), 'an assignment dated to the completion adds no row');
  assert.equal(projectTooltip(data,done).rows.find(row=>row.label==='Completed')?.value, dayLabel(140));
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

test('paused cards are named and cancelled episodes end by observation without a completion tick', () => {
  const data = fixture();
  data.projects[0].runs = [{...data.projects[0].runs[0],state:'paused',lastHour:139}, {...data.projects[0].runs[0],firstHour:140,lastHour:199,state:'cancelled',cancellation:{hour:140,exact:false}}];
  assert.equal(currentProjects(data,139).running.length,1);
  assert.equal(projectTooltip(data,currentProjects(data,139).running[0]).rows.find(row=>row.label==='State')?.value,'Paused');
  assert.equal(currentProjects(data,140).running.length,0);
  assert.equal(currentProjects(data,140).cancelled.length,1, 'a cancelled project is listed, not dropped');
  const lane=projectHistory(data,199).lanes[0];
  assert.equal(lane.muted,true);
  assert.equal(lane.spans.at(-1)?.lastHour,139);
  assert.equal(lane.spans.at(-1)?.positive,false);
  assert.equal(lane.spans.at(-1)?.endLabel,undefined);
  assert.equal(lane.spans.at(-1)?.shade,undefined, 'a paused stretch keeps the neutral fill');
});
test('history tooltip resolves hovered observation and span, bounded by playhead; gaps stay unknown', () => {
  const data=fixture();
  const span=projectHistory(data,199).lanes[0].spans[0];
  assert.equal(historyTooltip(data,'episode',span,120,199)?.rows.find(row=>row.label==='Progress')?.value,'17%');
  assert.ok(historyTooltip(data,'episode',span,120,199)?.rows.some(row=>row.label==='Completed'));
  assert.equal(historyTooltip(data,'episode',{...span,lastHour:199},170,130)?.rows.find(row=>row.label==='Progress')?.value,'17%');
  assert.match(historyTooltip(data,'episode',null,205,300)?.note ?? '',/not recorded at this hour/);
  const after=historyTooltip(data,'episode',null,170,199);
  assert.equal(after?.note, undefined, 'past its bar a completed project is recorded as completed, not missing');
  assert.equal(after?.rows.find(row=>row.label==='Completed')?.value, dayLabel(140));
});
test('group membership clips timeline bars and unassigned is an explicit flag', () => {
  const data=fixture();
  assert.deepEqual(projectHistory(data,199,[{firstHour:110,lastHour:115}]).lanes[0].spans.map(span=>[span.firstHour,span.lastHour]),[[110,115]]);
  data.people[0]={assigned:false,id:null,name:null,portrait:null,missing:'A translated message'};
  assert.equal(projectHistory(data,120).lanes[0].label,'No scientist assigned');
  data.people[0]={...data.people[0],assigned:null};
  assert.equal(projectHistory(data,120).lanes[0].label,'Scientist unknown');
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

test('membership cannot borrow a later project presentation or completion', () => {
  const data = fixture(), membership = [{ firstHour: 110, lastHour: 115 }];
  data.presentations.push({ ...data.presentations[0], name: 'Unselected later presentation' });
  data.projects[0].runs[1].presentation = 1;
  const lane = projectHistory(data, 199, membership).lanes[0];
  assert.equal(lane.identity?.label, 'Mod project name');
  assert.equal(lane.spans[0].positive, false);
  assert.equal(historyCard(data, 'episode', 199, membership)?.run.state, 'running');
  assert.ok(!historyTooltip(data, 'episode', lane.spans[0], 112, 199, membership)?.rows.some(row => row.label === 'Completed'));
  assert.equal(projectHistory(data, 199, []).lanes[0], undefined);
});

test('clipping a run into future membership cannot reveal its blueprint or menu before the playhead', () => {
  const data = fixture(), membership = [{ firstHour: 100, lastHour: 110 }, { firstHour: 180, lastHour: 190 }];
  data.projects = [{ ...data.projects[0], start: { hour: 140, exact: false }, runs: [{ ...data.projects[0].runs[0], firstHour: 140, lastHour: 200 }] }];
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
  data.projects = [{ ...data.projects[0], start: { hour: 112, exact: false }, runs: [{ ...data.projects[0].runs[0], firstHour: 112 }] }];
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
  const run = (firstHour: number, lastHour: number, presentation: number, progress: number, state: SpecialProjectRun['state'], extra: Partial<SpecialProjectRun> = {}): SpecialProjectRun =>
    ({ firstHour, lastHour, presentation, progress, state, completion: null, cancellation: null, iterations: 0, ...extra });
  data.projects = [
    { id: 'done', start: { hour: 100, exact: false }, runs: [run(100, 159, 0, 40, 'running'), run(160, 199, 0, 100, 'completed', { completion: { hour: 160, exact: true } })] },
    { id: 'early', start: { hour: 100, exact: false }, runs: [run(100, 199, 1, 100, 'completed', { completion: { hour: 100, exact: false } })] },
    { id: 'beta', start: { hour: 120, exact: false }, runs: [run(120, 149, 2, 0, 'running'), run(150, 172, 2, 30, 'running'), run(173, 199, 2, 30, 'running', { iterations: 1 })] },
    { id: 'gamma', start: { hour: 110, exact: false }, runs: [run(110, 199, 3, 55, 'running')] },
    { id: 'delta', start: { hour: 100, exact: false }, runs: [run(100, 199, 4, 65, 'cancelled', { cancellation: { hour: 100, exact: false } })] },
  ];
  return validateSpecialProjectsReply(data);
}

test('Current lists running projects closest to completion first, then completed, the folded first-recorded group and cancelled', () => {
  const ids = (cards: readonly { project: { id: string } }[]) => cards.map(card => card.project.id);
  const late = currentProjects(projects(), 199);
  assert.deepEqual(ids(late.running), ['gamma', 'beta']);
  assert.deepEqual(ids(late.completed), ['done']);
  assert.deepEqual(ids(late.earlier), ['early']);
  assert.deepEqual(ids(late.cancelled), ['delta']);
  assert.equal(earlierBound(late.earlier), 100);
  const before = currentProjects(projects(), 159);
  assert.deepEqual(ids(before.running), ['gamma', 'done', 'beta'], 'progress orders running projects, not their ids');
  assert.deepEqual(ids(before.completed), []);
  const data = projects();
  assert.equal(completedWhenFirstRecorded(data.projects[0], data.projects[0].runs[1]), false, 'an exact completion inside the recording lists on its own row');
  assert.equal(completedWhenFirstRecorded(data.projects[1], data.projects[1].runs[0]), true);
  data.projects[0].runs[1].completion = { hour: 160, exact: false };
  assert.equal(completedWhenFirstRecorded(data.projects[0], data.projects[0].runs[1]), false, 'a completion bounded by a later read is still its own row');
  const tie = projects();
  tie.projects[2].runs = tie.projects[2].runs.map(run => ({ ...run, progress: 55 }));
  assert.deepEqual(ids(currentProjects(tie, 199).running), ['gamma', 'beta'], 'equal progress keeps the earlier start first');
});

test('History lanes run in start order, split at each progress step and fold projects that ended before the range into one line', () => {
  const history = projectHistory(projects(), 199);
  assert.deepEqual(history.lanes.map(lane => lane.id), ['done', 'gamma', 'beta']);
  const [done, gamma, beta] = history.lanes;
  assert.deepEqual(beta.spans.map(span => [span.firstHour, span.lastHour, span.label, span.shade]), [[120, 149, '0%', 0], [150, 199, '30%', 0.3]],
    'an iteration count change at equal progress keeps one segment');
  assert.deepEqual(gamma.spans.map(span => span.shade), [0.55]);
  assert.deepEqual(done.spans.map(span => [span.lastHour, span.positive, span.endLabel]), [[159, true, '✓']]);
  assert.deepEqual([history.completed, history.cancelled, history.by], [1, 1, 100]);
  assert.equal(closedLane('closed', history)?.label, `1 completed, 1 cancelled by ${dayLabel(100)}`);
  assert.equal(closedLane('closed', history)?.compact, true);
  const ranged = projectHistory(projects(), 199, undefined, 165);
  assert.deepEqual(ranged.lanes.map(lane => lane.id), ['gamma', 'beta'], 'a project that finished before the range gets no empty lane');
  assert.deepEqual([ranged.completed, ranged.cancelled, ranged.by], [2, 1, 160]);
  assert.equal(closedLane('closed', ranged)?.label, `2 completed, 1 cancelled by ${dayLabel(160)}`);
  assert.equal(closedLane('closed', projectHistory(fixture(), 199)), null);
});

test('Current renders again only when a section, order, name, scientist, state or date changes, never for progress alone', () => {
  const data = projects();
  assert.equal(currentKey(data, 149), currentKey(data, 150));
  assert.equal(currentKey(data, 172), currentKey(data, 173));
  assert.notEqual(currentKey(data, 159), currentKey(data, 160));
  const overtaking = projects();
  overtaking.projects[2].runs[2] = { ...overtaking.projects[2].runs[2], progress: 60 };
  assert.notEqual(currentKey(overtaking, 172), currentKey(overtaking, 173));
  assert.equal(currentKey(data, 200), 'unobserved', 'an hour not recorded shows no cards');
});

test('a running card paints its progress, bar and iterations at the hour, and nothing where the hour holds no run', () => {
  const data = projects();
  const beta = data.projects[2];
  assert.deepEqual(runningValues(beta, 149), { progress: '0%', fill: 0, iterations: '0 iterations' });
  assert.deepEqual(runningValues(beta, 180), { progress: '30%', fill: 0.3, iterations: '1 iteration' });
  assert.equal(runningValues(beta, 100), null);
  const backwards = { ...beta, runs: [{ ...beta.runs[0], progress: -20 }] };
  assert.equal(runningValues(backwards, 130)?.fill, 0, 'a negative prototype progress draws an empty bar, its percent as recorded');
  assert.equal(runningValues(backwards, 130)?.progress, '-20%');
});

test('History following the head keeps bars past the shown hour for the chart to cut, and renders again only at a start, an end or a new scientist', () => {
  const data = projects();
  const live = projectHistory(data, 159, undefined, -Infinity, true);
  const done = live.lanes.find(lane => lane.id === 'done')!;
  assert.deepEqual(done.spans.map(span => [span.firstHour, span.lastHour, span.positive]), [[100, 159, false]], 'a completion after the hour is not shown yet');
  const beta = live.lanes.find(lane => lane.id === 'beta')!;
  assert.deepEqual(beta.spans.map(span => [span.firstHour, span.lastHour]), [[120, 149], [150, 199]], 'the running bar reaches past the hour');
  assert.deepEqual(projectHistory(data, 159).lanes.find(lane => lane.id === 'beta')!.spans.map(span => span.lastHour), [149, 159]);
  assert.equal(historyKey(data, 150), historyKey(data, 159), 'a progress step renders nothing');
  assert.notEqual(historyKey(data, 159), historyKey(data, 160), 'a completion renders');
  assert.notEqual(historyKey(data, 119), historyKey(data, 120), 'a project starting renders');
  assert.deepEqual(projectHistory(data, 160, undefined, -Infinity, true).lanes.find(lane => lane.id === 'done')!.spans.map(span => [span.lastHour, span.positive, span.endLabel]),
    [[159, true, '✓']]);
});

test('review 4 finding 12: a pinned or asked card sets its own progress variable; a page-painted one leaves it to the installed texts', async () => {
  const { progressStyle } = await import('./specialProjectsData.ts');
  assert.deepEqual(progressStyle(50, false), { '--project-progress': 0.5 });
  assert.equal(progressStyle(50, true), undefined);
});
