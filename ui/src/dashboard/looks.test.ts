import assert from 'node:assert/strict';
import test from 'node:test';
import { FAILED_FACT, FactRequests, factsKey, HourStore, nameKey, transientError } from './countryFacts.ts';
import { lookHour, lookPlan, LOOK_STEP } from './countryLooks.ts';
import { matchesLookSegment, nextLooks, retainLooks, sampledLooks } from './looks.ts';
import type { Look, LookRead } from './looks';
import { BROWSER_BRIDGE_BUSY, CAPACITY_REACHED, MAP_CONTROLS_BUSY, NATIVE_QUEUE_FULL, TRANSIENT_ERRORS } from '../generated/dashboard.ts';

const germany: Look = { name: 'Germany', color: [1, 2, 3], flag: 'data:ger' };
const style = { name: 'Germany', color: [1, 2, 3] as [number, number, number] };

test('each part of a look still being read keeps the one shown; a completed read replaces it, with nothing known when it gave nothing', () => {
  const shown = new Map<string, Look>([['GER', germany]]);
  const read = (value: LookRead) => nextLooks(shown, ['GER'], () => value).get('GER');
  assert.equal(read({ style: undefined, flag: undefined }), germany);
  assert.deepEqual(read({ style: { name: 'German Reich', color: [9, 9, 9] }, flag: 'data:reich' }), { name: 'German Reich', color: [9, 9, 9], flag: 'data:reich' });
  assert.deepEqual(read({ style: null, flag: null }), { name: null, color: null, flag: null });
  assert.equal(nextLooks(shown, ['ITA'], () => ({ style: null, flag: null })).get('GER'), germany);
});

test('a style applies on its own: flags that cannot be had (no game folders) leave the colours shown', () => {
  const looks = nextLooks(new Map(), ['GER'], () => ({ style, flag: null }));
  assert.deepEqual(looks.get('GER'), { name: 'Germany', color: [1, 2, 3], flag: null });
  assert.deepEqual(nextLooks(new Map(), ['GER'], () => ({ style, flag: undefined })).get('GER'), { name: 'Germany', color: [1, 2, 3], flag: null });
  const later = nextLooks(looks, ['GER'], () => ({ style: undefined, flag: 'data:ger' }));
  assert.deepEqual(later.get('GER'), germany);
});

test('only refusals for want of room, cancellations and an unreachable service are tried again', () => {
  for (const refusal of TRANSIENT_ERRORS) {
    assert.equal(transientError(new Error(refusal)), true, refusal);
    assert.equal(transientError(`${refusal}. Retry this view.`), true);
  }
  for (const refusal of [CAPACITY_REACHED, `${CAPACITY_REACHED}. Retry this view.`, BROWSER_BRIDGE_BUSY, NATIVE_QUEUE_FULL, `${MAP_CONTROLS_BUSY}; try again`]) {
    assert.equal(transientError(new Error(refusal)), true, refusal);
  }
  assert.deepEqual([...TRANSIENT_ERRORS], [CAPACITY_REACHED, BROWSER_BRIDGE_BUSY, NATIVE_QUEUE_FULL, MAP_CONTROLS_BUSY]);
  assert.equal(transientError(new DOMException('Request cancelled', 'AbortError')), true);
  assert.equal(transientError(new TypeError('Failed to fetch')), true);
  assert.equal(transientError(new Error('Choose the game and user folders in Settings first')), false);
  assert.equal(transientError(new Error('Country styles unavailable at this hour: gap')), false);
});

test('looks are read at a recorded hour of the moment’s own stretch, never in a gap', () => {
  const branch = { firstHour: 100, lastHour: 3000, gaps: [{ firstHour: 820, lastHour: 820 }, { firstHour: 1600, lastHour: 1700 }] };
  assert.deepEqual(lookHour(branch, 500), { hour: 100, stretch: 100 });
  assert.deepEqual(lookHour(branch, 821), { hour: 821, stretch: 821 });
  assert.deepEqual(lookHour(branch, 1500), { hour: 821, stretch: 821 });
  assert.deepEqual(lookHour(branch, 1550), { hour: 100 + 2 * LOOK_STEP, stretch: 821 });
  assert.equal(lookHour(branch, 1650), null);
  assert.deepEqual(lookHour(branch, 1701), { hour: 1701, stretch: 1701 });
  assert.equal(lookHour(branch, 50), null);
  assert.equal(lookHour(branch, 3001), null);
});

