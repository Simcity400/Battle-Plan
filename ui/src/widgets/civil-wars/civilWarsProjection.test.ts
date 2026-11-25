import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { countryKey } from '../../dashboard/countryKey.ts';
import { datedIdentity } from '../../families/identity/dated.fixture.ts';
import { assertQuietFrames, LANE_THEME } from '../../families/laneRuntime.fixture.ts';
import { createLaneView } from '../../families/laneRuntime.ts';
import { nextDayStart, timelineLane, type FamilyTimelineBinding } from '../../families/timeline/laneView.ts';
import type { CivilWar, CivilWarSide, CivilWarsReply, HolderTag, HourSpan, SideExit } from '../../generated/dashboard';
import {
  civilWarsDeclaration,
  civilWarsProjection,
  civilWarsProof,
  sideTree,
  validateCivilWarsReply,
  type CivilWarsProjectionOptions,
} from './civilWarsProjection.ts';

const tags = (...entries: [number, string][]): HolderTag[] => entries.map(([hour, tag]) => ({ hour, tag }));
const side = (
  slot: number,
  holderTags: HolderTag[],
  firstHour: number,
  lastHour: number,
  parent: number | null = null,
  existedBefore = false,
  exit: SideExit | null = null,
): CivilWarSide => ({ slot, tags: holderTags, firstHour, lastHour, parent, existedBefore, exit });

const war = (
  id: string,
  firstHour: number,
  lastHour: number,
  sides: CivilWarSide[],
  outcome: CivilWar['outcome'] = null,
  atWar: HourSpan[] = [{ firstHour, lastHour }],
): CivilWar => ({ id, firstHour, lastHour, atWar, sides, outcome });

const reply = (
  hour: number,
  wars: CivilWar[],
  extra: Partial<CivilWarsReply> = {},
): CivilWarsReply => ({
  branchId: '1',
  hour,
  nextChange: null,
  known: [{ firstHour: 0, lastHour: hour }],
  wars,
  warnings: [],
  ...extra,
});

const OPTIONS: CivilWarsProjectionOptions = {
  showEnded: true,
  firstHour: 0,
  surface: '#161616',
  selected: null,
  holders: null,
  membership: null,
  countryReason: null,
};

const threeSided = war('w1', 100, 400, [
  side(1, tags([0, 'SPA']), 100, 400, null, true),
  side(2, tags([100, 'SPR']), 100, 300, 0, false, { kind: 'annexedBySide', hour: 301, readHour: 305, by: 'SPA' }),
  side(3, tags([200, 'SPC']), 200, 350, 1, false, { kind: 'annexedBySide', hour: 351, readHour: 360, by: 'SPA' }),
], { winner: 0, hour: 351, readHour: 360 });

test('a won civil war shows title above the matchup pair, +N extra sides, vertical-bar outcome, and expandable side tree', () => {
  const identity = datedIdentity({
    SPA: [{ from: 0, name: 'Nationalist Spain', colour: [216, 155, 68] }],
    SPR: [{ from: 0, name: 'Spain', colour: [143, 114, 199] }],
    SPC: [{ from: 0, name: 'Carlist Spain', colour: [201, 76, 76] }],
  });
  const data = validateCivilWarsReply(reply(1000, [threeSided]));
  const model = civilWarsProjection.project(
    createBoundedReadView({ hour: 1000, identity, panels: new Map([['main', data]]) }),
    OPTIONS,
  );
  assert.equal(model.summaryText, '1 civil war · 0 ongoing');
  assert.equal(model.lanes.length, 4);
  const head = model.lanes[0]!;
  assert.equal(head.label, 'Civil war in Nationalist Spain');
  assert.equal(head.sidesBelowLabel, true);
  assert.equal(head.sidesSeparator, 'v');
  assert.equal(head.extraSides, 1);
  assert.deepEqual(head.sides?.map(s => s.name), ['Nationalist Spain', 'Spain']);
  assert.equal(head.outcome?.mark, 'bar');
  assert.equal(head.outcome?.text, 'Nationalist Spain won');
  assert.equal(head.expandable, true);
  assert.equal(head.defaultOpen, false);
  assert.equal(head.spans[0]!.endCap, true);
  assert.equal(head.spans[0]!.halves?.length, 2);
  assert.deepEqual(
    model.lanes.slice(1).map(l => [l.label, l.depth, l.aside]),
    [
      ['Nationalist Spain', 1, 'Winner'],
      ['Spain', 2, 'Annexed Hour 301'],
      ['Carlist Spain', 3, 'Annexed Hour 351'],
    ],
  );
  assert.deepEqual(sideTree(threeSided.sides), [{ index: 0, depth: 1 }, { index: 1, depth: 2 }, { index: 2, depth: 3 }]);
});

