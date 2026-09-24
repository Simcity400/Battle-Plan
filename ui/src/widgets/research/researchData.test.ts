import assert from 'node:assert/strict';
import test from 'node:test';
import { researchAt, researchColumns, researchDuration, researchEffects, researchFixed, researchProgressAt, researchRateAt, researchRequest, researchStateAt, validateResearchReply } from './researchData.ts';
import { loadProduct } from '../../dashboard/products.ts';
import type { ResearchEntry, ResearchHistory, ResearchReply } from '../../generated/dashboard';
import type { DashboardClient, ProductRequest } from '../../dashboard/client';

const request = researchRequest('source', 'v1', { branchId: '1', tag: 'GER' });
const entry = (token: string, start: number, end: number | null, outcome: ResearchEntry['outcome'] = 'completed'): ResearchEntry => ({
  token, technologyHandle: token.charCodeAt(0), continuityStartHour: 100,
  level: 0, firstObservedHour: start, startedAfterHour: start - 1, lastObservedHour: (end ?? 400) - 1, endedByHour: end, outcome,
});
const history: ResearchHistory = { coverage: [{ firstHour: 100, lastHour: 299 }, { firstHour: 301, lastHour: 400 }], slots: [
  { index: 0, availability: [{ firstHour: 100, lastHour: 400 }], states: [], progress: [], entries: [entry('A', 100, 150), entry('B', 150, 250, 'switched'), entry('C', 250, null, 'active')] },
  { index: 1, availability: [{ firstHour: 100, lastHour: 400 }], states: [], progress: [], entries: [entry('D', 100, 220), entry('E', 220, null, 'active')] },
  { index: 2, availability: [{ firstHour: 230, lastHour: 400 }], states: [], progress: [], entries: [entry('F', 235, 245), entry('G', 245, null, 'active')] },
] };
const reply: ResearchReply = { history, artwork: [], warnings: [] };

test('research slots stay sequential; only the new column receives a time-based offset', () => {
  const columns = researchColumns(researchAt(history, 400)!);
  assert.deepEqual(columns.map(c => c.offset), [0, 0, 1]);
  assert.equal(columns[0].slot.entries[1].token, 'B');
  assert.equal(columns[1].slot.entries[1].token, 'E');
  assert.equal(columns[2].slot.entries[1].token, 'G');
  // Completion dates differ: these second entries still use consecutive rows.
  assert.notEqual(columns[0].slot.entries[0].endedByHour, columns[1].slot.entries[0].endedByHour);
});

test('research progress is the recorded sample over its base cost, and absent outside samples', () => {
  const slot = { ...history.slots[0], progress: [
    { firstHour: 250, lastHour: 273, entryFirstObservedHour: 250, baseDaysRaw: '11000000', progressRaw: '2750000' },
    { firstHour: 274, lastHour: 280, entryFirstObservedHour: 250, baseDaysRaw: '11000000', progressRaw: '12000000' },
    { firstHour: 300, lastHour: 310, entryFirstObservedHour: 250, baseDaysRaw: null, progressRaw: '100' },
  ] };
  assert.equal(researchProgressAt(slot, 250, 260), 0.25);
  assert.equal(researchProgressAt(slot, 250, 275), 1);
  assert.equal(researchProgressAt(slot, 250, 290), null, 'no sample in a recording gap');
  assert.equal(researchProgressAt(slot, 250, 305), null, 'unknown cost');
  assert.equal(researchProgressAt(slot, 150, 260), null, 'another assignment');
});

test('rewind hides new slots, future assignments and future completion or switch outcomes', () => {
  const initial = researchAt(history, 120)!;
  assert.equal(initial.length, 2);
  assert.equal(initial[0].entries.length, 1);
  assert.equal(initial[0].entries[0].outcome, 'active');
  assert.equal(initial[0].entries[0].endedByHour, null);
  assert.equal(initial[0].entries[0].lastObservedHour, 120);
  assert.equal(researchAt(history, 150)![0].entries[0].outcome, 'completed');
  assert.equal(researchAt(history, 250)![0].entries[1].outcome, 'switched');
  assert.equal(researchAt(history, 230)![2].entries.length, 0);
  for (const hour of [99, 300, 401]) assert.equal(researchAt(history, hour), null);
});

