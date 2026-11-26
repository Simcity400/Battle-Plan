import assert from 'node:assert/strict';
import test from 'node:test';
import type { ResearchEntry, ResearchReply, ResearchSlotState } from '../../generated/dashboard';
import type { ResearchSlotsAt } from './types';
import { researchArt, researchArtwork, researchAt, researchEntryAt,
  researchRateAt, researchReadName, researchRecordedTemplates, validateResearchBreakdown, validateResearchReply, validateResearchSlots,
  withBreakdown } from './researchData.ts';

const entry = (token: string, first: number, end: number | null, outcome: ResearchEntry['outcome'] = end === null ? 'active' : 'completed', databaseIndex = 3): ResearchEntry => ({
  token, technologyHandle: 1, databaseIndex, continuityStartHour: 0, level: 0, firstObservedHour: first, startedAfterHour: first > 0 ? first - 1 : null,
  lastObservedHour: end === null ? first : end - 1, endedByHour: end, outcomeObservedHour: null, outcome,
});
const state = (token: string | null, first: number, entryFirst: number | null, extra: Partial<ResearchSlotState> = {}): ResearchSlotState => ({
  firstHour: first, lastHour: first + 23, token, databaseIndex: token === null ? null : 3, entryFirstObservedHour: entryFirst, speedMultiplierRaw: '100000', bankedDaysRaw: '0', appliedDaysRaw: '0',
  aheadReductionYearsRaw: token ? '0' : null, details: null, ...extra,
});
const reply = (entries: ResearchEntry[], extra: Partial<ResearchReply> = {}): ResearchReply => ({
  history: { coverage: [{ firstHour: 0, lastHour: 100 }], slots: [{ index: 0, availability: [{ firstHour: 0, lastHour: 100 }], entries }] },
  artwork: [], folders: [], technologyFolders: [], warnings: [], ...extra,
});

test('a history reply is checked and its slots made ready to join reads to', () => {
  const data = validateResearchReply(reply([entry('tech_alpha', 0, 10), entry('tech_beta', 10, null)]));
  assert.deepEqual(data.history.slots[0].states, []);
  assert.deepEqual(data.history.slots[0].progress, []);
  assert.equal(validateResearchReply(reply(Array.from({ length: 5000 }, (_, at) => entry(`t${at}`, at * 2, at * 2 + 1)))).history.slots[0].entries.length, 5000);
  for (const bad of [
    reply([entry('tech_alpha', 0, null), entry('tech_beta', 10, 20)]),
    reply([{ ...entry('tech_alpha', 5, 10), databaseIndex: 0 }]),
    reply([{ ...entry('tech_alpha', 5, 10), outcomeObservedHour: 7 }]),
    reply([entry('tech_alpha', 0, 10)], { folders: [{ key: '', name: null }] }),
    { ...reply([]), history: { coverage: [{ firstHour: 10, lastHour: 5 }], slots: [] } },
  ]) assert.throws(() => validateResearchReply(bad), /does not match/);
});

test('rewind hides new slots, future assignments and outcomes not yet observed', () => {
  const data = validateResearchReply(reply([{ ...entry('tech_alpha', 0, 10), outcomeObservedHour: 30 }, entry('tech_beta', 40, null)]));
  assert.equal(researchEntryAt(data.history.slots[0], state('tech_alpha', 0, 0), 5)?.outcomeObservedHour, null);
  const [early] = researchAt(data.history, 20);
  assert.equal(early.entries[0].outcomeObservedHour, null);
  assert.deepEqual(early.entries.map(item => [item.token, item.outcome, item.endedByHour]), [['tech_alpha', 'unknown', 10]]);
  const [late] = researchAt(data.history, 50);
  assert.deepEqual(late.entries.map(item => item.outcome), ['completed', 'active']);
  assert.deepEqual(researchAt(data.history, 5)[0].entries.map(item => [item.outcome, item.endedByHour]), [['active', null]]);
  assert.deepEqual(researchAt(data.history, Number.NaN), []);
});

test('the slots at an hour join their slot and assignment; an hour without a read joins nothing and guesses nothing', () => {
  const data = validateResearchReply(reply([entry('tech_alpha', 0, 10), entry('tech_beta', 10, null)]));
  const now: ResearchSlotsAt = { hour: 30, ...validateResearchSlots({ tag: 'MOD', readHour: 24, missing: null, warnings: [],
    grants: { nextId: 1, grants: [], unavailable: null },
    slots: [{ index: 0, state: state('tech_beta', 24, 10, { lastHour: 30, details: { remainingDays: 9, estimateRaw: '850000', bonuses: [], modifiers: null } }),
      progress: { firstHour: 24, lastHour: 24, entryFirstObservedHour: 10, baseDaysRaw: '1000000', progressRaw: '250000' }, definition: null }] }) };
  const [clipped] = researchAt(data.history, 30);
  const slot = { ...clipped!, states: [now.slots[0]!.state], progress: [now.slots[0]!.progress!] };
  assert.equal(researchEntryAt(slot, now.slots[0]!.state, 30)?.token, 'tech_beta');
  assert.throws(() => validateResearchSlots({ ...now, slots: [now.slots[0], now.slots[0]] }), /do not match/);
  assert.throws(() => validateResearchSlots({ ...now, grants: { nextId: 2 ** 40, grants: [], unavailable: null } }), /Invalid/);
  const estimate = (remainingDays: number, estimateRaw: string) => () => validateResearchSlots({ ...now,
    slots: [{ ...now.slots[0], state: { ...now.slots[0].state, details: { remainingDays, estimateRaw, bonuses: [], modifiers: null } } }] });
  assert.doesNotThrow(estimate(2, '100001'), 'an estimate just over a day is shown as two');
  assert.doesNotThrow(estimate(1, '100000'));
  assert.doesNotThrow(estimate(0, '0'));
  assert.throws(estimate(2, '100000'), /do not match/, 'an estimate of exactly one day is never two');
  const unjoinedRead = { ...now.slots[0], state: { ...now.slots[0].state, entryFirstObservedHour: null } };
  assert.throws(() => validateResearchSlots({ ...now, slots: [unjoinedRead] }), /do not match/, 'an unjoined read carries its own definition');
  assert.doesNotThrow(() => validateResearchSlots({ ...now, slots: [{ ...unjoinedRead, definition: { token: 'tech_beta', databaseIndex: 3, name: null, description: null } }] }));
  assert.throws(() => validateResearchSlots({ ...now, slots: [{ ...unjoinedRead, definition: { token: 'tech_beta', databaseIndex: 4, name: null, description: null } }] }), /do not match/);
});

