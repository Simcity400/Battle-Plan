import assert from 'node:assert/strict';
import test from 'node:test';
import { logCountryRequests, sameCountryRequests } from './logData.ts';
import { occurrenceCountryRequests } from '../../widgets/event-feed/scriptOccurrences.ts';
import type { ScriptOccurrence } from '../../generated/dashboard';

test('log identities use their own hours across a country rename, with opaque tags', () => {
  const rows = [{ key: 'old', hour: 10, item: ['mod: country', null] }, { key: 'new', hour: 20, item: ['mod: country', 'victim'] },
    { key: 'same', hour: 20, item: ['mod: country', 'victim'] }];
  const wanted = logCountryRequests(rows, row => ({ hour: row.hour, tags: row.item }));
  assert.deepEqual(wanted, [{ hour: 10, tag: 'mod: country' }, { hour: 20, tag: 'mod: country' }, { hour: 20, tag: 'victim' }]);
  const nameAt = (hour: number) => hour < 20 ? 'Old name' : 'New name';
  assert.deepEqual(wanted.filter(item => item.tag === 'mod: country').map(item => nameAt(item.hour)), ['Old name', 'New name']);
});

test('log label requests keep all 5000 synthetic countries and deduplicate only equal observations', () => {
  const rows = Array.from({ length: 5000 }, (_, index) => ({ key: String(index), hour: 100, item: `MOD ${index}` }));
  const wanted = logCountryRequests(rows, row => ({ hour: row.hour, tags: [row.item, row.item] }));
  assert.equal(wanted.length, 5000);
  assert.deepEqual(wanted.at(-1), { hour: 100, tag: 'MOD 4999' });
});

test('F5: sameCountryRequests detects a new chooser or nameHour added under the same occurrence row key', () => {
  const edge = (nameHour: number, tags: string[]) => ({
    option: null,
    lifecycle: null,
    days: null,
    costs: null,
    row: { hour: nameHour, readHour: nameHour, nameHour, kind: 'eventFired', tags, item: null },
  }) as unknown as ScriptOccurrence['edges'][number];
  const before: ScriptOccurrence = { id: 'ev-1', opened: true, edges: [edge(100, ['GER'])] };
  const afterSame: ScriptOccurrence = { id: 'ev-1', opened: true, edges: [edge(100, ['GER'])] };
  const afterExtended: ScriptOccurrence = { id: 'ev-1', opened: false, edges: [edge(100, ['GER']), edge(112, ['FRA'])] };
  const reqBefore = occurrenceCountryRequests([before]);
  const reqSame = occurrenceCountryRequests([afterSame]);
  const reqExtended = occurrenceCountryRequests([afterExtended]);
  assert.equal(sameCountryRequests(reqBefore, reqSame), true);
  assert.equal(sameCountryRequests(reqBefore, reqExtended), false);
  assert.deepEqual(reqExtended, [{ hour: 100, tag: 'GER' }, { hour: 112, tag: 'FRA' }]);
});
