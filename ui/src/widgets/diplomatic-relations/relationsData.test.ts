import assert from 'node:assert/strict';
import test from 'node:test';
import type { CountryGroupReply, DiplomaticRelationsReply, ScoreboardReply } from '../../generated/dashboard';
import { opinionDomain, opinionIndex, opinionText, orderCountries, pairHolders, pairKeys, pairOf, pairToken, readHourOf, relationsStructure,
  replyTopTags, validateRelations } from './relationsData.ts';
import { liveKey, valuesOf } from '../../charts/live.ts';
import { NO_ROW, NOT_READ } from '../../dashboard/follow/liveCells.ts';

const reply: DiplomaticRelationsReply = { warnings: [], cells: [], domain: [0, 0], owners: [
  { tag: 'A', countryId: 1, known: true, reason: null },
  { tag: 'B', countryId: 2, known: false, reason: 'Not read' },
  { tag: 'C', countryId: 3, known: true, reason: null },
  { tag: 'D', countryId: null, known: false, reason: 'No country holds this tag at this hour' },
] };
const cells = (opinions: Record<string, number>, reasons: Record<string, string> = {}) => valuesOf(new Map(Object.entries({ ...opinions,
  [liveKey('A', 'readHour')]: 8, [liveKey('C', 'readHour')]: 10 })), key => reasons[key] ?? NO_ROW);
const values = cells({ [liveKey('A', 'B')]: 40, [liveKey('A', 'A')]: 5, [liveKey('C', 'A')]: -120 });

test('a known owner’s pair with no row is exact zero, an unknown owner’s row is unknown, and self-pairs are kept', () => {
  const at = opinionIndex(reply, values);
  assert.deepEqual(at('A', 'B'), { value: 40 });
  assert.deepEqual(at('A', 'C'), { value: 0 }, 'no row in a read owner\'s opinions is zero');
  assert.deepEqual(at('B', 'A'), { unknown: true, reason: 'Not read' }, 'an owner without proof is unknown, never zero');
  assert.deepEqual(at('A', 'A'), { value: 5 }, 'the diagonal is recorded state');
  assert.equal(at('Z', 'A'), null);
  assert.equal(at('A', 'Z'), null, 'a country the reply was not asked about is not zero');
  assert.deepEqual(opinionDomain(reply, values), { low: -120, high: 40, settled: true }, 'the colour domain is the recorded extremes, not a fixed range');
  assert.equal(readHourOf(values, 'C'), 10);
  assert.equal(readHourOf(values, 'B'), null);
  assert.equal(opinionText(40), '+40'); assert.equal(opinionText(-3), '-3'); assert.equal(opinionText(0), '0');
});

test('a panel landing before its cells takes its colour domain once they answer, then holds it while they move', () => {
  const unread = valuesOf(new Map(), NOT_READ);
  const first = opinionDomain(reply, unread);
  assert.deepEqual(first, { low: 0, high: 0, settled: false }, 'no cell has answered: nothing to colour by yet');
  assert.equal(opinionDomain(reply, unread, first), first, 'nothing new: the same domain, no render');
  const answered = opinionDomain(reply, values, first);
  assert.deepEqual(answered, { low: -120, high: 40, settled: true }, 'the cells answered: the domain is theirs, never a grey 0 to 0');
  const moved = cells({ [liveKey('A', 'B')]: 90, [liveKey('C', 'A')]: -10 });
  assert.equal(opinionDomain(reply, moved, answered), answered, 'held while the values move, so the colours do not move each frame');
  assert.deepEqual(opinionDomain(reply, cells({}), null), { low: 0, high: 0, settled: true }, 'every pair read with no row: zero is recorded, and held');
});

