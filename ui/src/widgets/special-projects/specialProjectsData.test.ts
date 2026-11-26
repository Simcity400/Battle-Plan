import { test } from 'node:test';
import assert from 'node:assert/strict';
import type { ProjectEntry, ProjectRuntimeDetails, SpecialProjectDetails, SpecialProjectsReply } from '../../generated/dashboard';
import { runtimeProject, shownCard, episodeBound, validateSpecialProjectsReply } from './specialProjectsData.ts';
import { OPTIONS, definition } from './definition.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import { dayLabel } from '../../charts/draw.ts';

const marker = (date: number, readHour: number, counted = true): ProjectEntry => ({ ordinal: 3, date, completion: true, readHour, counted, presentation: counted ? 0 : null });

const fixture = (): SpecialProjectsReply => ({ branchId: '1', tag: 'tag with spaces', unavailable: null, warnings: [], shown: { through: null, running: [], unknown: [], completed: [], undated: 0 },
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
test('View is also a setting, and removed options are ignored while current options load', () => {
  const view = OPTIONS.find(field => field.key === 'view')!;
  assert.equal(view.visibleWhen, undefined, 'View stays reachable when the header is narrow');
  for (const saved of ['live', 'history']) {
    const options = resolveOptions(OPTIONS, { view: saved, completedRetentionDays: 12, grow: true });
    assert.equal(options.view, saved);
    assert.equal('completedRetentionDays' in options, false);
    assert.equal('grow' in options, false, 'the timeline family sets the History axis; grow is no longer an option');
  }
  assert.equal(resolveOptions(OPTIONS, { view: 'retired-view', completedRetentionDays: 30 }).view, 'live');
  assert.equal(OPTIONS.some(field => field.key === 'completedRetentionDays'), false);
  assert.equal(definition.version, 2);
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

test('X-S9: the pop-up joins the Projects read that supplied its card, though it precedes the published hours', () => {
  const data = fixture();
  data.projects[0] = { ...data.projects[0], statuses: [{ ...data.projects[0].statuses[0], firstHour: 101, readHour: 100 }], firstRead: 100 };
  data.available = [{ firstHour: 101, lastHour: 199 }];
  const card = shownCard(data, 120, 'episode')!;
  const read: ProjectRuntimeDetails = { owner: 9, id: 3, label: { token: 'mod_project', name: null }, readHour: 100, pendingKnown: true, pending: null, seed: null, variables: null, choices: null, historyReadHour: null };
  const joined = runtimeProject({ hour: 120, scientists: null, programs: null, breakthroughs: null, projects: [read], unavailable: [], warnings: [] }, card, 120);
  assert.ok(joined && typeof joined === 'object', `${joined}`);
});


test('History lanes come from each country’s recorded statuses: one bar per working stretch, cut at the hour, clipped to membership, a heading per country', async () => {
  const { specialProjectsHistoryProjection } = await import('./historyProjection.ts');
  const { createBoundedReadView } = await import('../../families/boundedView.ts');
  const status = (firstHour: number, lastHour: number, progress: number, state: 'running' | 'paused' | 'completed') =>
    ({ firstHour, lastHour, readHour: firstHour, presentation: 0, progress, state, iterations: null, scientistRead: null, lastScientistRead: null });
  const reply = (tag: string, statuses: ReturnType<typeof status>[]) => ({
    ...fixture(), tag, identities: [{ firstHour: 0, lastHour: 300, slot: 5, countryId: 10 }],
    available: [{ firstHour: 0, lastHour: 39 }, { firstHour: 45, lastHour: 200 }], partial: [],
    projects: [{ id: `${tag}:p`, owner: 1, local: 0, firstRead: statuses[0]!.firstHour, firstState: 'running' as const, statuses, entries: [], attempts: [] }],
  });
  const a = reply('AAA', [status(10, 29, 20, 'running'), status(30, 39, 40, 'running'), status(45, 99, 40, 'running'), status(100, 200, 100, 'completed')]);
  const b = reply('BBB', [status(10, 39, 50, 'running'), status(45, 200, 50, 'running')]);
  const panels = new Map<string, unknown>([['projects:AAA', a], ['projects:BBB', b]]);
  const options = { tags: ['AAA', 'BBB'], membership: { BBB: [[20, 30]] as const } as Record<string, readonly (readonly [number, number])[]>, firstHour: 0, branchFirstHour: 0, surface: '#111111' };
  const model = specialProjectsHistoryProjection.project(createBoundedReadView({ hour: 60, panels }), options);
  assert.deepEqual(model.lanes.map(lane => [lane.kind, lane.key]), [['heading', 'country:AAA'], ['item', 'AAA:AAA:p'], ['heading', 'country:BBB'], ['item', 'BBB:BBB:p']]);
  const own = model.lanes[1]!;
  assert.deepEqual(own.spans.map(span => [span.firstHour, span.lastHour, span.open]), [[10, 39, false], [45, 60, true]], 'one bar per working stretch, a gap left empty, cut at the hour');
  assert.equal(own.subtitle, 'In progress · 40%');
  assert.deepEqual(model.lanes[3]!.spans.map(span => [span.firstHour, span.lastHour]), [[20, 30]], 'a country is drawn only while a member');
  assert.deepEqual(model.unobserved.map(span => [span.firstHour, span.lastHour]), [[40, 44], [40, 44]], 'an unrecorded stretch is a gap, never no projects');
  const later = specialProjectsHistoryProjection.project(createBoundedReadView({ hour: 150, panels: new Map([['projects:AAA', a]]) }), { ...options, tags: ['AAA'], membership: null });
  assert.equal(later.lanes.length, 1, 'one country has no heading');
  assert.equal(later.lanes[0]!.subtitle?.startsWith('Completed'), true);
  assert.equal(specialProjectsHistoryProjection.nextChange!(createBoundedReadView({ hour: 60, panels }), options), 100, 'quiet until the next recorded status');
});

test('R2-S6: History clips ended statuses, dates earlier span names and tooltip titles, and looks up slot-keyed hovers', async () => {
  const { specialProjectsHistoryProjection } = await import('./historyProjection.ts');
  const { createBoundedReadView } = await import('../../families/boundedView.ts');
  const base = fixture();
  const slotKey = 'slot:5';
  const reply: SpecialProjectsReply = {
    ...base,
    tag: 'AAA',
    identities: [{ firstHour: 0, lastHour: 100, slot: 5, countryId: 10 }],
    available: [{ firstHour: 0, lastHour: 49 }, { firstHour: 60, lastHour: 80 }],
    partial: [],
    unresolved: [],
    presentations: [
      { ...base.presentations[0]!, name: 'Old Project' },
      { ...base.presentations[0]!, name: 'New Project' },
    ],
    projects: [
      {
        id: 'renamed',
        owner: 1,
        local: 0,
        firstRead: 10,
        firstState: 'running',
        statuses: [
          { firstHour: 10, lastHour: 29, readHour: 10, presentation: 0, progress: 20, state: 'running', iterations: null, scientistRead: null, lastScientistRead: null },
          { firstHour: 30, lastHour: 49, readHour: 30, presentation: 1, progress: 40, state: 'running', iterations: null, scientistRead: null, lastScientistRead: null },
        ],
        entries: [],
        attempts: [],
      },
    ],
  };
  const panels = new Map<string, unknown>([[`projects:${slotKey}`, reply]]);
  const options = { tags: [slotKey], membership: { [slotKey]: [[0, 100]] as const }, firstHour: 0, branchFirstHour: 0, surface: '#111111' };

  const unreadView = createBoundedReadView({ hour: 55, panels });
  const unreadModel = specialProjectsHistoryProjection.project(unreadView, options);
  const unreadLane = unreadModel.lanes[0]!;
  assert.equal(unreadLane.subtitle, 'Not recorded', 'unread hour after last status must not carry In progress · 40%');
  assert.deepEqual(unreadLane.spans.map(s => [s.firstHour, s.lastHour, s.label]), [
    [10, 29, 'Old Project'],
    [30, 49, 'New Project'],
  ], 'earlier span keeps its dated presentation');

  const vanishedView = createBoundedReadView({ hour: 70, panels });
  const vanishedModel = specialProjectsHistoryProjection.project(vanishedView, options);
  const vanishedLane = vanishedModel.lanes[0]!;
  assert.equal(vanishedLane.subtitle, 'Not in progress', 'vanished project after status end must not carry In progress · 40%');

  const earlyTip = specialProjectsHistoryProjection.tooltip!(vanishedModel, { kind: 'span', laneKey: vanishedLane.key, spanId: vanishedLane.spans[0]!.id, hour: 15 }, vanishedView, options);
  assert.equal(earlyTip?.title, 'Old Project', 'tooltip title dates to the hovered span');
  assert.deepEqual(earlyTip?.items?.slice(0, 2), [
    { label: 'State', value: 'In progress' },
    { label: 'Progress', value: '20%' },
  ], 'slot-keyed hover resolves the planned panel entry');

  const renamedTip = specialProjectsHistoryProjection.tooltip!(vanishedModel, { kind: 'span', laneKey: vanishedLane.key, spanId: vanishedLane.spans[1]!.id, hour: 35 }, vanishedView, options);
  assert.equal(renamedTip?.title, 'New Project');
  assert.equal(renamedTip?.items?.[2]?.value, earlyTip?.items?.[2]?.value, 'renaming mid-run keeps the working stretch start date');

  const beforeTip = specialProjectsHistoryProjection.tooltip!(vanishedModel, { kind: 'lane', laneKey: vanishedLane.key, hour: 5 }, vanishedView, options);
  assert.equal(beforeTip?.title, 'Special project', 'hovering before the first status does not reveal a later project name');

  const ongoingGapTip = specialProjectsHistoryProjection.tooltip!(unreadModel, { kind: 'lane', laneKey: unreadLane.key, hour: 58 }, unreadView, options);
  assert.deepEqual(ongoingGapTip?.items, [{ label: 'State', value: 'Not recorded' }], 'reused model inside an ongoing gap still checks coverage at the hovered hour');

  const gapTip = specialProjectsHistoryProjection.tooltip!(vanishedModel, { kind: 'lane', laneKey: vanishedLane.key, hour: 55 }, vanishedView, options);
  assert.equal(gapTip?.title, 'New Project');
  assert.deepEqual(gapTip?.items, [{ label: 'State', value: 'Not recorded' }]);

  const afterTip = specialProjectsHistoryProjection.tooltip!(vanishedModel, { kind: 'lane', laneKey: vanishedLane.key, hour: 70 }, vanishedView, options);
  assert.deepEqual(afterTip?.items, [{ label: 'State', value: 'Not in progress' }]);

  const emptyBase: SpecialProjectsReply = { ...base, identities: [{ firstHour: 0, lastHour: 100, slot: 5, countryId: 10 }], partial: [], unresolved: [], projects: [] };
  const emptyRecorded: SpecialProjectsReply = { ...emptyBase, tag: 'AAA', available: [{ firstHour: 0, lastHour: 80 }] };
  const emptyUnread: SpecialProjectsReply = { ...emptyBase, tag: 'BBB', available: [{ firstHour: 0, lastHour: 40 }] };
  const mixedPanels = new Map<string, unknown>([['projects:AAA', emptyRecorded], ['projects:BBB', emptyUnread]]);
  const identity = {
    resolve: (kind: 'country', key: string, atHour: number) => ({
      kind, key, atHour,
      holder: { status: 'ready' as const, value: key },
      tag: { status: 'ready' as const, value: key },
      name: { status: 'ready' as const, value: key },
      flagChoice: { status: 'absent' as const, reason: 'No flag' },
      validInterval: [0, 100] as [number, number],
      nextChangeAt: Infinity,
    }),
  } as Parameters<typeof createBoundedReadView>[0]['identity'];
  const mixed = specialProjectsHistoryProjection.project(createBoundedReadView({ hour: 50, panels: mixedPanels, identity }), {
    tags: ['AAA', 'BBB'], membership: null, firstHour: 0, branchFirstHour: 0, surface: '#111111',
  });
  assert.equal(mixed.empty, undefined, 'mixed coverage leaves per-country headings visible instead of a global empty message');
  assert.deepEqual(mixed.lanes.map(lane => [lane.key, lane.label]), [
    ['country:AAA', 'AAA · No special projects yet'],
    ['country:BBB', 'BBB · Not recorded at this date'],
  ], 'each empty country heading shows its own coverage status in its visible label');
  const bothUnread = specialProjectsHistoryProjection.project(createBoundedReadView({ hour: 50, panels: new Map([['projects:BBB', emptyUnread]]) }), {
    tags: ['BBB'], membership: null, firstHour: 0, branchFirstHour: 0, surface: '#111111',
  });
  assert.equal(bothUnread.emptyKind, 'unavailable', 'earlier coverage does not mask an unread displayed hour');
});
