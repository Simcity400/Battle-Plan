import assert from 'node:assert/strict';
import test from 'node:test';
import { FAILED_FACT, nameKey } from '../../dashboard/countryFacts.ts';
import type { FlagRead } from '../../dashboard/countryFlagsAt';
import { casualtyCountryCell } from './casualtyFlags.ts';

const row = { enemyTag: 'Opaque enemy [x:y]', enemyHour: 23 };
const key = nameKey(row.enemyHour, row.enemyTag);

test('an ended enemy table keeps its supplying facts rather than the later tag replacement', () => {
  const future = nameKey(29, row.enemyTag);
  const names = new Map([[key, 'Supplying enemy'], [future, 'Replacement']]);
  const flags = new Map([[key, { url: 'supplying-flag' }], [future, { url: 'replacement-flag' }]]);
  assert.deepEqual(casualtyCountryCell(row, names, flags), { name: 'Supplying enemy', flag: 'supplying-flag', flagPending: false, flagReason: undefined });
  flags.delete(key);
  const pending = casualtyCountryCell(row, names, flags);
  assert.equal(pending.flag, undefined);
  assert.equal(pending.flagPending, true);
  assert.equal(pending.name, 'Supplying enemy');
});

test('native flag reasons, completed empty reads and failures remain distinct from pending', () => {
  const names = new Map([[key, '']]);
  const facts: (FlagRead | null | typeof FAILED_FACT)[] = [{ reason: 'Native flag content unavailable' }, null, FAILED_FACT];
  const reasons = ['Native flag content unavailable', 'No readable flag was returned for this country at this hour.', 'The flag request failed.'];
  for (const [index, fact] of facts.entries()) {
    assert.deepEqual(casualtyCountryCell(row, names, new Map([[key, fact]])),
      { name: '', flag: undefined, flagPending: false, flagReason: reasons[index] });
  }
});
