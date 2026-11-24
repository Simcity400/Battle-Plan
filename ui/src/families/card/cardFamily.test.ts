import assert from 'node:assert/strict';
import test from 'node:test';
import type { LiveValues } from '../../charts/live';
import type { FocusCardReply, IdeaHistoryReply, WargoalsReply } from '../../generated/dashboard';
import { nationalFocusProjection } from '../../widgets/national-focus/nationalFocusProjection.ts';
import { nationalSpiritsProjection } from '../../widgets/national-spirits/nationalSpiritsProjection.ts';
import { warGoalsProjection } from '../../widgets/war-goals/warGoalsProjection.ts';
import { engineHour } from '../../dashboard/calendar.ts';
import { createBoundedReadView, FutureDataError } from '../boundedView.ts';
import { layoutCards } from './layout.ts';
import { assertCardModelValid, deriveCardAudit, FutureCardDataError, type CardHeroRow, type CardItemRow, type CardModel } from './model.ts';

const DAY = 24;
const art = (token: string, name: string | null, icon: string | null = null) => ({ token, name, nameMissing: name ? null : 'Name refused', icon, iconMissing: icon ? null : 'No sprite in the installed mods' });

function view(hour: number, panel: unknown, live?: LiveValues<number | bigint | string> | null) {
  return createBoundedReadView({ hour, panels: new Map([['main', panel]]), ...(live !== undefined ? { live: new Map([['main', live]]) } : {}) });
}

const items = (model: CardModel) => model.rows.filter((row): row is CardItemRow => row.kind === 'item');

test('a focus card shows the product’s dated completions, then its undated ones under their own heading, never a guessed date', () => {
  const hour = engineHour({ year: 1941, month: 9, day: 15, hour: 1 })!;
  const reply: FocusCardReply = {
    hour,
    validThrough: hour + 23,
    missing: null,
    current: { art: art('armament', 'Armament Rationalization', 'asset://focus.png'), continuous: false, paused: false, totalDays: 70, elapsedDays: 31, permyriad: 4571, daysLeft: 38, progressMissing: null },
    completed: [
      { art: art('fall_gelb', 'Fall Gelb'), completedByHour: hour - 30 * DAY, dateMissing: null },
      { art: art('four_year_plan', null), completedByHour: null, dateMissing: 'Already complete when first read' },
    ],
    undated: 1,
    warnings: [],
  };
  const model = nationalFocusProjection.project(view(hour, reply), {});
  assertCardModelValid(model, hour);
  const hero = model.rows[0] as CardHeroRow;
  assert.equal(hero.kind, 'hero');
  assert.deepEqual(hero.figure?.value, { kind: 'known', value: '38' });
  assert.match(hero.detail ?? '', /^Day 32 of 70 · due \d{1,2} \w{3} \d{4}$/, 'a date with no hour');
  assert.deepEqual(model.rows.map(row => row.kind), ['hero', 'item', 'section', 'item']);
  const section = model.rows[2]!;
  assert.equal(section.kind === 'section' && `${section.label} ${section.count}`, 'Date not recorded 1');
  const [dated, undated] = items(model);
  assert.equal(dated!.date?.hour, hour - 30 * DAY);
  assert.deepEqual(undated!.date, { hour: null, reason: 'Already complete when first read' });
  assert.deepEqual(undated!.title, { kind: 'notRecorded', reason: 'Name refused' });
  assert.equal(undated!.art?.art.status, 'confirmedMissing', 'missing art is the hatched tile, not a blank');

  const unknownRate = nationalFocusProjection.project(view(hour, { ...reply, current: { ...reply.current!, daysLeft: null, progressMissing: 'Daily progress was not recorded' } }), {});
  assert.deepEqual((unknownRate.rows[0] as CardHeroRow).figure?.value, { kind: 'notRecorded', reason: 'Daily progress was not recorded' }, 'days left are not recorded, never zero');

  const gap = nationalFocusProjection.project(view(hour, { ...reply, current: null, completed: [], undated: 0, missing: { firstHour: hour, lastHour: hour + 5, reason: 'No focus row was recorded for this country' } }), {});
  assert.deepEqual([gap.empty, gap.emptyKind], ['No focus row was recorded for this country', 'unavailable']);

  assert.throws(() => nationalFocusProjection.project(view(hour - 1, reply), {}), FutureDataError, 'a reply read after the displayed hour is refused');
});

