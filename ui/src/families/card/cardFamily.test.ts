import assert from 'node:assert/strict';
import test from 'node:test';
import type { LiveValues } from '../../charts/live';
import type { FocusCardReply, IdeaHistoryReply, WargoalsReply } from '../../generated/dashboard';
import { nationalFocusProjection } from '../../widgets/national-focus/nationalFocusProjection.ts';
import { nationalSpiritsProjection } from '../../widgets/national-spirits/nationalSpiritsProjection.ts';
import { warGoalsDeclaration, warGoalsProjection } from '../../widgets/war-goals/warGoalsProjection.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import { familyViewOptionFields } from '../../widgets/manifest.ts';
import '../../widgets/definitions.ts';
import type { FamilyPlanContext, InstalledPicture } from '../plan.ts';
import { displayedHour } from '../boundedView.ts';
import { engineHour } from '../../dashboard/calendar.ts';
import { createBoundedReadView, FutureDataError } from '../boundedView.ts';
import { assertFreshPaint, assertQuietFrames, LANE_THEME } from '../laneRuntime.fixture.ts';
import { registerFamilyProjection } from '../registry.ts';
import { cardHostAdapter } from './hostAdapter.ts';
import { familyCardLaneView } from './laneView.ts';
import { layoutCards } from './layout.ts';
import { assertCardModelValid, cardRowHours, deriveCardAudit, FutureCardDataError, type CardHeroRow, type CardItemRow, type CardModel } from './model.ts';

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
  assert.equal(undated!.art?.art.status, 'unavailable', 'art the product did not draw is unavailable with its reason, drawn as the hatched tile, not a blank');

  const unknownRate = nationalFocusProjection.project(view(hour, { ...reply, current: { ...reply.current!, daysLeft: null, progressMissing: 'Daily progress was not recorded' } }), {});
  assert.deepEqual((unknownRate.rows[0] as CardHeroRow).figure?.value, { kind: 'notRecorded', reason: 'Daily progress was not recorded' }, 'days left are not recorded, never zero');

  const gap = nationalFocusProjection.project(view(hour, { ...reply, current: null, completed: [], undated: 0, missing: { firstHour: hour, lastHour: hour + 5, reason: 'No focus row was recorded for this country' } }), {});
  assert.deepEqual([gap.empty, gap.emptyKind], ['No focus row was recorded for this country', 'unavailable']);

  assert.throws(() => nationalFocusProjection.project(view(hour - 1, reply), {}), FutureDataError, 'a reply read after the displayed hour is refused');
});

