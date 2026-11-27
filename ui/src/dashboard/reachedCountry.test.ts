import { datedToken, gatedHour, openGate } from '../test-support/gated.ts';
import assert from 'node:assert/strict';
import test from 'node:test';
import type { CountryIdentityHistory } from '../generated/dashboard';
import { datedCountrySpans, readDatedCountry } from './datedCountry.ts';
import { countryDrillTagAt } from './countryIdentity.ts';
import { choiceAt, resolveChoice, settleChoice } from './reachedCountry.ts';
import { buildStripKeyframe } from '../families/broker.ts';
import { stripContextAt } from '../families/host/planSources.ts';
import type { FamilyPlanContext } from '../families/plan.ts';
import { airMissionsDeclaration } from '../widgets/air-missions/airMissionsProjection.ts';

const histories: CountryIdentityHistory[] = [
  { tag: 'GER', identities: [{ slot: 7, countryId: 1, firstHour: 0, lastHour: 99 }, { slot: 9, countryId: 2, firstHour: 200, lastHour: 300 }] },
  { tag: 'DEU', identities: [{ slot: 7, countryId: 1, firstHour: 100, lastHour: 150 }] },
];
const at = (tag: string, chosenAt: number, hour: number, from = histories, complete = true) =>
  resolveChoice(from, complete, choiceAt('c', '1', 0, tag, chosenAt), 0, 300, hour);

test('a choice follows the identity holding its tag at the hour it was made, through its renames only', () => {
  assert.equal(at('GER', 50, 50).country.tag, 'GER');
  assert.equal(at('GER', 50, 120).country.tag, 'DEU', 'renamed, not absent');
  assert.deepEqual(at('GER', 50, 250).country, { chosen: 'GER', tag: null, absent: true, reason: null },
    'after it ends, a later holder of the tag never takes the choice');
  assert.equal(at('GER', 250, 50).country.absent, true, 'the later holder chosen later is not the earlier country');
  assert.equal(at('GER', 250, 250).country.tag, 'GER');
  const branch = { id: '1', firstHour: 0, lastHour: 300 };
  const initial = { countryTag: 'GER', countryName: 'Germany', branch, track: { source: { id: 's', revision: 'r' }, axisOffset: 0 }, planHour: 50 } as FamilyPlanContext;
  const spans = datedCountrySpans(histories, choiceAt('c', '1', 0, 'GER', 50), 0, 300)!;
  const hour = gatedHour(119);
  const viewport = { following: true, staticSpan: null };
  const initialPlan = airMissionsDeclaration.sources(initial, viewport);
  const ctx = stripContextAt(initial, initialPlan.valid, hour, countryDrillTagAt(spans, hour), 'Germany');
  const options = { following: true, timeMode: 'follow' as const, gate: openGate(0, 300), window: null, staticSpan: null, branch };
  const strip = buildStripKeyframe(airMissionsDeclaration.sources(ctx, viewport), hour, options);
  assert.equal(strip.asks[0]?.request?.params.country, 'DEU');
  for (const hour of [-1, 301]) assert.deepEqual(at('GER', 250, hour).country,
    { chosen: 'GER', tag: null, absent: false, reason: 'This time is outside the recording.' },
    `at ${hour}, outside the branch, the country is not called absent: the time is outside the recording`);
});

test('the binding is the stored dated choice, so it survives reloads', () => {
  const selection = choiceAt('c', '1', 0, 'GER', 50);
  assert.deepEqual(readDatedCountry(datedToken(selection), 'c', '1', { kind: 'tags', tags: ['GER'] }), selection);
});

test('a tag naming no country at the choice hour rebinds to its only identity, else asks which', () => {
  assert.deepEqual(at('DEU', 10, 120).rebind, choiceAt('c', '1', 0, 'DEU', 100), 'DEU named one identity only');
  assert.deepEqual(at('GER', 170, 170).holders, [{ origin: 0, firstHour: 0, lastHour: 99 }, { origin: 200, firstHour: 200, lastHour: 300 }]);
  assert.equal(at('FRA', 50, 50).country.reason, 'FRA is not in this campaign.');
  assert.deepEqual(settleChoice(histories, choiceAt('c', '1', 0, 'GER', 50), 0), { kind: 'held' });
});

test('a provisional history keeps the choice hour and never rebinds it', () => {
  const sampled: CountryIdentityHistory[] = [{ tag: 'GER', identities: [{ slot: 9, countryId: 2, firstHour: 720, lastHour: 800 }] }];
  const loading = resolveChoice(sampled, false, choiceAt('c', '1', 0, 'GER', 100), 0, 800, 100);
  assert.deepEqual(loading, { country: { chosen: 'GER', tag: null, absent: false, reason: null }, holders: null, rebind: null },
    'still loading, and the later holder is not bound');
  const finished: CountryIdentityHistory[] = [{ tag: 'GER', identities: [{ slot: 7, countryId: 1, firstHour: 50, lastHour: 200 },
    { slot: 9, countryId: 2, firstHour: 720, lastHour: 800 }] }];
  const done = resolveChoice(finished, true, choiceAt('c', '1', 0, 'GER', 100), 0, 800, 100);
  assert.equal(done.country.tag, 'GER');
  assert.equal(done.rebind, null, 'the choice hour named its holder once the history was complete');
  assert.equal(resolveChoice(finished, true, choiceAt('c', '1', 0, 'GER', 100), 0, 800, 750).country.absent, true);
});
