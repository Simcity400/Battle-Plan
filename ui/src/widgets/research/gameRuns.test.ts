import assert from 'node:assert/strict';
import test from 'node:test';
import type { GameRun } from '../../generated/dashboard';
import { plainRuns, toneClass, trimLabel, trimValue, validGameRuns } from './gameRuns.ts';
import { validateEntityStats } from './entityStats.ts';
import { validateTechnologyEffects } from './technologyEffects.ts';

const runs: GameRun[] = [
  { kind: 'icon', sprite: 'GFX_unit_icon|1' },
  { kind: 'text', text: 'Enables ', tone: null },
  { kind: 'link', entity: 'equipment', token: 'infantry_equipment_1', databaseIndex: null, text: 'Infantry Equipment I', tone: 'H' },
  { kind: 'text', text: '\n   Soft:', tone: null },
  { kind: 'text', text: '+0.17', tone: 'G' },
];

test('game text runs validate every run shape and refuse unknown ones', () => {
  assert.equal(validGameRuns(runs), true);
  assert.equal(validGameRuns([{ kind: 'flag', tag: 'GER' }]), true);
  assert.equal(validGameRuns([{ kind: 'link', entity: 'technology', token: 'mod_technology', databaseIndex: null, text: 'Gun Carriages', tone: null }]), true);
  assert.equal(validGameRuns([{ kind: 'text', text: 'x', tone: 'GG' }]), false);
  assert.equal(validGameRuns([{ kind: 'link', entity: 'planet', token: 'x', databaseIndex: null, text: 'x', tone: null }]), false);
  assert.equal(validGameRuns([{ kind: 'link', entity: 'module', token: '', databaseIndex: null, text: 'x', tone: null }]), false);
  assert.equal(validGameRuns([{ kind: 'link', entity: 'technology', token: 'mod_technology', databaseIndex: 6, text: 'Second', tone: null }]), true, 'a link names the template it resolved');
  assert.equal(validGameRuns([{ kind: 'link', entity: 'technology', token: 'mod_technology', databaseIndex: 0, text: 'x', tone: null }]), false);
  assert.equal(validGameRuns([{ kind: 'link', entity: 'technology', token: 'mod_technology', text: 'x', tone: null }]), false);
  assert.equal(validGameRuns([{ kind: 'bold', text: 'x' }]), false);
  assert.equal(plainRuns(runs), 'Enables Infantry Equipment I\n   Soft:+0.17');
});

test('game colour codes map to our tones and unknown codes stay plain', () => {
  assert.equal(toneClass('G'), 'positive');
  assert.equal(toneClass('R'), 'negative');
  assert.equal(toneClass('H'), 'strong');
  assert.equal(toneClass(null), null);
  assert.equal(toneClass('Z'), null);
});

test('stat labels lose the game separator and values their padding, keeping icons', () => {
  const label: GameRun[] = [{ kind: 'icon', sprite: 'production_cost' }, { kind: 'text', text: ' Production Cost: ', tone: null }];
  assert.deepEqual(trimLabel(label), [{ kind: 'icon', sprite: 'production_cost' }, { kind: 'text', text: ' Production Cost', tone: null }]);
  assert.deepEqual(trimValue([{ kind: 'text', text: ' ', tone: null }, { kind: 'text', text: '+2.0', tone: 'G' }]), [{ kind: 'text', text: '+2.0', tone: 'G' }]);
});

test('effect and stat replies are validated whole', () => {
  const effects = { hour: 4, runs, refusals: { 'why': 1 }, unavailable: null, warnings: [] };
  assert.equal(validateTechnologyEffects(effects), effects);
  assert.throws(() => validateTechnologyEffects({ ...effects, runs: undefined, text: 'old' }));
  const stats = { kind: 'tactic', token: 'mod_tactic', name: null, stats: [{ label: runs.slice(1, 2), value: runs.slice(4), source: 'On cas missions' }], refusals: {}, unavailable: 'Building stats are not read', warnings: [] };
  assert.equal(validateEntityStats(stats), stats);
  assert.throws(() => validateEntityStats({ ...stats, kind: 'planet' }));
  assert.throws(() => validateEntityStats({ ...stats, stats: [{ label: 'Soft', value: runs, source: null }] }));
  assert.throws(() => validateEntityStats({ ...stats, stats: [{ label: runs, value: runs }] }), 'a stat without its source is not this contract');
});