test('slot and holder country filters narrow civil wars across tag renames without leaking unmatched wars', () => {
  const identity = datedIdentity({
    MOD: [{ from: 0, name: 'Modland' }],
    D03: [{ from: 300, name: 'Modland Commune' }],
    D01: [{ from: 0, name: 'Free Modland' }],
    REU: [{ from: 0, name: 'Reused' }],
    D02: [{ from: 0, name: 'Rebel' }],
  });
  const data = validateCivilWarsReply(reply(600, [
    war('a', 400, 500, [side(1, tags([300, 'D03']), 400, 500, null, true), side(3, tags([400, 'D01']), 400, 500, 0)]),
    war('b', 200, 250, [side(5, tags([0, 'REU']), 200, 250, null, true), side(4, tags([200, 'D02']), 200, 250, 0)]),
  ]));
  const bySlot = civilWarsProjection.project(
    createBoundedReadView({ hour: 600, identity, panels: new Map([['main', data]]) }),
    { ...OPTIONS, selected: [countryKey(1)] },
  );
  assert.deepEqual(bySlot.lanes.filter(l => l.expandable).map(l => l.key), ['cw:a']);
  assert.equal(bySlot.summaryText, '1 civil war · 0 ongoing');

  const byAlias = civilWarsProjection.project(
    createBoundedReadView({ hour: 600, identity, panels: new Map([['main', data]]) }),
    { ...OPTIONS, selected: ['MOD'], holders: { MOD: [{ tag: 'D03', firstHour: 300, lastHour: 600 }] } },
  );
  assert.deepEqual(byAlias.lanes.filter(l => l.expandable).map(l => l.key), ['cw:a']);
});

test('a civil wars reply holds nothing after its hour, proves up to nextChange, and rejects malformed replies', () => {
  const ok = reply(200, [war('w1', 100, 200, [side(1, tags([0, 'SPA']), 100, 200, null, true), side(2, tags([100, 'SPR']), 100, 200, 0)])], { nextChange: 250 });
  assert.equal(validateCivilWarsReply(ok), ok);
  assert.deepEqual(civilWarsProof(ok, [0, 200], 220), [200, 249]);
  assert.equal(civilWarsProof(ok, [0, 200], 250), null);
  assert.throws(() => validateCivilWarsReply(reply(150, [war('w1', 100, 200, [side(1, tags([0, 'SPA']), 100, 200)])])), /malformed/);
  assert.throws(() => validateCivilWarsReply({ ...ok, nextChange: 200 }), /malformed/);
  const loop = war('loop', 0, 10, [side(1, [], 0, 10, 1), side(2, [], 0, 10, 0)]);
  assert.throws(() => validateCivilWarsReply(reply(10, [loop])), /malformed/);
});

test('a quiet stretch of played hours makes no extra civil-wars requests and paints at most once per day', () => {
  const identity = datedIdentity({
    SPA: [{ from: 0, name: 'Nationalist Spain', colour: [216, 155, 68] }],
    SPR: [{ from: 0, name: 'Spain', colour: [143, 114, 199] }],
  });
  const quietReply = validateCivilWarsReply(
    reply(400, [war('w1', 100, 400, [side(1, tags([0, 'SPA']), 100, 400, null, true), side(2, tags([100, 'SPR']), 100, 400, 0)])], { nextChange: 900 }),
  );
  const readView = createBoundedReadView({ hour: 400, identity, panels: new Map([['main', quietReply]]) });
  assert.equal(civilWarsProjection.nextChange!(readView, OPTIONS), 900);
  const binding: FamilyTimelineBinding<CivilWarsProjectionOptions> = {
    family: 'timeline',
    kind: 'civil-wars',
    options: OPTIONS,
    sources: {},
    staticPanels: { main: quietReply },
    width: 720,
    height: 220,
    scrollTop: 0,
    theme: LANE_THEME,
    identitySnapshot: null,
    selectedKey: null,
    focusedKey: null,
    expanded: {},
    pauseFlush: 0,
  };
  const surface = { width: 720, height: 220, dpr: 1 };
  const nextWake = assertQuietFrames(() => createLaneView(timelineLane, () => {}), { label: 'civil-wars quiet', binding, hour: 400 }, 30, surface);
  assert.equal(nextWake, nextDayStart(400));

  for (let h = 400; h <= 450; h++) {
    assert.deepEqual(civilWarsProof(quietReply, [0, h], h), [400, 899]);
  }
  assert.equal(civilWarsDeclaration.kind, 'civil-wars');
});
