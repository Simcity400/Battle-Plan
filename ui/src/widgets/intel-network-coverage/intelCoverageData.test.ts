import test from 'node:test';
import assert from 'node:assert/strict';
import { cellState, countriesAt, fieldText, measureField, pairsAt, pairTooltip, validateIntelCoverage, visibleCountries } from './intelCoverageData.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';

const reply = validateIntelCoverage({
  window: { firstHour: 0, lastHour: 48, known: [{ firstHour: 0, lastHour: 30 }], reads: [[0, 23, 0], [24, 30, 24]], series: [
    { name: 'GER@7|FRA@9|coveredControlled', runs: [[0, 40, '3', true]] },
    { name: 'GER@7|FRA@9|controlledStrength', runs: [[0, 23, '150000', true], [24, 30, '250000', false]] },
    { name: 'FRA@9|GER@7|coveredControlled', runs: [[24, 30, '0', true]] },
    { name: 'member|GER@7', runs: [[0, 48, '1', true]] },
    { name: 'member|FRA@9', runs: [[0, 48, '1', true]] },
    { name: 'member|ITA@8', runs: [[20, 48, '1', true]] },
    { name: 'rank|GER@7', runs: [[0, 23, '1', true], [24, 30, '2', false]] },
    { name: 'rank|FRA@9', runs: [[0, 23, '1', true], [24, 30, '2', false]] },
    { name: 'rank|ITA@8', runs: [[20, 23, '3', true], [24, 30, '0', false]] },
  ] },
  warnings: [],
});

test('directions are independent; an empty network is zero and no summary is no network', () => {
  const early = pairsAt(reply, 10);
  assert.equal(early.pairs.get('GER@7|FRA@9')?.get('coveredControlled'), 3n);
  assert.equal(cellState(early, 'FRA@9', 'GER@7').kind, 'absent');
  const late = pairsAt(reply, 25);
  assert.equal(late.pairs.get('FRA@9|GER@7')?.get('coveredControlled'), 0n);
  assert.equal(late.readHour, 24);
});

test('a present summary reads on its own; absence needs a proving hour and both countries present', () => {
  // Hour 40 proves no absence, yet the recorded network is still shown.
  const unproved = pairsAt(reply, 40);
  assert.equal(unproved.known, false);
  assert.equal(cellState(unproved, 'GER@7', 'FRA@9').kind, 'present');
  assert.equal(cellState(unproved, 'FRA@9', 'GER@7').kind, 'unknown');
  // At hour 10 Italy is not present, so a missing pair with it is unknown, not "no network".
  const early = pairsAt(reply, 10);
  assert.equal(cellState(early, 'GER@7', 'ITA@8').kind, 'unknown');
  assert.equal(cellState(pairsAt(reply, 25), 'GER@7', 'ITA@8').kind, 'absent');
});

test('the most active countries are chosen at the playback hour, not the branch end', () => {
  assert.deepEqual(countriesAt(reply, 21, 2)!.map(country => country.id), ['ITA@8', 'FRA@9']);
  assert.deepEqual(countriesAt(reply, 25, 2)!.map(country => country.id), ['FRA@9', 'GER@7']);
  // Before Italy exists it is never shown, whatever its later rank.
  assert.deepEqual(countriesAt(reply, 10, 3)!.map(country => country.id), ['FRA@9', 'GER@7']);
  // Without a recorded ranking the hour shows nothing rather than a borrowed one.
  assert.equal(countriesAt(reply, 40, 3), null);
  // Explicit tags choose the countries present then, by tag or exact TAG@slot.
  assert.deepEqual(countriesAt(reply, 40, 3, ['GER', 'ITA@8'])!.map(country => country.id), ['GER@7', 'ITA@8']);
  assert.deepEqual(countriesAt(reply, 10, 3, ['ITA'])!, []);
});

test('measures map onto the recorded fields on each basis', () => {
  assert.equal(measureField('covered', 'controlled'), 'coveredControlled');
  assert.equal(measureField('covered', 'core'), 'coveredCore');
  assert.equal(measureField('strength', 'core'), 'coreStrength');
  assert.equal(measureField('strongest', 'core'), 'maxStrength');
  assert.equal(fieldText('controlledStrength', 250000n), '2.5');
  assert.equal(fieldText('coveredControlled', 1200n), '1,200');
});

test('tooltips keep unknown, no network and recorded values apart', () => {
  assert.equal(pairTooltip('A', 'B', { kind: 'unknown' }, null).rows[0].value, 'Not recorded');
  assert.equal(pairTooltip('A', 'B', { kind: 'absent' }, null).rows[0].value, 'No network');
  const late = pairsAt(reply, 25);
  assert.match(pairTooltip('Germany', 'France', cellState(late, 'GER@7', 'FRA@9'), '1 Jan').rows.map(row => row.value).join(','), /3,Not recorded,2.5/);
});

test('labels load only for the axes the matrix draws', () => {
  const countries = countriesAt(reply, 25, 3)!;
  assert.deepEqual(visibleCountries(countries, new Set(['GER@7'])).map(country => country.id), ['GER@7']);
});

test('a member or rank series must name a country', () => {
  assert.throws(() => validateIntelCoverage({ window: { firstHour: 0, lastHour: 1, known: [], reads: [], series: [{ name: 'member|bad', runs: [] }] }, warnings: [] }),
    /malformed/u);
});

test('the matrix asks for the chosen group\'s members, and a Top countries group sets how many are shown', async () => {
  const { intelCoverageRequest, intelShown } = await import('../../dashboard/widgetRequests.ts');
  const track = { id: 'main', source: { id: 's', revision: '1' }, branch: { id: '1', firstHour: 0, lastHour: 48 } } as unknown as BoundTrack;
  const widget = (countries: unknown) => ({ id: 'intel', kind: 'intel-network-coverage', trackIds: ['main'], countries, time: { mode: 'follow' },
    options: { mostCountries: 5 } }) as unknown as WidgetSpec;
  assert.equal(intelShown(widget({ kind: 'group', group: { type: 'top', count: 30 }, membership: 'hour', aggregate: false })), 30);
  assert.equal(intelShown(widget({ kind: 'all' })), 5);
  const humans = widget({ kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false });
  assert.deepEqual(intelCoverageRequest(humans, track, ['GER', 'ITA'])!.params, { branchId: '1', view: 'matrix', shown: 5, tags: ['GER', 'ITA'] });
  assert.deepEqual(intelCoverageRequest(widget({ kind: 'all' }), track)!.params, { branchId: '1', view: 'matrix', shown: 5 });
});