test('war goals keep the product’s order and states, show a missing expiry by name and unread progress as not recorded', () => {
  const hour = 2000 * DAY;
  const reply: WargoalsReply = {
    branchId: '1', tag: 'AAA', asked: 'AAA',
    standings: [],
    justifications: [{ id: '11', ordinal: 0, name: 'Take state', actor: 'AAA', recipient: 'BBB', states: [1], total: '1000000', daily: '10000', payUpfront: false, start: null, completion: hour + 21 * DAY, paused: false }],
    goals: [
      { handle: '1', name: 'Puppet', actor: 'AAA', recipient: 'CCC', available: false, expires: null, states: [2, 3], liberate: [], puppets: [], state: 'atWar', granted: null, grantedMissing: 'Held when the branch began' },
      { handle: '2', name: 'Conquer', actor: 'AAA', recipient: 'DDD', available: true, expires: null, states: [4], liberate: [], puppets: [], state: 'ready', granted: hour - 40 * DAY, grantedMissing: null },
      { handle: '3', name: 'Conquer', actor: 'AAA', recipient: 'EEE', available: true, expires: hour - 10 * DAY, states: [5, 6], liberate: [], puppets: [], state: 'expired', granted: hour - 300 * DAY, grantedMissing: null },
    ],
    removed: [{ handle: '9', name: 'Take state', recipient: 'FFF', seen: hour - 6 * DAY, granted: hour - 90 * DAY, grantedMissing: null, removed: hour - 5 * DAY, removedExact: false, reasonMissing: 'War goal removal reason was not recorded' }],
    warnings: [],
    stateNames: [{ id: 1, name: 'Invented state 1', known: true }, { id: 2, name: 'Invented state 2', known: true }, { id: 3, name: 'Invented state 3', known: true }, { id: 4, name: 'Invented state 4', known: true }, { id: 5, name: 'Invented state 5', known: true }],
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
  assert.equal(rows[1]!.place, 'Invented state 2 · Invented state 3');
  assert.equal(rows[2]!.figure?.value.kind, 'notRecorded', 'an unrecorded expiry is named, never blank');
  const unreadWar = items(warGoalsProjection.project(view(hour, { ...reply, goals: [{ ...reply.goals[1]!, state: 'unknown' }], justifications: [], removed: [] }, live), { targetNames: false }))[0]!;
  assert.equal(unreadWar.state, undefined, 'unread war relations claim no state');
  assert.equal(unreadWar.figure?.value.kind, 'notRecorded');
  assert.equal(rows[3]!.muted, true);
  assert.equal(rows[3]!.place, 'Invented state 5 · State not recorded');
  const unread = warGoalsProjection.project(view(hour, reply, null), { targetNames: false });
  assert.equal(items(unread)[0]!.meter?.fraction.kind, 'notRecorded', 'progress not read is not zero');
  const justification = reply.justifications[0]!;
  const withPause = (paused: boolean | null) => items(warGoalsProjection.project(view(hour, { ...reply, justifications: [{ ...justification, paused }] }, live), { targetNames: false }))[0]!;
  assert.equal(withPause(true).state?.label, 'Paused', 'a paused justification is not Justifying');
  assert.equal(withPause(true).figure?.value.kind, 'notRecorded', 'and has no countdown');
  assert.equal(withPause(null).state, undefined, 'an unread pause state claims no state');
  assert.equal(withPause(null).figure?.value.kind, 'notRecorded');
  const named = warGoalsProjection.project(view(hour, reply, live), { targetNames: true });
  assert.equal(items(named)[0]!.subtitle, 'Take state');
  assert.ok(deriveCardAudit(model).some(row => row.key === `${row.row}:state` && String(row.value).endsWith(':Expired')));
});

test('national spirits list the product’s membership at the displayed hour, in its order, with since dates and unclassified items kept', () => {
  const first = 100 * DAY;
  const looks = ['mod_spirit_a', 'mod_spirit_b', 'mod_spirit_c', 'mod_law'].map((token, at) => ({
    info: { token, category: at === 3 ? 'laws' : at === 2 ? null : 'spirits', slot: null, law: at === 3, designer: false, effects: [], targeted: [], equipment: [], research: [], traits: [], tooltips: [], effectsKnown: true, unknown: at === 2 ? 'Invented reason the idea has no category' : null, nameMissing: null },
    artwork: { token, name: `Invented ${token}`, description: null, baseDurationDays: null, icon: at === 0 ? 'asset://spirit-a.png' : null },
  }));
  const categories = [{ token: 'spirits', name: 'Spirits', kind: 'national_spirit', hidden: false, politicsTab: false, slots: [] }, { token: 'laws', name: 'Laws', kind: null, hidden: false, politicsTab: true, slots: [] }];
  const held = (look: number, firstHour: number, hour: number, start: string) => ({ token: looks[look]!.info.token, firstHour, lastHour: hour, start, end: { kind: 'held' }, slot: null, character: null, look });
  const replyAt = (hour: number, holdings: ReturnType<typeof held>[], nextChange: number | null, unrecorded: string | null = null) => ({
    branchId: '1', tag: 'AAA', asked: 'AAA', hour, nextChange, unrecorded, holdings, held: holdings.map((_, at) => at).reverse(), looks, categories,
    unobserved: [], modifiers: [], texts: [], warnings: [],
  } as unknown as IdeaHistoryReply);
  const hour = first + 20 * DAY;
  const reply = replyAt(hour, [held(0, first, hour, 'heldBefore'), held(1, first + 10 * DAY, hour, 'seen'), held(3, first, hour, 'seen')], first + 40 * DAY);
  const model = nationalSpiritsProjection.project(view(hour, reply), {});
  const rows = items(model);
  assert.deepEqual(rows.map(row => row.key), ['mod_spirit_b', 'mod_spirit_a'], 'the product decides the order; a law is not a spirit');
  assert.match(String(rows[1]!.figure?.value.kind === 'known' && rows[1]!.figure.value.value), /^Held before /);
  assert.match(String(rows[0]!.figure?.value.kind === 'known' && rows[0]!.figure.value.value), /^Since /);
  assert.equal(rows[0]!.art?.art.status, 'unavailable');
  assert.equal(model.nextChangeAt, first + 40 * DAY);
  const gained = first + 45 * DAY;
  const later = replyAt(gained, [held(0, first, gained, 'heldBefore'), held(1, first + 10 * DAY, gained, 'seen'), held(2, first + 41 * DAY, gained, 'firstObserved')], null);
  const unclassified = items(nationalSpiritsProjection.project(view(gained, later), {})).find(row => row.key === 'mod_spirit_c')!;
  assert.match(String(unclassified.figure?.value.kind === 'known' && unclassified.figure.value.value), /^First observed /, 'first seen after a gap is a bound, not a start');
  assert.ok(unclassified.tooltip?.items?.some(item => item.label === 'Category' && 'missingReason' in item), 'an unclassified idea is kept with its reason');
  const unseen = nationalSpiritsProjection.project(view(first + 60 * DAY, replyAt(first + 60 * DAY, [], null, 'Invented unread reason')), {});
  assert.equal(unseen.emptyKind, 'unavailable');

  registerFamilyProjection(nationalSpiritsProjection);
  const options = {};
  const lane = (panel: unknown) => ({ family: 'card', kind: 'national-spirits', options, sources: {}, staticPanels: { main: panel }, scrollTop: 0, theme: LANE_THEME, width: 420, height: 160 });
  const renamed = { ...reply, looks: reply.looks.map(entry => (entry.artwork.token === 'mod_spirit_b' ? { ...entry, artwork: { ...entry.artwork, name: 'Invented Spirit Renamed' } } : entry)) } as IdeaHistoryReply;
  const surface = { width: 420, height: 160, dpr: 1 };
  assert.equal(assertQuietFrames(() => familyCardLaneView(() => {}), { label: 'spirits', binding: lane(reply), hour }, 24, surface), first + 40 * DAY);
  assertFreshPaint(() => familyCardLaneView(() => {}), [
    { label: 'held', binding: lane(reply), hour, check: true },
    { label: 'a spirit gained', binding: lane(later), hour: gained, speed: 24 },
    { label: 'seek back', binding: lane(reply), hour, check: true },
    { label: 'same-hour correction', binding: lane(renamed), hour, check: true },
  ], surface);
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

test('special projects follow the product’s order: running with progress and scientist, completed newest first, undated last', async () => {
  const { specialProjectsProjection } = await import('../../widgets/special-projects/specialProjectsProjection.ts');
  const hour = engineHour({ year: 1941, month: 9, day: 15, hour: 1 })!;
  const first = hour - 400 * DAY;
  const info = (name: string, scientist: number) => ({ name, specialization: 'naval', specializationName: 'Naval', icon: null, iconMissing: 'No sprite', scientist, lastScientist: null });
  const status = (presentation: number, state: string, progress: number) => ({ firstHour: first, lastHour: hour + 10, readHour: first, presentation, progress, state, iterations: null, scientistRead: null, lastScientistRead: null });
  const project = (id: string, firstState: string, statuses: unknown[], entries: unknown[] = []) => ({ id, owner: 1, local: 0, firstRead: first, firstState, statuses, entries, attempts: [] });
  const reply = {
    branchId: '1', tag: 'AAA', identities: [{ firstHour: first, lastHour: hour + 10, slot: 7, countryId: 9 }], unresolved: [], available: [{ firstHour: first, lastHour: hour + 10 }], partial: [],
    projects: [
      project('support', 'running', [status(0, 'running', 24)]),
      project('radar', 'running', [status(1, 'completed', 100)], [{ ordinal: 0, date: hour - 196 * DAY, completion: true, readHour: hour - 196 * DAY, counted: true, presentation: null }]),
      project('old', 'completed', [status(2, 'completed', 100)]),
      project('unread', 'running', [status(3, 'unknown', 40)]),
    ],
    reads: [], presentations: [info('Invented project A', 1), info('Invented project B', 0), info('Invented project C', 0), info('Invented project', 0)],
    people: [{ assigned: false, id: null, name: null, portrait: null, missing: null }, { assigned: null, id: null, name: null, portrait: null, missing: 'The program read was not recorded' }],
    iterationTexts: {}, unavailable: null, warnings: [],
    shown: { through: hour, running: [0], unknown: [3], completed: [{ project: 1, date: hour - 196 * DAY }, { project: 2, date: null }], undated: 1 },
  };
  const model = specialProjectsProjection.project(view(hour, reply), {});
  assertCardModelValid(model, hour);
  assert.deepEqual(model.rows.map(row => row.kind === 'section' ? `${row.label} ${row.count}` : row.key), ['In progress 1', 'running:support', 'State not recorded 1', 'running:unread', 'Completed 2', 'done:radar', 'Date not recorded 1', 'done:old']);
  const [running, , radar, old] = items(model);
  assert.equal(running!.meter?.text, '24%');
  assert.equal(running!.figure?.value.kind, 'notRecorded', 'an unread scientist is named as not recorded');
  assert.deepEqual(radar!.figure?.value, { kind: 'known', value: 'No scientist' });
  assert.deepEqual(old!.date, { hour: null, reason: 'Already complete when the project was first recorded' });
  assert.equal(radar!.art?.art.status, 'unavailable');
});

test('a stored "Show names" reaches the war goals projection through the declaration the host resolves', () => {
  const resolve = (stored: Record<string, boolean>) => warGoalsDeclaration.projectionOptions({
    options: resolveOptions(familyViewOptionFields('war-goals'), stored),
  } as FamilyPlanContext<unknown>);
  assert.equal(resolve({ targetNames: true }).targetNames, true);
  assert.equal(resolve({}).targetNames, false);
});

test('a dated card row acts at its own date; an undated one at the installed hour', () => {
  const model: CardModel = { hour: 500, columns: { date: true }, rowHeight: 36, rows: [
    { kind: 'item', key: 'done', title: { kind: 'known', value: 'Invented focus' }, date: { hour: 120 } },
    { kind: 'item', key: 'undated', title: { kind: 'known', value: 'Other focus' }, date: { hour: null, reason: 'Completion not recorded' } },
  ] };
  const installed = { id: 1, owner: 'o', hour: displayedHour(500), actionKeys: new Map() } as unknown as InstalledPicture;
  const fields = { rowHours: cardRowHours(model) };
  assert.equal(cardHostAdapter.resolveHitAction!({ key: 'done' }, installed, fields)?.hour, 120);
  assert.equal(cardHostAdapter.resolveHitAction!({ key: 'undated' }, installed, fields)?.hour, 500);
});

test('the card audit lists what is drawn: printed meter text, subtitle, hero detail and caption, state mark', () => {
  const model: CardModel = { hour: 500, columns: {}, rowHeight: 36, rows: [
    { kind: 'hero', key: 'hero', title: { kind: 'known', value: 'Invented focus' }, state: { mark: 'running', label: 'In progress' }, detail: 'Day 12 of 70',
      figure: { value: { kind: 'known', value: '58 days' }, caption: 'Due 3 Mar 1937' }, meter: { fraction: { kind: 'known', value: 0 }, text: '-20%' } },
    { kind: 'item', key: 'item', title: { kind: 'known', value: 'Invented project' }, subtitle: 'Invented science' },
  ] };
  const audit = new Map(deriveCardAudit(model).map(row => [row.key.replace(`${row.row}:`, `${row.row}/`), row.value]));
  assert.equal(audit.get('hero/meterText'), '-20%');
  assert.equal(audit.get('hero/detail'), 'Day 12 of 70');
  assert.equal(audit.get('hero/caption'), 'Due 3 Mar 1937');
  assert.equal(audit.get('hero/state'), 'running:In progress');
  assert.equal(audit.get('item/subtitle'), 'Invented science');
});