test('war goals keep the product’s order and states, show a missing expiry by name and unread progress as not recorded', () => {
  const hour = 2000 * DAY;
  const reply: WargoalsReply = {
    branchId: '1', tag: 'GER', asked: 'GER',
    standings: [],
    justifications: [{ id: '11', ordinal: 0, name: 'Take state', actor: 'GER', recipient: 'SWE', states: [1], total: '1000000', daily: '10000', payUpfront: false, start: null, completion: hour + 21 * DAY, paused: false }],
    goals: [
      { handle: '1', name: 'Puppet', actor: 'GER', recipient: 'SOV', available: false, expires: null, states: [2, 3], liberate: [], puppets: [], state: 'atWar', granted: null, grantedMissing: 'Held when the branch began' },
      { handle: '2', name: 'Conquer', actor: 'GER', recipient: 'DEN', available: true, expires: null, states: [4], liberate: [], puppets: [], state: 'ready', granted: hour - 40 * DAY, grantedMissing: null },
      { handle: '3', name: 'Conquer', actor: 'GER', recipient: 'POL', available: true, expires: hour - 10 * DAY, states: [5, 6], liberate: [], puppets: [], state: 'expired', granted: hour - 300 * DAY, grantedMissing: null },
    ],
    removed: [{ handle: '9', name: 'Take state', recipient: 'NOR', granted: hour - 90 * DAY, grantedMissing: null, removed: hour - 5 * DAY, removedExact: false, reasonMissing: 'War goal removal reason was not recorded' }],
    warnings: [],
    stateNames: [{ id: 1, name: 'Norrbotten', known: true }, { id: 2, name: 'Moskva', known: true }, { id: 3, name: 'Leningrad', known: true }, { id: 4, name: 'Sjaelland', known: true }, { id: 5, name: 'Danzig', known: true }],
  };
  const live: LiveValues<number | bigint | string> = { get: key => (key.startsWith('11') ? 640_000 : undefined), missing: () => 'Not read at this hour' };
  const model = warGoalsProjection.project(view(hour, reply, live), { targetNames: false });
  assertCardModelValid(model, hour);
  const rows = items(model);
  assert.deepEqual(rows.map(row => row.state?.label), ['Justifying', 'At war', 'Ready', 'Expired', 'Removed']);
  assert.match(String(rows[4]!.figure?.value.kind === 'known' && rows[4]!.figure.value.value), /^By /, 'a removal seen only after a gap is a bound, not an exact date');
  assert.ok(rows[4]!.tooltip?.items?.some(item => item.label === 'Reason' && item.missingReason === 'War goal removal reason was not recorded'));
  assert.deepEqual(rows[0]!.meter?.fraction, { kind: 'known', value: 0.64 });
  assert.deepEqual(rows[0]!.figure?.value, { kind: 'known', value: '21 days left' });
  assert.equal(rows[1]!.place, 'Moskva · Leningrad');
  assert.equal(rows[2]!.figure?.value.kind, 'notRecorded', 'an unrecorded expiry is named, never blank');
  assert.equal(rows[3]!.muted, true);
  assert.equal(rows[3]!.place, 'Danzig · State not recorded');
  const unread = warGoalsProjection.project(view(hour, reply, null), { targetNames: false });
  assert.equal(items(unread)[0]!.meter?.fraction.kind, 'notRecorded', 'progress not read is not zero');
  const named = warGoalsProjection.project(view(hour, reply, live), { targetNames: true });
  assert.equal(items(named)[0]!.subtitle, 'Take state');
  assert.ok(deriveCardAudit(model).some(row => row.key === 'state' && row.value === 'Expired'));
});

test('national spirits list only what is held at the displayed hour, with since dates from the recording', () => {
  const first = 100 * DAY;
  const reply = {
    branchId: '1', tag: 'USA', asked: 'USA',
    spans: [
      { token: 'war_dept', firstHour: first, lastHour: first + 500 * DAY, slot: null, character: null },
      { token: 'home_free', firstHour: first + 10 * DAY, lastHour: first + 500 * DAY, slot: null, character: null },
      { token: 'later', firstHour: first + 40 * DAY, lastHour: first + 500 * DAY, slot: null, character: null },
    ],
    ideas: ['war_dept', 'home_free', 'later'].map(token => ({ token, category: 'spirits', slot: null, law: false, designer: false, effects: [], targeted: [], equipment: [], research: [], traits: [], tooltips: [], effectsKnown: true, unknown: null, nameMissing: null })),
    artwork: [
      { token: 'war_dept', name: 'War Department', description: null, baseDurationDays: null, icon: 'asset://war.png' },
      { token: 'home_free', name: 'Home of the Free', description: null, baseDurationDays: null, icon: null },
      { token: 'later', name: 'Later Spirit', description: null, baseDurationDays: null, icon: null },
    ],
    holders: [], categories: [{ token: 'spirits', name: 'Spirits', kind: 'national_spirit', hidden: false, politicsTab: false, slots: [] }],
    unobserved: [{ firstHour: first + 60 * DAY, lastHour: first + 61 * DAY }], identityStarts: [], modifiers: [], texts: [], warnings: [],
  } as unknown as IdeaHistoryReply;
  const hour = first + 20 * DAY;
  const model = nationalSpiritsProjection.project(view(hour, reply), { branchFirst: first });
  const rows = items(model);
  assert.deepEqual(rows.map(row => row.key), ['war_dept', 'home_free'], 'a spirit gained after the displayed hour is not shown');
  assert.match(String(rows[0]!.figure?.value.kind === 'known' && rows[0]!.figure.value.value), /^Held before /);
  assert.match(String(rows[1]!.figure?.value.kind === 'known' && rows[1]!.figure.value.value), /^Since /);
  assert.equal(rows[1]!.art?.art.status, 'confirmedMissing');
  assert.equal(model.nextChangeAt, first + 40 * DAY);
  const unseen = nationalSpiritsProjection.project(view(first + 60 * DAY, reply), { branchFirst: first });
  assert.equal(unseen.emptyKind, 'unavailable');
});

test('a card dated after the displayed hour is refused, and narrow cards fold the place under the title', () => {
  const model: CardModel = {
    hour: 10,
    rows: [{ kind: 'item', key: 'a', title: { kind: 'known', value: 'A' }, date: { hour: 11 }, place: 'Somewhere' }],
    columns: { date: true, flag: true, place: true, figure: 170, state: 120 },
    rowHeight: 36,
  };
  assert.throws(() => assertCardModelValid(model, 10), FutureCardDataError);
  const wide = layoutCards(model, 900, 300);
  const narrow = layoutCards(model, 400, 300);
  assert.ok(wide.placeWidth > 0 && wide.figureWidth === 170);
  assert.equal(narrow.compact, true);
  assert.equal(narrow.placeWidth, 0);
  assert.equal(narrow.figureWidth, 0);
  assert.ok(narrow.stateWidth > 0, 'the state keeps its word at the minimum size');
});
