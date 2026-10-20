import assert from 'node:assert/strict';
import test from 'node:test';
import { FAILED_FACT, MAX_NAME_HOURS, nameKey } from './countryFacts.ts';
import { holderLooks, holderReads, nameReadText } from './holderLooks.ts';
import { lookPlan } from './countryLooks.ts';

const tokens = { surface: '#ffffff', muted: '#888888', countryColours: 'raw' as const };

test('a dated name is loading only while its read is going, and unavailable once it completed without one or failed', () => {
  assert.equal(nameReadText(undefined), 'Loading name…');
  assert.equal(nameReadText(null), 'Country name unavailable');
  assert.equal(nameReadText(FAILED_FACT), 'Country name unavailable');
  assert.equal(nameReadText('A mod country'), 'A mod country');
});

test('holders reusing a tag inside one look sampling interval are each read at their own hour', () => {
  // The branch starts at 0, the tag passes to another country at 24 and the moment shown is 48:
  // a monthly sample would read both at 0, the earlier holder's hour.
  const branch = { firstHour: 0, lastHour: 2000, gaps: [] };
  assert.deepEqual(lookPlan(branch, ['mod:Ω'], 48).wanted, [{ tag: 'mod:Ω', hour: 0 }], 'what sampling would read');
  const holders = [{ key: 'earlier', tag: 'mod:Ω', hour: 10 }, { key: 'later', tag: 'mod:Ω', hour: 48.5 }];
  assert.deepEqual(holderReads(holders).wanted, [{ hour: 10, tag: 'mod:Ω' }, { hour: 48, tag: 'mod:Ω' }]);
  assert.equal(holderReads(Array.from({ length: MAX_NAME_HOURS + 5 }, (_, hour) => ({ key: String(hour), tag: 'A', hour }))).maxHours, MAX_NAME_HOURS + 5,
    'every holder’s hour stays held, so long histories never cycle reads');
  const styles = new Map<string, unknown>([[nameKey(10, 'mod:Ω'), { tag: 'mod:Ω', name: 'Earlier', color: [255, 0, 0] }]]);
  const pending = holderLooks(holders, styles as never, tokens);
  assert.deepEqual([pending.name('earlier'), pending.name('later')], ['Earlier', 'Loading name…'], 'the later holder never borrows the earlier one’s name while its own read is going');
  assert.equal(pending.colour('earlier'), '#ff0000');
  assert.notEqual(pending.colour('later'), '#ff0000', 'nor its colour: it keeps its palette colour until its own read arrives');
  styles.set(nameKey(48, 'mod:Ω'), { tag: 'mod:Ω', name: null, color: null });
  const missing = holderLooks(holders, styles as never, tokens);
  assert.equal(missing.name('later'), 'Country name unavailable', 'a completed read without a name is not loading for ever');
  assert.notEqual(missing.revision, pending.revision, 'a completed read redraws');
  styles.set(nameKey(48, 'mod:Ω'), FAILED_FACT);
  assert.equal(holderLooks(holders, styles as never, tokens).name('later'), 'Country name unavailable');
  assert.equal(pending.name('not drawn'), 'Country name unavailable', 'a key it was never asked for is not loading');
});
