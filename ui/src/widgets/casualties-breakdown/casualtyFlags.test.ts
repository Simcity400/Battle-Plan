import assert from 'node:assert/strict';
import test from 'node:test';
import { FAILED_FACT, nameKey } from '../../dashboard/countryFacts.ts';
import { nameReadFrom, LOADING_NAME, ABSENT_NAME, type NameRead } from '../../dashboard/nameReads.ts';
import { RefusedError } from '../../dashboard/queryError.ts';
import type { FlagRead } from '../../dashboard/countryFlags';
import { casualtyCountryCell, casualtyEnemyName } from './casualtyFlags.ts';

const row = { enemyTag: 'Opaque enemy [x:y]', enemyHour: 23 };
const key = nameKey(row.enemyHour, row.enemyTag);

test('an ended enemy table keeps its supplying facts and distinguishes all name states', () => {
  const future = nameKey(29, row.enemyTag);
  const names = new Map<string, NameRead>([[key, nameReadFrom('Supplying enemy', true)], [future, nameReadFrom('Replacement', true)]]);
  const flags = new Map([[key, { url: 'supplying-flag' }], [future, { url: 'replacement-flag' }]]);
  assert.deepEqual(casualtyCountryCell(row, names, flags), { name: 'Supplying enemy', known: true, nameFailed: false,
    flag: 'supplying-flag', flagPending: false, flagReason: undefined });
  flags.delete(key);
  assert.equal(casualtyCountryCell(row, names, flags).flagPending, true);
  assert.equal(casualtyCountryCell(row, names, flags).name, 'Supplying enemy');
  const refusal = new RefusedError('Synthetic enemy identity cannot be resolved');
  for (const [read, text] of [[ABSENT_NAME, 'Country name unavailable'], [LOADING_NAME, 'Loading name…'],
    [nameReadFrom('', true), ''], [nameReadFrom(row.enemyTag, true), row.enemyTag],
    [nameReadFrom(refusal, true), refusal.text], [nameReadFrom(FAILED_FACT, true), 'Couldn’t load country name.']] as const) {
    names.set(key, read);
    assert.equal(casualtyEnemyName(row, names), text);
    const cell = casualtyCountryCell(row, names, flags);
    assert.equal(cell.name, text);
    if (read.kind === 'failed') assert.equal(cell.nameDetail, read.failure.message);
  }
  names.delete(key);
  assert.equal(casualtyEnemyName(row, names), 'Country name unavailable');
});

test('native flag reasons, completed empty reads and failures remain distinct from pending', () => {
  const names = new Map([[key, nameReadFrom('', true)]]);
  const facts: (FlagRead | null | typeof FAILED_FACT)[] = [{ reason: 'Native flag content unavailable' }, null, FAILED_FACT];
  const reasons = ['Native flag content unavailable', 'No readable flag was returned for this country at this hour.', 'The flag request failed.'];
  for (const [index, fact] of facts.entries()) {
    const cell = casualtyCountryCell(row, names, new Map([[key, fact]]));
    assert.equal(cell.flagReason, reasons[index]);
    assert.equal(cell.flagPending, false);
    assert.equal(cell.name, '');
    assert.equal(cell.flagDetail, fact === FAILED_FACT ? FAILED_FACT.message : undefined);
  }
});