test('a country taking over a tag starts its own appearance sample, even without a recording gap', () => {
  const branch = { firstHour: 100, lastHour: 3000, gaps: [] };
  assert.deepEqual(lookHour(branch, 199, 100), { hour: 100, stretch: 100 });
  assert.deepEqual(lookHour(branch, 200, 200), { hour: 200, stretch: 200 });
  assert.deepEqual(lookHour(branch, 819, 200), { hour: 200, stretch: 200 });
  assert.deepEqual(lookHour(branch, 820, 200), { hour: 820, stretch: 200 });
  assert.deepEqual(lookHour(branch, 199, 100), { hour: 100, stretch: 100 });
  assert.equal(lookHour(branch, 200, null), null);
});

test('looks never retain an earlier identity on takeover or a backward seek; other countries keep theirs', () => {
  const shown = new Map<string, Look>([['ABC', germany], ['opaque,"tag', germany]]);
  const before = new Map([['ABC', 100], ['opaque,"tag', 100]]);
  const after = new Map([['ABC', 200], ['opaque,"tag', 100]]);
  const retained = retainLooks(shown, before, after);
  const loading = nextLooks(retained, [...after.keys()], () => ({ style: undefined, flag: undefined }));
  assert.equal(loading.has('ABC'), false);
  assert.equal(loading.get('opaque,"tag'), germany);
  const current = nextLooks(loading, ['ABC'], () => ({ style: { name: 'New country', color: [9, 8, 7] }, flag: 'new flag' }));
  assert.equal(current.get('ABC')?.name, 'New country');
  assert.equal(retainLooks(current, after, before).has('ABC'), false);
  assert.equal(retainLooks(current, after, new Map()).size, 0);
});

test('all look parts use one sample inside each identity segment', () => {
  const branch = { firstHour: 100, lastHour: 3000, gaps: [] };
  const segment = (tag: string, hour: number) => tag === 'unknown' ? null : tag === 'ABC' && hour >= 200 ? 200 : 100;
  const plan = lookPlan(branch, ['ABC', 'other', 'ABC', 'unknown'], 205.5, segment);
  assert.deepEqual(plan.samples, [{ tag: 'ABC', hour: 200, stretch: 200 }, { tag: 'other', hour: 100, stretch: 100 }]);
  assert.deepEqual(plan.wanted, [{ tag: 'ABC', hour: 200 }, { tag: 'other', hour: 100 }]);
  assert.deepEqual(lookPlan(branch, ['ABC'], 205).samples, [{ tag: 'ABC', hour: 100, stretch: 100 }]);
  assert.equal(lookPlan(branch, ['unknown'], 205, segment).wanted.length, 0);
  assert.equal(lookPlan({ ...branch, gaps: [{ firstHour: 205, lastHour: 206 }] }, ['ABC'], 205, segment).wanted.length, 0);
});

test('without an identity lookup looks still follow the recorded sample grid', () => {
  const branch = { firstHour: 100, lastHour: 3000, gaps: [{ firstHour: 800, lastHour: 830 }] };
  assert.deepEqual(lookPlan(branch, ['ABC'], 102).samples, [{ tag: 'ABC', hour: 100, stretch: 100 }]);
  assert.deepEqual(lookPlan(branch, ['ABC'], 831).samples, [{ tag: 'ABC', hour: 831, stretch: 831 }]);
});

test("a coarse step of the head is a step from the branch's first hour, never one before it", () => {
  const branch = { firstHour: 1000, lastHour: 100_000, gaps: [] };
  assert.ok(Math.floor(1000 / LOOK_STEP) * LOOK_STEP < branch.firstHour);
  assert.equal(lookHour(branch, 1000)?.hour, 1000);
  assert.equal(lookHour(branch, 1000 + LOOK_STEP - 1)?.hour, 1000);
  assert.equal(lookHour(branch, 1000 + LOOK_STEP)?.hour, 1000 + LOOK_STEP);
});