test('an opinion the hour did not record is unknown with its reason, never zero; values moving never change the structure', () => {
  const unread = cells({}, { [liveKey('A', 'B')]: 'Not recorded at this hour' });
  assert.deepEqual(opinionIndex(reply, unread)('A', 'B'), { unknown: true, reason: 'Not recorded at this hour' });
  assert.equal(relationsStructure(reply), relationsStructure(validateRelations(JSON.parse(JSON.stringify(reply)))));
  assert.notEqual(relationsStructure(reply), relationsStructure({ ...reply, owners: reply.owners.map(owner => ({ ...owner, known: true })) }));
});

test('a reply of another shape is refused, never read wrongly', () => {
  assert.equal(validateRelations(reply), reply);
  assert.throws(() => validateRelations({ ...reply, owners: [{ ...reply.owners[0], known: 'yes' }] }), /malformed/);
  assert.throws(() => validateRelations({ owners: [] }), /malformed/);
  const at = opinionIndex(reply, values);
  assert.deepEqual(at('C', 'B'), { value: 0 });
  assert.deepEqual(at('D', 'C'), { unknown: true, reason: 'No country holds this tag at this hour' }, 'an unheld tag is unknown, never zero');
});

test('rows order by faction then name, countries outside any faction last; or by name', () => {
  const groups = { factions: [
    { id: '2', members: [{ tag: 'C', firstHour: 0, lastHour: 20 }, { tag: 'A', firstHour: 0, lastHour: 20 }] },
    { id: '1', members: [{ tag: 'D', firstHour: 0, lastHour: 20 }] },
  ] } as unknown as CountryGroupReply;
  const names: Record<string, string> = { A: 'Zeta', B: 'Alpha', C: 'Beta', D: 'Gamma' };
  const factions: Record<string, string> = { '1': 'A mod faction', '2': 'Second' };
  assert.deepEqual(orderCountries(['A', 'B', 'C', 'D'], 'faction', tag => names[tag], groups, 10, id => factions[id]), ['D', 'C', 'A', 'B']);
  assert.deepEqual(orderCountries(['A', 'B', 'C', 'D'], 'name', tag => names[tag], groups, 10, id => factions[id]), ['B', 'C', 'D', 'A']);
  assert.deepEqual(orderCountries(['A', 'C'], 'faction', tag => names[tag], groups, 30, id => factions[id]), ['C', 'A'], 'after the rosters end, by name');
});

test('a saved pair round-trips with its hour and the raw ids its lines name', () => {
  const pair = { first: 'A', second: 'mod:Ω', hour: 120, firstId: 7, secondId: 9 };
  assert.deepEqual(pairOf(pairToken(pair)), pair, 'the pin keeps the hour that names its holders');
  assert.deepEqual(pairKeys(pair), [{ tag: 'A', part: '9' }, { tag: 'mod:Ω', part: '7' }]);
  assert.equal(pairOf(JSON.stringify(['A', 'B', 120])), null, 'a pair without its ids cannot name its lines');
  assert.equal(pairOf('not json'), null); assert.equal(pairOf(null), null);
  assert.deepEqual(pairHolders(pair).map(holder => [holder.key, holder.hour]), [['first', 120], ['second', 120]],
    'both holders are read at the chosen hour itself, never a sample before a reuse of the tag');
});

test('a top N is the Scoreboard’s ranking at the hour, greatest first, of countries ranked by a recorded value', () => {
  const cell = (valueRaw: string | null) => ({ valueRaw, missing: valueRaw === null ? 'Not recorded' : null, partial: false });
  const ranking: ScoreboardReply = { branchId: 'b', hour: 0, missing: null, warnings: [],
    rows: [{ country: 'SOV', tag: 'SOV', cells: [cell('152')] }, { country: 'ENG', tag: 'ENG', cells: [cell('94')] }, { country: 'XXX', tag: 'XXX', cells: [cell(null)] }] };
  assert.deepEqual(replyTopTags(ranking), ['SOV', 'ENG']);
  assert.deepEqual(replyTopTags({ ...ranking, rows: [], missing: 'Not recorded at this hour' }), []);
});
