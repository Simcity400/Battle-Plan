import assert from 'node:assert/strict';
import test from 'node:test';
import { encode } from '@msgpack/msgpack';
import { applyStripPatch, decodeState, readStrip } from './decode.ts';
import { stripLattice } from './lattice.ts';
import fixture from './messages.fixture.json' with { type: 'json' };
import { StripStore } from './store.ts';

test('the TypeScript decoder rebuilds the shared states, from the whole and from an omitted anchor, in both directions', t => {
  let statesChecked = 0;
  for (const [states, bytes] of [[fixture.states, fixture.whole], [fixture.states, fixture.patchesOnly],
    ...fixture.arrayCases.map(value => [value.states, value.whole] as const)] as const) {
    const { whole, patches } = readStrip(Uint8Array.from(bytes));
    let state = whole ? decodeState(whole) : structuredClone(states[0]) as ReturnType<typeof decodeState>;
    assert.deepEqual(state, states[0]);
    patches.forEach(([forward], i) => { state = applyStripPatch(state, forward); assert.deepEqual(state, states[i + 1]); });
    for (let i = patches.length - 1; i >= 0; i--) {
      state = applyStripPatch(state, patches[i]![1]);
      assert.deepEqual(state, states[i]);
    }
    statesChecked += 1 + patches.length * 2;
  }
  t.diagnostic(`${statesChecked} state comparisons, 0 differences`);
});

test('framing rejects invalid headers, missing lengths or payload, and trailing bytes and keeps exact wide integers', () => {
  const bytes = Uint8Array.from(fixture.whole);
  for (const [offset, byte] of [[0, 0], [4, 2], [5, 2], [6, 0], [6, 18], [7, 1], [8, 0xff]]) {
    const changed = bytes.slice(); changed[offset!] = byte!;
    assert.throws(() => readStrip(changed), /Invalid dashboard strip/);
  }
  assert.throws(() => readStrip(bytes.subarray(0, 7)), /Invalid dashboard strip/);
  assert.throws(() => readStrip(bytes.subarray(0, 10)), /Invalid dashboard strip/);
  assert.throws(() => readStrip(bytes.subarray(0, -1)), /Invalid dashboard strip/);
  assert.throws(() => readStrip(Uint8Array.of(...bytes, 0)), /Invalid dashboard strip/);
  const state = decodeState(encode({ answer: { zero: 0, big: 2n ** 63n + 1n, signed: -(2n ** 63n) } }, { useBigInt64: true }));
  assert.deepEqual(applyStripPatch(state, encode([{ replace: [['answer', 'zero'], 2n ** 53n + 1n] }], { useBigInt64: true })),
    { answer: { zero: 9007199254740993n, big: 9223372036854775809n, signed: -9223372036854775808n } });
});

test('the lattice uses the finest nested absolute grid, preserves both sides of each gap and freezes the recorded end', () => {
  const offset = 901;
  const branch = { firstHour: 100, lastHour: 87462, gaps: [{ firstHour: 999, lastHour: 1055 }] } as Parameters<typeof stripLattice>[0];
  const lattice = stripLattice(branch, offset);
  assert.equal(lattice.bucket, 192);
  assert.deepEqual(lattice.stretches, [{ first: -801, last: 97 }, { first: 155, last: 86561 }]);
  const full = lattice.hours(16);
  assert.ok(full.includes(97) && full.includes(155));
  assert.ok(full.every(hour => hour < 98 || hour > 154));
  for (const density of [1, 4, 8] as const) assert.ok(lattice.hours(density).every(hour => full.includes(hour)));
  assert.ok(full.filter(hour => (hour + offset + 1) % lattice.bucket === 0).length <= 512);
  const grown = stripLattice({ ...branch, lastHour: branch.lastHour + 100 }, offset);
  assert.equal(lattice.hours(16).at(-1), 86561);
  assert.equal(grown.hours(16).at(-1), 86661);
  const many = stripLattice({ firstHour: 0, lastHour: 899, gaps: Array.from({ length: 299 }, (_, i) => ({
    firstHour: i * 3 + 2, lastHour: i * 3 + 2,
  })) } as Parameters<typeof stripLattice>[0], 0);
  assert.ok(many.hours(1).length > 512);
  assert.ok(many.stretches.every(span => many.hours(1).includes(span.first) && many.hours(1).includes(span.last)));
});

test('the cursor takes a coarse skip across a fine chain and drops only fine bytes', () => {
  const store = new StripStore();
  const [a, b] = fixture.states;
  const whole = Uint8Array.from(fixture.whole);
  const part = (data: Uint8Array) => [data.length & 255, (data.length >> 8) & 255, 0, 0, ...data];
  const frame = (state: unknown, ...changes: unknown[]) => Uint8Array.of(66, 80, 83, 84, 1, state ? 1 : 0, 1 + changes.length / 2, 0,
    ...(state ? part(encode(state)) : []), ...changes.flatMap(change => part(encode(change))));
  const skip = frame(a, [{ replace: [[], b] }], [{ replace: [[], a] }]);
  const fine = frame(null, [], [], [{ replace: [[], b] }], [{ replace: [[], a] }]);
  store.append([0, 10], skip, false);
  store.append([0, 5, 10], fine, true);
  assert.equal(store.bytes, skip.byteLength + fine.byteLength);
  assert.deepEqual(store.at(10), b);
  assert.equal(store.patchesApplied, 1);
  assert.deepEqual(store.at(5), a);
  assert.deepEqual(store.at(0), a);
  assert.equal(store.patchesApplied, 3);
  store.dropFine();
  assert.equal(store.at(5), undefined);
  assert.equal(store.bytes, skip.length);
  assert.ok(readStrip(whole).whole);
});
