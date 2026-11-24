import assert from 'node:assert/strict';
import test from 'node:test';
import type { ResearchHistory } from '../research/types';
import { HOUR_EPOCH } from '../../dashboard/calendar.ts';
import { estimateHour, inspectedSlot, slotInspection, slotValues, slotNumbers, slotsStructure } from '../research/researchSlots.ts';
import { activeSlots, openedResearch } from './activeResearchData.ts';
import { panelViews, readingStructure } from '../../draw/panels/views.ts';
import type { ResearchEntry, ResearchSlotNow, ResearchSlotState } from '../../generated/dashboard';
import type { ResearchSlotsAt } from '../research/types';

const base = HOUR_EPOCH + 1944 * 8760 + 12;
const day = (n: number) => base + n * 24;
const state = (token: string | null, entry: number | null, extra: Partial<ResearchSlotState> = {}): ResearchSlotState => ({
  firstHour: day(96), lastHour: day(100), token, databaseIndex: token === null ? null : token === 'radar' ? 4 : 9, entryFirstObservedHour: entry, speedMultiplierRaw: '100000', bankedDaysRaw: '0',
  appliedDaysRaw: '0', aheadReductionYearsRaw: token ? '0' : null, details: null, ...extra,
});
const active: ResearchEntry = { token: 'radar', technologyHandle: 1, databaseIndex: 4, continuityStartHour: day(0), level: 0, firstObservedHour: day(0),
  startedAfterHour: null, lastObservedHour: day(100), endedByHour: null, outcomeObservedHour: null, outcome: 'active' };
const modifiers = (year: number) => ({ year, totalBonusRaw: '0', aheadFactorRaw: '200000', minimumSpeedRaw: '10000', components: [] });
const open = [{ firstHour: day(0), lastHour: day(100) }];
const reply = (hour: number, slots: ResearchSlotNow[], missing: string | null = null): ResearchSlotsAt => ({
  tag: 'MOD', hour, readHour: missing ? null : day(96), missing, slots, warnings: [], grants: { nextId: 0, grants: [], unavailable: null } });

test('each open slot says what it researches, and its painted numbers count down from the read', () => {
  const history: ResearchHistory = { coverage: open, slots: [
    { index: 0, availability: open, entries: [active], states: [], progress: [] },
    { index: 1, availability: open, entries: [], states: [], progress: [] },
    { index: 2, availability: open, entries: [], states: [], progress: [] },
    { index: 3, availability: [{ firstHour: day(0), lastHour: day(50) }], entries: [], states: [], progress: [] },
  ] };
  const radar: ResearchSlotNow = { index: 0, state: state('radar', day(0), { details: { remainingDays: 40, estimateRaw: '3950000', bonuses: null, modifiers: modifiers(1945) } }),
    progress: { firstHour: day(96), lastHour: day(96), entryFirstObservedHour: day(0), baseDaysRaw: '10000000', progressRaw: '6200000' }, definition: null };
  const sonar: ResearchSlotNow = { index: 1, state: state('sonar', null, { details: { remainingDays: 12, estimateRaw: '1200000', bonuses: null, modifiers: modifiers(1940) } }), progress: null,
    definition: { token: 'sonar', databaseIndex: 9, name: 'Sonar', description: null } };
  const idle: ResearchSlotNow = { index: 2, state: state(null, null, { bankedDaysRaw: '1200000' }), progress: null, definition: null };
  const now = reply(day(100), [radar, sonar, idle]);
  const slots = activeSlots(history, now);
  assert.deepEqual(slots.map(slot => slot.kind), ['researching', 'researching', 'idle'], 'a slot closed by then is not shown');
  const [first, second] = slots;
  assert.ok(first.kind === 'researching' && second.kind === 'researching');
  assert.equal(first.entry?.token, 'radar');
  assert.equal(second.entry, null, 'a technology no assignment joins is shown without a history to open');
  assert.deepEqual(slotNumbers(radar, day(100)), { fraction: 0.62, percent: '62%', left: 36, days: '36 days left', saved: '' }, 'the estimate counts down from its read');
  assert.equal(slotNumbers(idle, day(100)).saved, '12 days saved');
  assert.equal(slotNumbers(sonar, day(100)).fraction, null, 'no progress is guessed for it');
  const later = reply(day(101), [{ ...radar, state: { ...radar.state, firstHour: day(101) }, progress: { ...radar.progress!, progressRaw: '6400000' } }, sonar, idle]);
  assert.equal(slotsStructure(later), slotsStructure(now));
  assert.notEqual(slotsStructure(reply(day(101), [radar, sonar])), slotsStructure(now));
  const nextSonar = { ...sonar, state: { ...sonar.state, firstHour: day(101), lastHour: day(101), details: { ...sonar.state.details!, remainingDays: 11, estimateRaw: '1100000' } } };
  const plain = (read: ResearchSlotNow) => ({ ...read, state: { ...read.state, details: read.state.details && { ...read.state.details, modifiers: null } } });
  const opened = reply(day(100), [sonar].map(plain)), next = { ...reply(day(101), [nextSonar].map(plain)), readHour: day(101) };
  const structureOf = readingStructure('research-slots')!;
  const shown = structureOf({ data: opened } as never, day(100))!;
  assert.notEqual(structureOf({ data: next } as never, day(101)), shown, 'the follow structure carries the read that supplied it');
  const view = panelViews['research-slots']!;
  assert.equal(view.values(next, null, day(101), shown), null, 'a later read is never painted into the one shown');
  assert.equal(view.values(opened, null, day(100), shown)?.get('1:days'), '8 days left');
});

