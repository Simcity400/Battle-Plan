import assert from 'node:assert/strict';
import test from 'node:test';
import { nukeParts, partName, validateNuclearArsenal } from './nuclearArsenalData.ts';
import type { NuclearArsenalReply } from '../../generated/dashboard';

const REPLY: NuclearArsenalReply = { missing: null, warnings: [],
  types: [{ ordinal: 0, token: 'nuclear_bomb', name: 'Nuclear Bomb' }, { ordinal: 1, token: 'thermonuclear_bomb', name: null }, { ordinal: 2, token: null, name: null }],
  countries: [{ tag: 'USA', countryId: 1, total: 7, counts: [{ ordinal: 0, value: 5, ready: 1 }, { ordinal: 2, value: 2, ready: 0 }] }] };

test('a weapon type is named by the content, else its token, else its ordinal: never dropped', () => {
  assert.deepEqual(['0', '1', '2', '9'].map(part => partName(REPLY, part)),
    ['Nuclear Bomb', 'thermonuclear_bomb', 'Unnamed nuclear type 2', 'Unnamed nuclear type 9']);
  assert.equal(partName(undefined, '0'), 'Unnamed nuclear type 0', 'before the names land a part is still shown');
  const parts = nukeParts(REPLY);
  assert.equal(parts.kind, 'keyed');
  assert.deepEqual(parts.kind === 'keyed' && [parts.family, parts.partBits], [{ kind: 'country-nukes', measure: 'amount' }, 32]);
  assert.throws(() => validateNuclearArsenal({ ...REPLY, countries: [{ tag: 'USA', total: 1.5, counts: [] }] }));
});
