import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView, displayedHour } from '../boundedView.ts';
import type { ArtProvider, BoundedReadView, IdentityProvider, IdentityResult } from '../plan.ts';
import { drawFamilyTimeline, hitFamilyTimeline } from './painter.ts';
import { layoutTimeline } from './layout.ts';
import { deriveTimelineAudit } from './model.ts';
import { ideaTimelineProjection } from '../../widgets/idea-timeline/ideaTimelineProjection.ts';
import { spiritHistoryProjection } from '../../widgets/spirit-history/spiritHistoryProjection.ts';
import { factionsProjection } from '../../widgets/factions/factionsProjection.ts';
import { subjectsProjection } from '../../widgets/subjects/subjectsProjection.ts';
import { diplomaticPactsProjection } from '../../widgets/diplomatic-pacts/diplomaticPactsProjection.ts';
import { civilWarsProjection } from '../../widgets/civil-wars/civilWarsProjection.ts';
import { warsProjection } from '../../widgets/wars/warsProjection.ts';
import { autonomyProjection } from '../../widgets/autonomy/autonomyProjection.ts';
import { generalCareerProjection } from '../../widgets/general-career/generalCareerProjection.ts';
import { factionUpgradesProjection } from '../../widgets/faction-upgrades/factionUpgradesProjection.ts';
import type { FamilyTheme } from '../contract.ts';

function fakeIdentity(names: Record<string, string>, colors: Record<string, string> = {}): IdentityProvider {
  return {
    demandOwner: 'view',
    resolve(kind, key, atHour): IdentityResult {
      return {
        kind,
        key,
        atHour,
        holder: { status: 'ready', value: key },
        tag: { status: 'ready', value: key },
        name: { status: 'ready', value: names[key] ?? key },
        colour: colors[key] ? { status: 'ready', value: colors[key]! } : { status: 'absent', reason: 'none' },
        flagChoice: { status: 'ready', value: `${key}:neutral` },
        validInterval: [0, 10_000],
        nextChangeAt: Infinity,
      };
    },
  };
}

const fakeArt: ArtProvider = {
  demandOwner: 'view',
  resolve() {
    return { status: 'pending' };
  },
};

function makeReadView(
  hour: number,
  panels: Record<string, unknown>,
  options?: {
    readonly names?: Record<string, string>;
    readonly colors?: Record<string, string>;
    readonly hover?: Record<string, { readonly hour: number; readonly data: unknown }>;
  },
): BoundedReadView {
  return createBoundedReadView({
    hour: displayedHour(hour),
    panels: new Map(Object.entries(panels)),
    hovers: options?.hover ? new Map(Object.entries(options.hover).map(([k, v]) => [k, { hour: displayedHour(v.hour), data: v.data }])) : undefined,
    identity: fakeIdentity(options?.names ?? { GER: 'Germany', ENG: 'United Kingdom', SOV: 'Soviet Union', POL: 'Poland', SPR: 'Spain', D01: 'Nationalist Spain', RAJ: 'British Raj', CAN: 'Canada' }, options?.colors),
    art: fakeArt,
  });
}

function makeFakeCtx() {
  const calls: string[] = [];
  const ctx = {
    font: '',
    fillStyle: '',
    strokeStyle: '',
    lineWidth: 1,
    globalAlpha: 1,
    textAlign: 'left',
    textBaseline: 'alphabetic',
    save() { calls.push('save'); },
    restore() { calls.push('restore'); },
    beginPath() { calls.push('beginPath'); },
    closePath() {},
    rect() {},
    clip() {},
    moveTo() {},
    lineTo() {},
    quadraticCurveTo() {},
    arc() {},
    roundRect() {},
    fill() { calls.push('fill'); },
    stroke() { calls.push('stroke'); },
    fillRect() { calls.push('fillRect'); },
    strokeRect() {},
    clearRect() {},
    setLineDash() {},
    createLinearGradient() { return { addColorStop() {} }; },
    fillText(t: string) { calls.push(`text:${t}`); },
    measureText(t: string) { return { width: t.length * 6.5 }; },
    drawImage() {},
  };
  return { ctx: ctx as never, calls };
}

const DARK_THEME: FamilyTheme = {
  surface: '#16181d',
  text: '#e6edf3',
  muted: '#8b949e',
  line: '#30363d',
  lineSoft: '#262c34',
  accent: '#58a6ff',
  blue: '#58a6ff',
  red: '#e5534b',
  amber: '#d29922',
  green: '#3fb950',
  grey: '#6e7681',
  hatch: '#30363d',
  font: 'system-ui, sans-serif',
  fontSize: { small: 11, body: 12 },
  lineWidth: 1,
  hatchSpacing: 6,
  country: {},
};