test('the drawn card uses the dated slot identity, progress and countdown', () => {
  const read: ResearchSlotNow = { index: 0, state: state('radar', day(0), { details: { remainingDays: 3, estimateRaw: '300000', bonuses: null, modifiers: null } }),
    progress: { firstHour: day(96), lastHour: day(96), entryFirstObservedHour: day(0), baseDaysRaw: '100000', progressRaw: '50000' }, definition: null };
  const values = slotValues(reply(day(97), [read]), day(97));
  assert.equal(values.get('0:percent'), '50%');
  assert.equal(values.get('0:days'), '2 days left');
  assert.equal(values.get('0:bar'), '0.5');
  const later = slotValues(reply(day(97), [read]), day(98), () => .625);
  assert.equal(later.get('0:percent'), '63%');
  assert.equal(later.get('0:days'), '1 day left');
  assert.equal(later.get('0:bar'), '0.625');
});

test('review 7 B: a research tooltip reads the progress and countdown the installed card painted, not the retained reply', () => {
  const read: ResearchSlotNow = { index: 0, definition: null, state: state('radar', day(0), { details: { remainingDays: 3, estimateRaw: '300000', bonuses: null, modifiers: null } }),
    progress: { firstHour: day(96), lastHour: day(96), entryFirstObservedHour: day(0), baseDaysRaw: '100000', progressRaw: '50000' } };
  const painted = slotValues(reply(day(97), [read]), day(98), () => .625);
  assert.deepEqual(slotInspection(painted, 0, day(98)), { fraction: 0.625, left: 1, hour: day(98) }, 'the tooltip shows what the card shows at its painted hour');
  assert.deepEqual(slotInspection(slotValues(reply(day(97), [{ ...read, progress: null, state: { ...read.state, details: null } }]), day(98)), 0, day(98)),
    { fraction: null, left: null, hour: day(98) }, 'a value the card shows as not recorded stays not recorded');
  assert.equal(slotInspection(undefined, 0, day(98)), undefined, 'with nothing installed the tooltip falls back to the asked reply');
});

test('a slot without a usable progress or estimate says so, an idle slot says what it saved, and a reading becoming usable redraws the row', () => {
  const unknown: ResearchSlotNow = { index: 0, state: state('radar', day(0)),
    progress: { firstHour: day(96), lastHour: day(96), entryFirstObservedHour: day(0), baseDaysRaw: '10000000', progressRaw: '-1' }, definition: null };
  const numbers = slotNumbers(unknown, day(100));
  assert.deepEqual([numbers.fraction, numbers.percent, numbers.days], [null, 'Not recorded', 'Not recorded']);
  assert.equal(slotNumbers({ index: 1, state: state(null, null), progress: null, definition: null }, day(100)).saved, '0 days saved');
  const usable = { ...unknown, progress: { ...unknown.progress!, progressRaw: '5000000' } };
  assert.notEqual(slotsStructure(reply(day(100), [unknown])), slotsStructure(reply(day(100), [usable])));
});

