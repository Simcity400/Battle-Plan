import assert from 'node:assert/strict';
import test from 'node:test';
import { campaignToken, chosenShown, largestToken, storedChoices, toggleChosen } from './chosen.ts';
import type { ChoiceSource } from './chosen';
import { contentKey, setToken, storableToken, UNSTORABLE } from './options.ts';

/** Infantry equipment and artillery draw a line each; a modded type drawn twice. */
const LINES: Record<string, number> = { infantry_equipment: 1, artillery_equipment: 1, twice: 2 };
const SOURCE: ChoiceSource = { lines: token => LINES[token] ?? 0, name: token => token.toUpperCase(), noun: 'equipment type' };

test('the automatic choice is the largest recorded token, else the first', () => {
  assert.equal(largestToken([{ token: 'a', size: 5n, recorded: true }, { token: 'b', size: 9n, recorded: true }, { token: 'c', size: 99n, recorded: false }]), 'b');
  // Ties keep the first; nothing recorded falls back to the first token.
  assert.equal(largestToken([{ token: 'a', size: 5n, recorded: true }, { token: 'b', size: 5n, recorded: true }]), 'a');
  assert.equal(largestToken([{ token: 'a', size: 0n, recorded: false }]), 'a');
  assert.equal(largestToken([]), null);
});

test('only chosen tokens this recording has are shown, else the automatic one', () => {
  assert.deepEqual(chosenShown(SOURCE, ['mod_gun', 'artillery_equipment'], 'infantry_equipment'), ['artillery_equipment']);
  assert.deepEqual(chosenShown(SOURCE, ['mod_gun'], 'infantry_equipment'), ['infantry_equipment']);
  assert.deepEqual(chosenShown(SOURCE, [], null), []);
  // Several automatic tokens, such as the main types of each arm, show together.
  assert.deepEqual(chosenShown(SOURCE, [], ['infantry_equipment', 'artillery_equipment']), ['infantry_equipment', 'artillery_equipment']);
});

test('a chip toggles its own token, keeps other campaigns’ tokens and says why it cannot act', () => {
  // Nothing stored: the automatic choice becomes explicit, plus the click.
  assert.deepEqual(toggleChosen(SOURCE, [], ['infantry_equipment'], 'artillery_equipment', 8), { value: 'infantry_equipment artillery_equipment' });
  assert.deepEqual(toggleChosen(SOURCE, ['mod_gun', 'infantry_equipment'], ['infantry_equipment'], 'artillery_equipment', 8),
    { value: 'mod_gun infantry_equipment artillery_equipment' });
  // The last token shown stays, named by the source's noun.
  assert.deepEqual(toggleChosen(SOURCE, ['mod_gun', 'infantry_equipment'], ['infantry_equipment'], 'infantry_equipment', 8),
    { reason: 'One equipment type always stays chosen' });
  // The stored count includes tokens this campaign lacks.
  assert.deepEqual(toggleChosen(SOURCE, ['mod_gun', 'infantry_equipment'], ['infantry_equipment'], 'artillery_equipment', 2), { reason: 'At most 2 can be chosen' });
  // A token drawing two lines counts both toward the limit; removing is always allowed.
  assert.deepEqual(toggleChosen(SOURCE, ['artillery_equipment'], ['artillery_equipment'], 'twice', 2), { reason: 'At most 2 lines can be drawn; TWICE adds 2' });
  assert.deepEqual(toggleChosen(SOURCE, ['artillery_equipment', 'twice'], ['artillery_equipment', 'twice'], 'twice', 2), { value: 'artillery_equipment' });
  // A token that cannot be stored never blocks choosing another, and its own chip says why.
  const long = 'x'.repeat(161);
  const source = { ...SOURCE, lines: (token: string) => token === long ? 1 : SOURCE.lines(token) };
  assert.deepEqual(toggleChosen(source, [], [long], 'artillery_equipment', 8), { value: 'artillery_equipment' });
  assert.deepEqual(toggleChosen(source, [], [long], long, 8), { reason: UNSTORABLE });
});

test('with no stored token in this campaign, a click toggles from the automatic choice shown', () => {
  // Stored: another campaign's type only, so the automatic infantry and artillery show.
  const shown = chosenShown(SOURCE, ['old_mod_type'], ['infantry_equipment', 'artillery_equipment']);
  assert.deepEqual(shown, ['infantry_equipment', 'artillery_equipment']);
  // Clicking a shown chip removes it and keeps the rest shown, and the other campaign's token.
  assert.deepEqual(toggleChosen(SOURCE, ['old_mod_type'], shown, 'infantry_equipment', 8), { value: 'old_mod_type artillery_equipment' });
  // Clicking another adds it to what is shown.
  assert.deepEqual(toggleChosen(SOURCE, ['old_mod_type'], shown, 'twice', 8), { value: 'old_mod_type infantry_equipment artillery_equipment twice' });
});

test('a campaign-scoped token differs between campaigns and stays storable', () => {
  const key = '0123456789abcdef0123456789abcdef';
  assert.equal(campaignToken(key, '1351:light_tank_chassis_1'), `${key}:1351:light_tank_chassis_1`);
  assert.notEqual(campaignToken(key, 'x'), campaignToken('fedcba', 'x'));
  assert.ok(storableToken(campaignToken(key, '1351:light_tank_chassis_1')));
  // Chosen in one campaign, it draws nothing in another and is kept.
  const here = campaignToken(key, '7:tank');
  const source: ChoiceSource = { lines: token => token === here ? 1 : 0, name: token => token, noun: 'variant' };
  assert.deepEqual(chosenShown(source, [campaignToken('fedcba', '7:tank')], here), [here]);
});

test('a key a setting cannot store is chosen through its stored token', () => {
  const keys: Record<string, number> = { 'mod/rifles': 1, infantry_equipment: 1 };
  const source = storedChoices({ lines: key => keys[key] ?? 0, name: key => key, noun: 'equipment type' });
  const token = setToken('mod/rifles');
  assert.equal(storableToken('mod/rifles'), false);
  assert.equal(source.lines(token), 1);
  // Chosen, it is stored as its token and read back as the key.
  const action = toggleChosen(source, [], ['infantry_equipment'], token, 8);
  assert.ok('value' in action, JSON.stringify(action));
  assert.deepEqual(action.value.split(' ').map(contentKey), ['infantry_equipment', 'mod/rifles']);
  assert.deepEqual(chosenShown(source, action.value.split(' '), 'infantry_equipment').map(contentKey), ['infantry_equipment', 'mod/rifles']);
});