test('per-country label hours share the sample grid and their own identity bound', () => {
  const branch = { firstHour: 100, lastHour: 3000, gaps: [] };
  const hours = new Map([['early', 199], ['late', 205]]);
  const segment = (_tag: string, hour: number) => hour >= 200 ? 200 : 100;
  assert.deepEqual(lookPlan(branch, ['early', 'late', 'not drawn'], 3000, segment, hours).samples,
    [{ tag: 'early', hour: 100, stretch: 100 }, { tag: 'late', hour: 200, stretch: 200 }]);
});

test('many line label samples stay wanted until every read completes, without cycling', async () => {
  const hours = new Map(Array.from({ length: 140 }, (_, index) => [`tag ${index}`, 100 + index * LOOK_STEP]));
  const plan = lookPlan({ firstHour: 100, lastHour: 200 * LOOK_STEP, gaps: [] }, [...hours.keys()], 200 * LOOK_STEP, undefined, hours);
  assert.equal(plan.maxHours, hours.size);
  let calls = 0;
  const store = new HourStore<string>(64);
  const requests = new FactRequests(store, async (plan, _signal, done) => {
    calls++;
    await Promise.resolve();
    done(plan[0][0], plan[0][1], new Map(plan[0][1].map(tag => [tag, tag])));
  }, () => {});
  requests.want(plan.wanted, plan.maxHours, 64);
  for (let tick = 0; tick < 500; tick++) await Promise.resolve();
  assert.equal(store.pick(plan.wanted).size, hours.size);
  assert.equal(calls, hours.size);
  assert.equal(requests.want(plan.wanted, plan.maxHours, 64), 0);
  requests.cancel();
});

test('an exiting row keeps its look without a new read, only while its identity is unchanged', () => {
  const before = { looks: new Map([['ABC', germany]]), segments: new Map([['ABC', 100]]) };
  const unexpectedRead = () => { throw new Error('an exiting country must not be requested'); };
  assert.equal(sampledLooks(before, [], unexpectedRead, new Map([['ABC', 100]])).looks.get('ABC'), germany);
  assert.equal(sampledLooks(before, [], unexpectedRead, new Map([['ABC', 200]])).looks.has('ABC'), false);
});

test('a line end moving across a takeover cannot use the previous render\'s look while its reads replan', () => {
  const branch = { firstHour: 100, lastHour: 1000, gaps: [] };
  const segment = (_tag: string, hour: number) => hour >= 200 ? 200 : 100;
  const before = lookPlan(branch, ['ABC'], 1000, segment, new Map([['ABC', 199]]));
  const held = new Map(before.samples.map(sample => [sample.tag, sample.stretch]));
  assert.equal(matchesLookSegment(held, 'ABC', lookHour(branch, 199, segment('ABC', 199))!.stretch), true);
  assert.equal(matchesLookSegment(held, 'ABC', lookHour(branch, 200, segment('ABC', 200))!.stretch), false);
  assert.equal(matchesLookSegment(held, 'ABC', null), false);
});

test('playback 100 to 101 to 102 starts no new look requests and holds every pending part', async () => {
  const branch = { firstHour: 100, lastHour: 3000, gaps: [] };
  let styleCalls = 0;
  let flagCalls = 0;
  let styleCancelled = false;
  let flagCancelled = false;
  let finishStyle = () => {};
  let finishFlag = () => {};
  const styles = new FactRequests(new HourStore<typeof style>(4), async (plan, signal, done) => {
    styleCalls++;
    signal.addEventListener('abort', () => { styleCancelled = true; });
    await new Promise<void>(resolve => { finishStyle = () => { done(plan[0][0], plan[0][1], new Map()); resolve(); }; });
  }, () => {});
  const flags = new FactRequests(new HourStore<string>(4), async (plan, signal, done) => {
    flagCalls++;
    signal.addEventListener('abort', () => { flagCancelled = true; });
    await new Promise<void>(resolve => { finishFlag = () => { done(plan[0][0], plan[0][1], new Map()); resolve(); }; });
  }, () => {});
  let shown = { looks: new Map([['ABC', germany]]), segments: new Map([['ABC', 100]]) };
  for (const hour of [100, 101, 102]) {
    const plan = lookPlan(branch, ['ABC'], hour);
    assert.equal(styles.want(plan.wanted, 64, 64), hour === 100 ? 1 : 0);
    assert.equal(flags.want(plan.wanted, 64, 32), hour === 100 ? 1 : 0);
    const styleRead = styles.pickRead(plan.wanted);
    const flagRead = flags.pickRead(plan.wanted);
    shown = sampledLooks(shown, plan.samples, item => ({ style: styleRead.get(nameKey(item.hour, item.tag)), flag: flagRead.get(nameKey(item.hour, item.tag)) }));
    assert.equal(shown.looks.get('ABC'), germany);
  }
  assert.deepEqual([styleCalls, flagCalls, styleCancelled, flagCancelled], [1, 1, false, false]);
  finishStyle(); finishFlag();
  await Promise.resolve();
  const plan = lookPlan(branch, ['ABC'], 102);
  shown = sampledLooks(shown, plan.samples, item => ({ style: styles.pickRead(plan.wanted).get(nameKey(item.hour, item.tag)), flag: flags.pickRead(plan.wanted).get(nameKey(item.hour, item.tag)) }));
  assert.deepEqual(shown.looks.get('ABC'), { name: null, color: null, flag: null });
  styles.cancel(); flags.cancel();
});

