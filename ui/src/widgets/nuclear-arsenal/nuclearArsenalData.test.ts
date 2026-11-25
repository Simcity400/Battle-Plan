import assert from 'node:assert/strict';
import test from 'node:test';
import { typeName, validateNuclearArsenal } from './nuclearArsenalData.ts';
import type { NuclearArsenalReply } from '../../generated/dashboard';

const REPLY: NuclearArsenalReply = { missing: null, warnings: [],
  types: [{ ordinal: 0, token: 'nuclear_bomb', name: 'Nuclear Bomb' }, { ordinal: 1, token: 'thermonuclear_bomb', name: null }, { ordinal: 2, token: null, name: null }],
  countries: [{ tag: 'USA', countryId: 1, slot: null, total: 7, counts: [{ ordinal: 0, value: 5, ready: 1 }, { ordinal: 2, value: 2, ready: 0 }] }] };

test('a weapon type is named by the content, else its token, else its ordinal: never dropped', () => {
  assert.deepEqual(REPLY.types.map(typeName), ['Nuclear Bomb', 'thermonuclear_bomb', 'Unnamed nuclear type 2']);
  assert.deepEqual(validateNuclearArsenal(REPLY), REPLY);
  assert.throws(() => validateNuclearArsenal({ ...REPLY, countries: [{ tag: 'USA', total: 1.5, counts: [] }] }));
});
