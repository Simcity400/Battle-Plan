import assert from 'node:assert/strict';
import test from 'node:test';
import { validateCombatLog } from './combatLogData.ts';

const reply = () => ({ hour: 5, groups: [], manpower: [], equipment: [], templates: [], battles: [], active: [], warnings: [],
  availability: { groups: null, manpower: null, months: null, ownEquipment: null, enemyEquipment: null, templates: null, battles: null, active: null,
    terminal: 'LandCombatResults: not available at this hour' } });

test('a Combat Log reply keeps its named unavailable parts and a malformed one is refused', () => {
  assert.equal(validateCombatLog(reply()).availability.terminal, 'LandCombatResults: not available at this hour');
  assert.throws(() => validateCombatLog({ ...reply(), active: null }), /Invalid Combat Log reply/);
});