test('idea-timeline clips open spans at playhead, hides future spans, and respects iconsOnBars', () => {
  const reply = {
    tag: 'GER',
    firstHour: 100,
    lastHour: 500,
    recorded: [{ firstHour: 100, lastHour: 500 }],
    unobserved: [],
    warnings: [],
    ideas: {
      early_mob: { token: 'early_mob', name: 'Early Mobilisation', category: 'economy', artwork: { path: 'asset://idea/early_mob.png' } },
      war_econ: { token: 'war_econ', name: 'War Economy', category: 'economy', artwork: { path: 'asset://idea/war_econ.png' } },
      total_mob: { token: 'total_mob', name: 'Total Mobilisation', category: 'economy', artwork: { path: 'asset://idea/total_mob.png' } },
    },
    spans: [
      { idea: 'early_mob', category: 'economy', firstHour: 100, lastHour: 150, heldBefore: true },
      { idea: 'war_econ', category: 'economy', firstHour: 151, lastHour: 350, heldBefore: false },
      { idea: 'total_mob', category: 'economy', firstHour: 351, lastHour: 500, heldBefore: false },
    ],
  };

  const viewAt200 = makeReadView(200, { main: reply });
  const withIcons = ideaTimelineProjection.project(viewAt200, {
    category: 'laws',
    iconsOnBars: true,
    branchFirst: 100,
    surface: DARK_THEME.surface,
  });
  assert.equal(withIcons.mode, 'lanes');
  assert.equal(withIcons.lanes?.length, 1);
  const spans = withIcons.lanes![0]!.spans!;
  assert.equal(spans.length, 2, 'future span starting at 351 must not appear at hour 200');
  assert.equal(spans[0]!.fadedStart, true);
  assert.equal(spans[1]!.lastHour, 200, 'open span must end at displayed hour 200');
  assert.equal(spans[1]!.openAtPlayhead, true);
  assert.ok(spans[1]!.icon, 'iconsOnBars=true includes bar icon handle');

  const withoutIcons = ideaTimelineProjection.project(viewAt200, {
    category: 'laws',
    iconsOnBars: false,
    branchFirst: 100,
    surface: DARK_THEME.surface,
  });
  assert.equal(withoutIcons.lanes![0]!.spans![1]!.icon, undefined, 'iconsOnBars=false hides bar icon handle');
});

test('spirit-history clips open spirits at displayed hour and omits future spirits', () => {
  const reply = {
    tag: 'GER',
    firstHour: 100,
    lastHour: 400,
    recorded: [{ firstHour: 100, lastHour: 400 }],
    unobserved: [],
    warnings: [],
    ideas: {
      mefo: { token: 'mefo', name: 'MEFO Bills', category: 'country', artwork: { path: 'asset://spirit/mefo.png' } },
      future_spirit: { token: 'future_spirit', name: 'Future Spirit', category: 'country', artwork: null },
    },
    spans: [
      { idea: 'mefo', category: 'country', firstHour: 100, lastHour: 300, heldBefore: true },
      { idea: 'future_spirit', category: 'country', firstHour: 260, lastHour: 400, heldBefore: false },
    ],
  };

  const model = spiritHistoryProjection.project(makeReadView(200, { main: reply }), {
    sort: 'gained',
    showEnded: true,
    iconsOnBars: true,
    branchFirst: 100,
    surface: DARK_THEME.surface,
  });
  assert.equal(model.lanes?.length, 1);
  assert.equal(model.lanes![0]!.label, 'MEFO Bills');
  assert.equal(model.lanes![0]!.spans![0]!.lastHour, 200);
  assert.equal(model.lanes![0]!.spans![0]!.openAtPlayhead, true);
  assert.ok(model.lanes![0]!.spans![0]!.icon);
});