test('duration is observed time in the slot; partial starts and interrupted history remain explicit', () => {
  assert.equal(researchDuration(entry('A', 100, 148)), '≈ 2 days');
  assert.match(researchDuration({ ...entry('A', 100, 148), startedAfterHour: null }), /^≥ /);
  assert.match(researchDuration({ ...entry('A', 100, 148), outcome: 'unknown' }), /^≥ /);
});

test('compressed countdowns are evaluated at playback hour and delayed completion stays hidden', () => {
  const slot = { ...history.slots[0], entries: [{ ...entry('A', 100, 148), outcomeObservedHour: 150 }], states: [{
    firstHour: 100, lastHour: 147, token: 'A', entryFirstObservedHour: 100,
    speedMultiplierRaw: '100000', bankedDaysRaw: '0', appliedDaysRaw: '0', aheadReductionYearsRaw: '0',
    details: { remainingDays: 10, bonuses: [{ id: 7, token: 'radio_bonus', name: 'Radio bonus', speedBonusRaw: '50000', aheadReductionYearsRaw: '200000' }] },
  }] };
  assert.equal(researchStateAt(slot, 123)?.details?.remainingDays, 10);
  assert.equal(researchStateAt(slot, 124)?.details?.remainingDays, 9);
  assert.equal(researchAt({ ...history, slots: [slot] }, 149)![0].entries[0].outcome, 'unknown');
  assert.equal(researchAt({ ...history, slots: [slot] }, 150)![0].entries[0].outcome, 'completed');
  assert.equal(validateResearchReply({ ...reply, history: { ...history, slots: [slot] } }).history.slots[0], slot);
  const malformed = structuredClone(slot);
  malformed.states[0].details.remainingDays = -1;
  assert.throws(() => validateResearchReply({ ...reply, history: { ...history, slots: [malformed] } }));
});

test('live slot states distinguish idle and missing data without leaking future bonuses', () => {
  const base = { speedMultiplierRaw: '150000', bankedDaysRaw: '0', appliedDaysRaw: '3000000', aheadReductionYearsRaw: '200000' };
  const slot = { ...history.slots[2], states: [
    { ...base, firstHour: 230, lastHour: 234, token: null, entryFirstObservedHour: null, aheadReductionYearsRaw: null },
    { ...base, firstHour: 235, lastHour: 239, token: 'F', entryFirstObservedHour: 235 },
    { ...base, firstHour: 240, lastHour: 244, token: 'F', entryFirstObservedHour: 235, speedMultiplierRaw: '200000' },
  ] };
  const at = researchAt({ ...history, slots: [slot] }, 237)![0];
  assert.equal(researchStateAt(at, 237)?.token, 'F');
  assert.equal(researchStateAt(at, 230)?.token, null);
  assert.equal(researchStateAt(at, 229), null);
  assert.equal(researchStateAt(at, 238), null);
  assert.equal(at.states.at(-1)?.lastHour, 237);
  assert.deepEqual(researchEffects(at, at.entries[0]).map(s => s.speedMultiplierRaw), ['150000']);
  assert.deepEqual(researchEffects(slot, slot.entries[0]).map(s => s.speedMultiplierRaw), ['150000', '200000']);
  const data = { ...reply, history: { ...history, slots: [slot] } };
  assert.equal(validateResearchReply(data), data);
  const malformed = structuredClone(data);
  malformed.history.slots[0].states[1].speedMultiplierRaw = '9223372036854775808';
  assert.throws(() => validateResearchReply(malformed));
  assert.equal(researchFixed('-123456'), '-1.23456');
  assert.equal(researchFixed('9223372036854775807'), '92233720368547.75807');
});

test('research responses reject stale identities, overlapping assignments and unsafe artwork', () => {
  assert.equal(validateResearchReply(reply), reply);
  assert.throws(() => validateResearchReply({ ...reply, history: { ...history, coverage: [...history.coverage, history.coverage[0]] } }));
  const badEntry = { ...history.slots[0].entries[0], endedByHour: 99 };
  assert.throws(() => validateResearchReply({ ...reply, history: { ...history, slots: [{ ...history.slots[0], entries: [badEntry] }] } }));
  assert.throws(() => validateResearchReply({ ...reply, artwork: [{ token: 'A', name: null, description: null, baseDurationDays: null, icon: 'https://example.com/image.png' }] }));
});