test('a breakdown\'s reads join the slots of its parts in hour order', () => {
  const data = validateResearchReply({ ...reply([]), history: { coverage: [{ firstHour: 0, lastHour: 100 }], slots: [
    { index: 0, availability: [{ firstHour: 0, lastHour: 100 }], entries: [entry('tech_alpha', 0, 10, 'switched')] },
    { index: 1, availability: [{ firstHour: 0, lastHour: 100 }], entries: [entry('tech_alpha', 10, 40)] },
  ] } });
  const breakdown = validateResearchBreakdown({ warnings: [], parts: [
    { slot: 1, firstObservedHour: 10, states: [state('tech_alpha', 10, 10, { lastHour: 33 }), state('tech_alpha', 34, 10)], progress: [], missing: null },
    { slot: 0, firstObservedHour: 0, states: [state('tech_alpha', 0, 0, { lastHour: 9 })], progress: [], missing: null },
  ] });
  const joined = withBreakdown(data.history.slots, breakdown);
  assert.deepEqual(joined.map(slot => slot.states.map(item => item.firstHour)), [[0], [10, 34]]);
  assert.throws(() => validateResearchBreakdown({ warnings: [], parts: [{ slot: 0, firstObservedHour: 0, states: [state('tech_alpha', 10, 0), state('tech_alpha', 0, 0)], progress: [], missing: null }] }), /does not match/);
});

test('modifier playback uses the hour\'s day, exact fixed point, and the minimum speed', () => {
  const hour = 43_800_000 + 1937 * 8760;
  const modified = (total: string) => state('tech_alpha', hour, 0, { details: { remainingDays: null, estimateRaw: null, bonuses: [], modifiers: { components: [], totalBonusRaw: total, year: 1938, aheadFactorRaw: '200000', minimumSpeedRaw: '10000' } } });
  assert.deepEqual(researchRateAt(modified('20000'), hour), { rateRaw: '40000', penaltyRaw: '200000', floored: false });
  assert.ok(BigInt(researchRateAt(modified('20000'), hour + 24)!.rateRaw) > 40000n, 'the ahead penalty falls each day');
  assert.deepEqual(researchRateAt(modified('-250000'), hour), { rateRaw: '10000', penaltyRaw: '200000', floored: true });
  assert.equal(researchRateAt(state('tech_alpha', hour, 0), hour), null, 'no modifiers, no rate');
});

test('a repeated technology is named and placed by the definition each assignment recorded', () => {
  const data = validateResearchReply(reply([entry('tech_alpha', 0, 10, 'completed', 3), entry('tech_alpha', 10, 20, 'completed', 4), entry('tech_beta', 20, null, 'active', 5)], {
    artwork: [{ token: 'tech_alpha', name: 'Alpha', description: null, baseDurationDays: null, icon: null },
      { token: 'tech_beta', name: 'Beta', description: null, baseDurationDays: null, icon: null }],
    definitions: [{ token: 'tech_alpha', databaseIndex: 3, name: 'Alpha I', description: null }, { token: 'tech_alpha', databaseIndex: 4, name: 'Alpha II', description: 'Better' }],
    folders: [{ key: 'folder_one', name: 'Folder one' }, { key: 'folder_two', name: 'Folder two' }],
    technologyFolders: [{ token: 'tech_alpha', databaseIndex: 3, folder: 'folder_one' }, { token: 'tech_alpha', databaseIndex: 4, folder: 'folder_two' }, { token: 'tech_beta', folder: 'folder_two' }],
  }));
  assert.deepEqual([...researchRecordedTemplates(data.history)], [['tech_alpha', [3, 4]], ['tech_beta', [5]]]);
  const artwork = researchArtwork(data, new Map());
  assert.equal(researchArt(artwork, 'tech_alpha', 4)?.name, 'Alpha II');
  assert.equal(researchArt(artwork, 'tech_alpha', 3)?.name, 'Alpha I');
  assert.equal(researchArt(artwork, 'tech_beta', 5)?.name, 'Beta', 'a token recorded with one definition is named by it');
  assert.equal(researchArt(artwork, 'tech_beta', 6), undefined, 'another definition sharing the token is never named by it');
  const unjoined = { index: 0, progress: null, state: state('tech_beta', 30, null, { databaseIndex: 6 }),
    definition: { token: 'tech_beta', databaseIndex: 6, name: 'Beta (variant)', description: null } };
  assert.equal(researchReadName(artwork, unjoined), 'Beta (variant)', 'an unjoined read is named by its own definition');
  assert.equal(researchReadName(artwork, { ...unjoined, state: { ...unjoined.state, databaseIndex: 5, entryFirstObservedHour: 20 }, definition: null }), 'Beta');
});