test('factions, subjects, and diplomatic-pacts never leak spans past displayed hour', () => {
  const factionsReply = {
    branchId: '1',
    firstHour: 100,
    lastHour: 500,
    unobserved: [],
    warnings: [],
    factions: [
      { id: 'axis', firstHour: 100, lastHour: 500, names: [{ name: 'Axis', firstHour: 100, lastHour: 500 }], leaders: [{ tag: 'GER', firstHour: 100, lastHour: 500 }], members: [{ tag: 'GER', firstHour: 100, lastHour: 500 }] },
      { id: 'future', firstHour: 300, lastHour: 500, names: [{ name: 'Future Pact', firstHour: 300, lastHour: 500 }], leaders: [{ tag: 'SOV', firstHour: 300, lastHour: 500 }], members: [{ tag: 'SOV', firstHour: 300, lastHour: 500 }] },
    ],
  };
  const fModel = factionsProjection.project(makeReadView(200, { main: factionsReply }), {
    order: 'members',
    members: 'joined',
    showEnded: true,
    selectedTags: null,
    branchFirst: 100,
    surface: DARK_THEME.surface,
  });
  assert.equal(fModel.lanes?.length, 1);
  assert.equal(fModel.lanes![0]!.label, 'Axis');
  assert.equal(fModel.lanes![0]!.spans![0]!.lastHour, 200);
  assert.equal(fModel.lanes![0]!.spans![0]!.openAtPlayhead, true);

  const subjectsReply = {
    branchId: '1',
    firstHour: 100,
    lastHour: 500,
    recorded: [{ firstHour: 100, lastHour: 500 }],
    unobserved: [],
    warnings: [],
    levels: { colony: { token: 'colony', name: 'Colony' } },
    runs: [
      { subject: 'RAJ', overlord: 'ENG', level: 'colony', firstHour: 100, lastHour: 450 },
      { subject: 'CAN', overlord: 'ENG', level: 'colony', firstHour: 320, lastHour: 450 },
    ],
  };
  const sModel = subjectsProjection.project(makeReadView(200, { main: subjectsReply }), {
    order: 'subjects',
    subjects: 'since',
    showEnded: true,
    showLevelNames: true,
    selectedTags: null,
    branchFirst: 100,
    surface: DARK_THEME.surface,
  });
  assert.equal(sModel.lanes?.length, 1);
  assert.equal(sModel.lanes![0]!.spans![0]!.lastHour, 200);

  const pactsReply = {
    branchId: '1',
    firstHour: 100,
    lastHour: 500,
    recorded: [{ firstHour: 100, lastHour: 500 }],
    unobserved: [],
    warnings: [],
    pacts: [
      { kind: 'non_aggression_pact', a: 'GER', b: 'SOV', firstHour: 120, lastHour: 400 },
      { kind: 'guarantee', a: 'ENG', b: 'POL', firstHour: 280, lastHour: 400 },
    ],
  };
  const pModel = diplomaticPactsProjection.project(makeReadView(200, { main: pactsReply }), {
    countryTag: 'GER',
    kinds: ['non_aggression_pact', 'guarantee'],
    showEnded: true,
    branchFirst: 100,
    surface: DARK_THEME.surface,
  });
  assert.equal(pModel.lanes?.length, 1);
  assert.equal(pModel.lanes![0]!.spans![0]!.lastHour, 200);
  assert.equal(pModel.lanes![0]!.spans![0]!.openAtPlayhead, true);
});

test('civil-wars shows recorded title, dimmed losers, and winner in outcome column', () => {
  const reply = {
    branchId: '1',
    firstHour: 100,
    lastHour: 500,
    known: [{ firstHour: 100, lastHour: 500 }],
    unobserved: [],
    warnings: [],
    wars: [
      {
        id: 'cw-1',
        name: 'Spanish Civil War',
        firstHour: 110,
        lastHour: 220,
        ended: true,
        sides: [
          { tag: 'D01', winner: true, firstHour: 110, lastHour: 220 },
          { tag: 'SPR', winner: false, firstHour: 110, lastHour: 220 },
        ],
      },
      {
        id: 'cw-future',
        name: 'Future Civil War',
        firstHour: 350,
        lastHour: 450,
        ended: false,
        sides: [
          { tag: 'GER', winner: false, firstHour: 350, lastHour: 450 },
          { tag: 'POL', winner: false, firstHour: 350, lastHour: 450 },
        ],
      },
    ],
  };

  const model = civilWarsProjection.project(makeReadView(250, { main: reply }), {
    selectedTags: null,
    showEnded: true,
    branchFirst: 100,
    branchGaps: [],
    surface: DARK_THEME.surface,
  });
  assert.equal(model.lanes?.length, 1);
  const lane = model.lanes![0]!;
  assert.equal(lane.label, 'Spanish Civil War');
  assert.equal(lane.matchup?.outcome.status, 'succeeded');
  assert.equal(lane.matchup?.defenders.lead.dimmed, true);
});