test('at an hour without a read, slots open at the last read are unavailable rather than guessed', () => {
  const history: ResearchHistory = { coverage: [{ firstHour: day(0), lastHour: day(50) }, { firstHour: day(80), lastHour: day(100) }], slots: [
    { index: 0, availability: [{ firstHour: day(0), lastHour: day(50) }, { firstHour: day(80), lastHour: day(100) }], entries: [], states: [], progress: [] },
    { index: 1, availability: [{ firstHour: day(0), lastHour: day(20) }], entries: [], states: [], progress: [] },
  ] };
  const slots = activeSlots(history, reply(day(60), [], 'No research read covers this hour'));
  assert.deepEqual(slots.map(slot => [slot.kind, slot.slot.index]), [['unavailable', 0]]);
  const covered = activeSlots(history, reply(day(90), [], 'This country\'s research slots were not read together'));
  assert.deepEqual(covered.map(slot => slot.kind === 'unavailable' && slot.reason), ['This country\'s research slots were not read together']);
  assert.equal(covered[0]!.slot, history.slots[0], 'the slot is the recorded one, not a copy made for the hour');
  assert.equal(activeSlots(history, reply(day(91), [], 'Not read'))[0]!.slot, history.slots[0], 'a later reading copies no history');
});
test('a research without a joined assignment keeps the definition it was read with, and a later read is never taken for it', () => {
  const first: ResearchEntry = { ...active, token: 'mod_tech', databaseIndex: 5 };
  const history: ResearchHistory = { coverage: open, slots: [{ index: 0, availability: open, entries: [first], states: [], progress: [] }] };
  const read = (at: number): ResearchSlotNow => ({ index: 0, progress: null, definition: { token: 'mod_tech', databaseIndex: 6, name: 'Mod technology', description: null },
    state: state('mod_tech', null, { firstHour: at, lastHour: at, databaseIndex: 6, details: { remainingDays: 9, estimateRaw: '900000', bonuses: null, modifiers: null } }) });
  const researching = (at: number) => {
    const [slot] = activeSlots(history, reply(at, [read(at)]));
    assert.ok(slot.kind === 'researching');
    return slot;
  };
  const opened = openedResearch(researching(day(96)));
  assert.deepEqual([opened.databaseIndex, opened.firstObservedHour, opened.readHour], [6, null, day(96)], 'the definition the slot read is kept');
});

test('U-S7: an open breakdown reads at the installed estimate, and moves only when the shown days left or its read change', () => {
  const read: ResearchSlotNow = { index: 0, definition: null, state: state('radar', null, { firstHour: day(96), lastHour: day(99), details: { remainingDays: 9, estimateRaw: '900000', bonuses: null, modifiers: null } }),
    progress: null };
  const asked = reply(day(96), [read]);
  const installed = (hour: number, painted = asked) => ({ hour, data: asked, texts: slotValues(painted, hour) });
  assert.equal(estimateHour(installed(day(97) + 5), asked, 0, 'radar'), day(97), 'the installed hour, not the asked reply, sets the day the breakdown reads');
  const reread = reply(day(97), [{ ...read, state: { ...read.state, firstHour: day(97) + 3 } }]);
  assert.equal(estimateHour(installed(day(97) + 5, reread), asked, 0, 'radar'), day(97) + 3,
    'F-S4: a new read painted with an unchanged structure moves the breakdown to the read the painted days left come from');
  assert.equal(estimateHour(installed(day(97) + 5), asked, 0, 'radar'), estimateHour(installed(day(97) + 20), asked, 0, 'radar'), 'within one shown days left the breakdown stays put');
  assert.equal(estimateHour(undefined, asked, 0, 'radar'), day(96), 'with nothing installed it reads at the asked hour');
  assert.equal(estimateHour(installed(day(97) + 5), asked, 0, 'sonar'), day(97) + 5, 'another research in the slot is not this estimate');
});

test('review 10: an open tooltip reads its technology and its values from the same installation, and a vanished slot has no subject', () => {
  const texts = slotValues(reply(day(97), [{ index: 0, definition: null, state: state('radar', day(0), { details: { remainingDays: 3, estimateRaw: '300000', bonuses: null, modifiers: null } }), progress: null }]), day(98));
  const installed = (token: string | null) => ({ texts, hour: day(98), data: { slots: [{ index: 0, state: { token } }] } });
  assert.equal(inspectedSlot(installed('sonar'), 0)?.token, 'sonar', 'the successor installed with these values is the subject, so Radar\'s tooltip withdraws');
  assert.equal(inspectedSlot({ ...installed('radar'), data: { slots: [] } }, 0)?.token, undefined, 'a slot gone from the installation has no subject');
});

test('review 11: a slot gone from the installation is an explicit absent result, so the open tooltip closes', () => {
  const texts = slotValues(reply(day(97), []), day(98));
  const absent = inspectedSlot({ texts, hour: day(98), data: { slots: [] } }, 0);
  assert.deepEqual(absent, { fraction: null, left: null, hour: day(98), token: undefined });
});
