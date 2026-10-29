import test from 'node:test';
import assert from 'node:assert/strict';
import { aliasTagsAt, cellState, countriesAt, fieldText, measureField, pairsAt, pairTooltip, validateIntelCoverage, visibleCountries } from './intelCoverageData.ts';
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

test('one chosen country is shown first with every country it has a network in or that has one in it then', () => {
  const runs = (first: number, last: number) => [[first, last, '1', true]];
  const pair = (owner: string, target: string, first: number) => ['coveredControlled', 'controlledStrength'].map(field => ({ name: `${owner}|${target}|${field}`, runs: runs(first, 40) }));
  const touching = validateIntelCoverage({
    window: { firstHour: 0, lastHour: 40, known: [{ firstHour: 0, lastHour: 40 }], reads: [[0, 40, 0]], series: [
      ...pair('GER@7', 'FRA@9', 0), ...pair('ITA@8', 'GER@7', 0), ...pair('FRA@9', 'ITA@8', 0), ...pair('ENG@4', 'SOV@5', 0), ...pair('GER@7', 'ENG@4', 30),
      { name: 'member|GER@7', runs: runs(0, 35) }, ...['FRA@9', 'ITA@8', 'ENG@4', 'SOV@5'].map(id => ({ name: `member|${id}`, runs: runs(0, 40) })),
    ] },
    warnings: [],
  });
  const ids = (hour: number, linked = true) => countriesAt(touching, hour, 2, ['GER'], linked)!.map(country => country.id);
  assert.deepEqual(ids(10), ['GER@7', 'FRA@9', 'ITA@8'], 'its own networks and those in it, whatever Countries shown says; never SOV');
  assert.deepEqual(ids(30), ['GER@7', 'ENG@4', 'FRA@9', 'ITA@8'], 'a network begun later counts from its hour');
  assert.deepEqual(ids(10, false), ['GER@7'], 'chosen countries alone otherwise');
  assert.deepEqual(ids(38), [], 'without the chosen country there, nothing is its network');
  // FRA and ITA's networks in each other are shown with GER's.
  assert.equal(cellState(pairsAt(touching, 10), 'FRA@9', 'ITA@8').kind, 'present');
});

test('Top 1 shows the most active country then with every country it has a network in or that has one in it', () => {
  // At hour 21 Italy ranks first and has no network; at 25 France does (ties by identity), with Germany's both ways.
  assert.deepEqual(countriesAt(reply, 21, 1, undefined, true)!.map(country => country.id), ['ITA@8']);
  assert.deepEqual(countriesAt(reply, 25, 1, undefined, true)!.map(country => country.id), ['FRA@9', 'GER@7']);
  assert.deepEqual(countriesAt(reply, 25, 1)!.map(country => country.id), ['FRA@9'], 'one country, never padded to two');
  assert.equal(countriesAt(reply, 40, 1, undefined, true), null, 'without a recorded ranking, no top country');
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
  // The dashboard's country, or one picked, asks for its networks with every country.
  for (const one of [widget({ kind: 'selected' }), widget({ kind: 'tags', tags: ['GER'] })]) {
    assert.deepEqual(intelCoverageRequest(one, track, ['GER'])!.params, { branchId: '1', view: 'matrix', shown: 5, tags: ['GER'], touching: true });
  }
  assert.equal(intelCoverageRequest(widget({ kind: 'tags', tags: ['GER', 'ITA'] }), track)!.params.touching, undefined);
  assert.equal(intelCoverageRequest(humans, track, ['GER'])!.params.touching, undefined, 'a group of one stays its members');
  // Top 1 asks for exactly one, read as one country; Top 2 stays the most active among themselves.
  const top = (count: number) => widget({ kind: 'group', group: { type: 'top', count }, membership: 'hour', aggregate: false });
  assert.equal(intelShown(top(1)), 1);
  assert.deepEqual(intelCoverageRequest(top(1), track, undefined)!.params, { branchId: '1', view: 'matrix', shown: 1, touching: true });
  assert.deepEqual(intelCoverageRequest(top(2), track, undefined)!.params, { branchId: '1', view: 'matrix', shown: 2 });
});

test('a chosen country renamed by the hour shown is found under the tag it holds then', () => {
  const renamed = validateIntelCoverage({
    window: { firstHour: 0, lastHour: 48, known: [{ firstHour: 0, lastHour: 48 }], reads: [[0, 48, 0]], series: [
      { name: 'member|OLD@7', runs: [[0, 23, '1', true]] },
      { name: 'member|NEW@7', runs: [[24, 48, '1', true]] },
      { name: 'member|FRA@9', runs: [[0, 48, '1', true]] },
    ] },
    warnings: [],
  });
  // One native holder chose by its first tag: OLD until 23, NEW from 24.
  const aliases = [{ tag: 'OLD', firstHour: 0, lastHour: 23 }, { tag: 'NEW', firstHour: 24, lastHour: 48 }];
  assert.deepEqual(aliasTagsAt(aliases, 30), ['NEW']);
  assert.deepEqual(countriesAt(renamed, 30, 10, aliasTagsAt(aliases, 30), true)!.map(country => country.id), ['NEW@7']);
  assert.deepEqual(countriesAt(renamed, 10, 10, aliasTagsAt(aliases, 10))!.map(country => country.id), ['OLD@7']);
  assert.deepEqual(countriesAt(renamed, 30, 10, ['OLD']), [], 'the literal first tag alone loses the renamed holder');
});