test('wars projects Wars and Countries views with split bars, expandable rows, and hover losses', () => {
  const reply = {
    branchId: '1',
    firstHour: 100,
    lastHour: 500,
    recorded: [{ firstHour: 100, lastHour: 500 }],
    unobserved: [],
    warnings: [],
    wars: [
      {
        id: 'w-1',
        name: 'German-Polish War',
        firstHour: 120,
        lastHour: 400,
        ended: false,
        participants: [
          { tag: 'GER', side: 'attackers', major: true, firstHour: 120, lastHour: 400, outcome: 'fighting' },
          { tag: 'POL', side: 'defenders', major: true, firstHour: 120, lastHour: 190, outcome: 'capitulated' },
          { tag: 'ENG', side: 'defenders', major: true, firstHour: 130, lastHour: 400, outcome: 'fighting' },
        ],
      },
    ],
  };
  const losses = {
    branchId: '1',
    hour: 200,
    wars: [{ id: 'w-1', attackers: [{ tag: 'GER', losses: 12_000 }], defenders: [{ tag: 'POL', losses: 45_000 }, { tag: 'ENG', losses: 3_000 }] }],
  };
  const rv = makeReadView(200, { main: reply }, { hover: { 'war-losses': { hour: 200, data: losses } } });
  const warsModel = warsProjection.project(rv, {
    view: 'gantt',
    colour: 'war',
    branchFirst: 100,
    surface: DARK_THEME.surface,
    surrenderIconUrl: null,
  }, { 'war:w-1': true });
  assert.ok(warsModel.lanes && warsModel.lanes.length >= 2, 'expanded war includes child participant rows');
  const parent = warsModel.lanes![0]!;
  assert.equal(parent.expandable, true);
  assert.equal(parent.spans![0]!.lastHour, 200);
  assert.equal(parent.spans![0]!.openAtPlayhead, true);

  const countriesModel = warsProjection.project(rv, {
    view: 'countries',
    colour: 'side',
    branchFirst: 100,
    surface: DARK_THEME.surface,
    surrenderIconUrl: null,
  });
  assert.ok(countriesModel.lanes && countriesModel.lanes.length >= 2);
});

test('autonomy supports both Meters (A, sorted closest to threshold) and Over time (B) views', () => {
  const subjectsReply = {
    branchId: '1',
    firstHour: 100,
    lastHour: 500,
    recorded: [{ firstHour: 100, lastHour: 500 }],
    unobserved: [],
    warnings: [],
    levels: { colony: { token: 'colony', name: 'Colony' } },
    runs: [
      { subject: 'RAJ', overlord: 'ENG', level: 'colony', firstHour: 100, lastHour: 500 },
      { subject: 'CAN', overlord: 'ENG', level: 'colony', firstHour: 100, lastHour: 500 },
    ],
  };
  const samples = {
    RAJ: { tag: 'RAJ', points: [[100, 0.45], [200, 0.5]] as readonly (readonly [number, number | null])[], current: 0.5 },
    CAN: { tag: 'CAN', points: [[100, 0.8], [200, 0.92]] as readonly (readonly [number, number | null])[], current: 0.92 },
  };
  const rv = makeReadView(200, { main: subjectsReply });

  const metersModel = autonomyProjection.project(rv, {
    view: 'meters',
    bySubject: false,
    selectedTags: null,
    branchFirst: 100,
    surface: DARK_THEME.surface,
    seriesSamples: samples,
  });
  assert.equal(metersModel.mode, 'meters');
  assert.equal(metersModel.meters?.length, 2);
  assert.equal(metersModel.meters![0]!.label, 'Canada', 'closest to threshold (0.92 vs 0.50) sorts first');

  const historyModel = autonomyProjection.project(rv, {
    view: 'history',
    bySubject: false,
    selectedTags: null,
    branchFirst: 100,
    surface: DARK_THEME.surface,
    seriesSamples: samples,
  });
  assert.equal(historyModel.mode, 'series');
  assert.equal(historyModel.series?.lines.length, 2);
});