test('research widgets share preparation and playback does not fetch per hour', async () => {
  let calls = 0;
  const client = { product: async (input: ProductRequest) => { calls++; return { sourceId: input.sourceId, revision: input.revision, product: input.product, data: reply }; } } as unknown as DashboardClient;
  const results = await Promise.all(Array.from({ length: 12 }, () => loadProduct<ResearchReply>(client, request, new AbortController().signal, validateResearchReply)));
  assert.equal(calls, 1);
  for (let hour = 100; hour <= 400; hour++) researchAt(results[0].history, hour);
  assert.equal(calls, 1);
  await loadProduct(client, researchRequest('source', 'v1', { branchId: '1', tag: 'ENG' }), new AbortController().signal, validateResearchReply);
  assert.equal(calls, 2);
});


test('modifier playback uses the selected day, exact fixed point, and minimum speed', () => {
  const hour = 43800000 + 1937 * 8760;
  const state = { firstHour: hour, lastHour: hour + 8760, token: 'radio', entryFirstObservedHour: hour,
    speedMultiplierRaw: '999999', bankedDaysRaw: '0', appliedDaysRaw: '0', aheadReductionYearsRaw: '0',
    details: { remainingDays: 10, bonuses: [], modifiers: {
      components: [], totalBonusRaw: '20000', year: 1938, aheadFactorRaw: '200000', minimumSpeedRaw: '10000',
    } },
  };
  assert.deepEqual(researchRateAt(state, hour), { rateRaw: '40000', penaltyRaw: '200000', floored: false });
  assert.deepEqual(researchRateAt(state, hour + 23), researchRateAt(state, hour));
  assert.ok(BigInt(researchRateAt(state, hour + 24)!.rateRaw) > 40000n);
  assert.deepEqual(researchRateAt(state, hour + 8760), { rateRaw: '120000', penaltyRaw: '0', floored: false });
  assert.equal(researchRateAt({ ...state, aheadReductionYearsRaw: '100000' }, hour)?.rateRaw, '120000');
  state.details.modifiers.totalBonusRaw = '-200000';
  assert.equal(researchRateAt(state, hour)?.rateRaw, '10000');
  assert.equal(researchRateAt(state, hour)?.floored, true);
});

test('modifier changes survive history compression and rewind hides later values', () => {
  const components = ['country', 'category', 'designTeam', 'experience', 'license', 'scientist', 'applied', 'sharing'].map(kind => ({ kind, raw: '0', sources: [] }));
  const state = { firstHour: 100, lastHour: 199, token: 'A', entryFirstObservedHour: 100,
    speedMultiplierRaw: '100000', bankedDaysRaw: '0', appliedDaysRaw: '0', aheadReductionYearsRaw: '0',
    details: { remainingDays: null, bonuses: [], modifiers: { components, totalBonusRaw: '0', year: 1938, aheadFactorRaw: '200000', minimumSpeedRaw: '10000' } },
  };
  const next = structuredClone(state); next.firstHour = 200; next.lastHour = 299;
  next.details.modifiers.components[0].raw = '25000'; next.details.modifiers.totalBonusRaw = '25000';
  const slot = { ...history.slots[0], availability: [{ firstHour: 100, lastHour: 299 }], entries: [entry('A', 100, 300)], states: [state, next] };
  const data = { ...reply, history: { coverage: [{ firstHour: 100, lastHour: 299 }], slots: [slot] } };
  assert.equal(validateResearchReply(data), data);
  const before = researchAt(data.history, 150)![0];
  assert.equal(researchEffects(before, before.entries[0]).length, 1);
  assert.equal(researchStateAt(before, 150)?.details?.modifiers?.totalBonusRaw, '0');
  assert.equal(researchEffects(slot, slot.entries[0]).length, 2);
  assert.equal(researchStateAt(slot, 200)?.details?.modifiers?.totalBonusRaw, '25000');
  const bad = structuredClone(data); bad.history.slots[0].states[0].details.modifiers.components[0].raw = 'NaN';
  assert.throws(() => validateResearchReply(bad));
});