test('a failed appearance read shows unknown, never the old look or an empty answer', () => {
  const result = nextLooks(new Map([['ABC', germany]]), ['ABC'], () => ({ style: FAILED_FACT, flag: FAILED_FACT }));
  assert.deepEqual(result.get('ABC'), { name: null, color: null, flag: null, nameFailure: FAILED_FACT, flagFailure: FAILED_FACT });
});

test('facts are asked for by the set of countries, so a re-ranking never cancels a read', () => {
  const ranked = [{ hour: 10, tag: 'GER' }, { hour: 10, tag: 'ITA' }, { hour: 10, tag: 'SOV' }];
  const reordered = [ranked[2], ranked[0], ranked[1], ranked[0]];
  assert.equal(factsKey('track', reordered), factsKey('track', ranked));
  assert.notEqual(factsKey('track', ranked.slice(1)), factsKey('track', ranked));
  assert.notEqual(factsKey('other', ranked), factsKey('track', ranked));
  assert.notEqual(factsKey('track', [{ hour: 11, tag: 'GER' }]), factsKey('track', [{ hour: 10, tag: 'GER' }]));
  assert.notEqual(factsKey('track', [{ hour: 10, tag: 'A","B' }]), factsKey('track', [{ hour: 10, tag: 'A' }, { hour: 10, tag: 'B' }]));
});

test('the fact store tells a read without a value from one still going', () => {
  const store = new HourStore<string>(4);
  const wanted = [{ hour: 10, tag: 'GER' }, { hour: 10, tag: 'ITA' }, { hour: 20, tag: 'GER' }];
  store.record(10, ['GER', 'ITA'], new Map([['GER', 'Germany']]));
  const read = store.pickRead(wanted);
  assert.equal(read.get(nameKey(10, 'GER')), 'Germany');
  assert.equal(read.get(nameKey(10, 'ITA')), null);
  assert.equal(read.has(nameKey(20, 'GER')), false);
  assert.deepEqual([...store.pick(wanted).keys()], [nameKey(10, 'GER')]);
});


test('exact dated looks use every row read hour while preserving identity and recording bounds', () => {
  const branch = { firstHour: 100, lastHour: 3000, gaps: [{ firstHour: 210, lastHour: 220 }] };
  const hours = new Map([['first', 119], ['changed holder', 205], ['gap', 215], ['after gap', 221]]);
  const segment = (tag: string, hour: number) => tag === 'changed holder' && hour >= 200 ? 200 : 100;
  const plan = lookPlan(branch, [...hours.keys()], 250, segment, hours, true);
  assert.deepEqual(plan.samples, [
    { tag: 'after gap', hour: 221, stretch: 221 },
    { tag: 'changed holder', hour: 205, stretch: 200 },
    { tag: 'first', hour: 119, stretch: 100 },
  ]);
  const before = { looks: new Map([['changed holder', germany], ['first', germany]]), segments: new Map([['changed holder', 100], ['first', 100]]) };
  const pending = sampledLooks(before, plan.samples, () => ({ style: undefined, flag: undefined }));
  assert.equal(pending.looks.has('changed holder'), false);
  assert.equal(pending.looks.get('first'), germany);
  const missing = sampledLooks(pending, plan.samples, () => ({ style: null, flag: FAILED_FACT }));
  assert.deepEqual(missing.looks.get('first'), { name: null, color: null, flag: null, flagFailure: FAILED_FACT });
});