test('general-career renders Timeline, Dossier, and Record forms without leaking future events', () => {
  const role = { owner: 'GER', kind: 1, category: 0, rank: 'Field Marshal', attributeNames: ['Attack', 'Defense', 'Planning', 'Logistics'], maximumSkill: 9 };
  const career = {
    reference: '1',
    name: 'Gerd von Rundstedt',
    portraitHour: 100,
    runs: [{ firstHour: 100, lastHour: 500, role, skill: 4, components: [3, 4, 4, 3] }],
    traitGains: [[150, 0, 1], [350, 1, 1]],
    traits: [
      { token: 'offensive_doctrine', name: 'Offensive Doctrine', icon: null, missing: null },
      { token: 'future_trait', name: 'Future Trait', icon: null, missing: null },
    ],
    commands: [
      { firstHour: 110, lastHour: 400, formation: '1', kind: 'army_group', name: 'Army Group South' },
      { firstHour: 320, lastHour: 450, formation: '2', kind: 'army', name: 'Future Army' },
    ],
    battles: [
      { id: 'b1', firstHour: 140, lastHour: 160, province: 1, result: 'won' },
      { id: 'b2', firstHour: 310, lastHour: 330, province: 2, result: 'lost' },
    ],
    observed: [[100, 500]],
    battlesPartialFrom: null,
    resultsMissing: '',
    missing: null,
    warnings: [],
  };
  const rv = makeReadView(200, { career });

  const timeline = generalCareerProjection.project(rv, { view: 'timeline', reference: '1', branchFirst: 100, surface: DARK_THEME.surface, portraitUrl: null, portraitMissing: false });
  assert.equal(timeline.mode, 'lanes');
  assert.ok(timeline.headerCard);
  assert.equal(timeline.headerCard!.name, 'Gerd von Rundstedt');
  const allSpans = timeline.lanes!.flatMap(l => l.spans ?? []);
  assert.ok(allSpans.every(s => s.lastHour <= 200), 'no command/trait span extends past displayed hour 200');

  const dossier = generalCareerProjection.project(rv, { view: 'dossier', reference: '1', branchFirst: 100, surface: DARK_THEME.surface, portraitUrl: null, portraitMissing: false });
  assert.equal(dossier.mode, 'meters');
  assert.ok(dossier.meters && dossier.meters.length >= 2);

  const record = generalCareerProjection.project(rv, { view: 'record', reference: '1', branchFirst: 100, surface: DARK_THEME.surface, portraitUrl: null, portraitMissing: false });
  assert.equal(record.mode, 'sections');
  assert.ok(record.sections && record.sections.length >= 2);
});

test('faction-upgrades projects Rules and Upgrades side-by-side with Since {date} and hides future items', () => {
  const reply = {
    branchId: '1',
    firstHour: 100,
    lastHour: 500,
    warnings: [],
    rules: [
      { id: 'r1', name: 'Joining Rule', group: 'Joining', firstHour: 100, lastHour: 500 },
      { id: 'r2', name: 'Future Rule', group: 'War', firstHour: 350, lastHour: 500 },
    ],
    upgrades: [
      { id: 'u1', name: 'Joint Research', group: 'Research', firstHour: 120, lastHour: 500 },
      { id: 'u2', name: 'Ended Upgrade', group: 'Industry', firstHour: 100, lastHour: 180 },
    ],
  };
  const rv = makeReadView(200, { main: reply });
  const shown = factionUpgradesProjection.project(rv, { factionName: 'Allies', endedProgrammes: true, branchFirst: 100 });
  assert.equal(shown.mode, 'sections');
  assert.equal(shown.sections?.length, 2);
  assert.equal(shown.sections![0]!.items.length, 1);
  assert.match(shown.sections![0]!.items[0]!.rightText, /^Since /);
  assert.equal(shown.sections![1]!.items.length, 2);

  const hideEnded = factionUpgradesProjection.project(rv, { factionName: 'Allies', endedProgrammes: false, branchFirst: 100 });
  assert.equal(hideEnded.sections![1]!.items.length, 1);
});

test('layout, painter, hit-testing, and audit derivation work cleanly across modes', () => {
  const reply = {
    tag: 'GER',
    firstHour: 100,
    lastHour: 500,
    recorded: [{ firstHour: 100, lastHour: 500 }],
    unobserved: [],
    warnings: [],
    ideas: { war_econ: { token: 'war_econ', name: 'War Economy', category: 'economy', artwork: null } },
    spans: [{ idea: 'war_econ', category: 'economy', firstHour: 100, lastHour: 300, heldBefore: false }],
  };
  const rv = makeReadView(200, { main: reply });
  const model = ideaTimelineProjection.project(rv, { category: 'laws', iconsOnBars: true, branchFirst: 100, surface: DARK_THEME.surface });
  const layout = layoutTimeline(model, 520, 260, {});
  const { ctx, calls } = makeFakeCtx();
  drawFamilyTimeline(ctx, { model, layout, readView: rv, theme: DARK_THEME, scrollTop: 0, expanded: {} });
  assert.ok(calls.length > 0, 'painter issues draw operations');
  const laneRect = layout.lanes[0]!;
  const hit = hitFamilyTimeline(model, layout, { x: layout.plotLeft + 20, y: laneRect.y + laneRect.height / 2 }, 0);
  assert.ok(hit && hit.kind === 'span', `expected span hit, got ${JSON.stringify(hit)}`);
  const audit = deriveTimelineAudit(model, {});
  assert.ok(audit.length >= 1 && audit[0]!.row === audit[0]!.key);
});
