import assert from 'node:assert/strict';
import test from 'node:test';
import { campaignToken, chosenShown, largestToken, toggleChosen } from './chosen.ts';
import type { ChoiceSource } from './chosen';
import { storableToken, UNSTORABLE } from './options.ts';

const LINES: Record<string, number> = { infantry_equipment: 1, artillery_equipment: 1, twice: 2 };
const SOURCE: ChoiceSource = { lines: token => LINES[token] ?? 0, name: token => token.toUpperCase(), noun: 'equipment type' };

test('the automatic choice is the largest recorded token, else the first', () => {
  assert.equal(largestToken([{ token: 'a', size: 5n, recorded: true }, { token: 'b', size: 9n, recorded: true }, { token: 'c', size: 99n, recorded: false }]), 'b');
  assert.equal(largestToken([{ token: 'a', size: 5n, recorded: true }, { token: 'b', size: 5n, recorded: true }]), 'a');
  assert.equal(largestToken([{ token: 'a', size: 0n, recorded: false }]), 'a');
  assert.equal(largestToken([]), null);
});

test('only chosen tokens this recording has are shown, else the automatic one', () => {
  assert.deepEqual(chosenShown(SOURCE, ['mod_gun', 'artillery_equipment'], 'infantry_equipment'), ['artillery_equipment']);
  assert.deepEqual(chosenShown(SOURCE, ['mod_gun'], 'infantry_equipment'), ['infantry_equipment']);
  assert.deepEqual(chosenShown(SOURCE, [], null), []);
  assert.deepEqual(chosenShown(SOURCE, [], ['infantry_equipment', 'artillery_equipment']), ['infantry_equipment', 'artillery_equipment']);
});

test('a chip toggles its own token, keeps other campaigns’ tokens and says why it cannot act', () => {
  assert.deepEqual(toggleChosen(SOURCE, [], ['infantry_equipment'], 'artillery_equipment', 8), { value: 'infantry_equipment artillery_equipment' });
  assert.deepEqual(toggleChosen(SOURCE, ['mod_gun', 'infantry_equipment'], ['infantry_equipment'], 'artillery_equipment', 8),
    { value: 'mod_gun infantry_equipment artillery_equipment' });
  assert.deepEqual(toggleChosen(SOURCE, ['mod_gun', 'infantry_equipment'], ['infantry_equipment'], 'infantry_equipment', 8),
    { reason: 'One equipment type always stays chosen' });
  assert.deepEqual(toggleChosen(SOURCE, ['mod_gun', 'infantry_equipment'], ['infantry_equipment'], 'artillery_equipment', 2), { reason: 'At most 2 can be chosen' });
  assert.deepEqual(toggleChosen(SOURCE, ['artillery_equipment'], ['artillery_equipment'], 'twice', 2), { reason: 'At most 2 lines can be drawn; TWICE adds 2' });
  assert.deepEqual(toggleChosen(SOURCE, ['artillery_equipment', 'twice'], ['artillery_equipment', 'twice'], 'twice', 2), { value: 'artillery_equipment' });
  const invalid = 'two words';
  const source = { ...SOURCE, lines: (token: string) => token === invalid ? 1 : SOURCE.lines(token) };
  assert.deepEqual(toggleChosen(source, [], [invalid], 'artillery_equipment', 8), { value: 'artillery_equipment' });
  assert.deepEqual(toggleChosen(source, [], [invalid], invalid, 8), { reason: UNSTORABLE });
});

test('uncapped callers can choose twenty tokens including a long content name', () => {
  const tokens = Array.from({ length: 20 }, (_, index) => `mod_type_${index}`.padEnd(200, 'x'));
  const source: ChoiceSource = { lines: token => tokens.includes(token) ? 1 : 0, name: token => token, noun: 'type' };
  assert.deepEqual(toggleChosen(source, tokens.slice(0, 19), tokens.slice(0, 19), tokens[19], Number.MAX_SAFE_INTEGER), { value: tokens.join(' ') });
});

test('with no stored token in this campaign, a click toggles from the automatic choice shown', () => {
  const shown = chosenShown(SOURCE, ['old_mod_type'], ['infantry_equipment', 'artillery_equipment']);
  assert.deepEqual(shown, ['infantry_equipment', 'artillery_equipment']);
  assert.deepEqual(toggleChosen(SOURCE, ['old_mod_type'], shown, 'infantry_equipment', 8), { value: 'old_mod_type artillery_equipment' });
  assert.deepEqual(toggleChosen(SOURCE, ['old_mod_type'], shown, 'twice', 8), { value: 'old_mod_type infantry_equipment artillery_equipment twice' });
});

test('a campaign-scoped token differs between campaigns and stays storable', () => {
  const key = '0123456789abcdef0123456789abcdef';
  assert.equal(campaignToken(key, '1351:light_tank_chassis_1'), `${key}:1351:light_tank_chassis_1`);
  assert.notEqual(campaignToken(key, 'x'), campaignToken('fedcba', 'x'));
  assert.ok(storableToken(campaignToken(key, '1351:light_tank_chassis_1')));
  const here = campaignToken(key, '7:tank');
  const source: ChoiceSource = { lines: token => token === here ? 1 : 0, name: token => token, noun: 'variant' };
  assert.deepEqual(chosenShown(source, [campaignToken('fedcba', '7:tank')], here), [here]);
});
