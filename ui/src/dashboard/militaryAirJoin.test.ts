import assert from 'node:assert/strict';
import test from 'node:test';
import { LedgerJoin, ledgerWindows } from './militaryAirJoin.ts';
import { ledgerCauseName, validateMilitaryAirWindow } from './militaryAirLedger.ts';
import { WINDOW_HOURS } from './heldWindows.ts';
import type { MilitaryAirPart, MilitaryAirReply } from '../generated/dashboard';

const joinMilitaryAir = (windows: readonly MilitaryAirReply[]) => {
  if (!windows.length) return null;
  const join = new LedgerJoin('test', windows[0]!.window);
  for (const item of windows) join.append(item);
  return join.ledger();
};

const span = (firstHour: number, lastHour = firstHour) => ({ firstHour, lastHour });
const part = (tag: string | null, dealt: boolean, points: [number, number, string][], cause = 1): MilitaryAirPart => ({ tag, identity: tag && `${tag.length}:${tag}`,
  dealt, cause, opponentKnown: true, buildingToken: null, buildingId: null, state: null, points });
const window = (index: number, firstHour: number, lastHour: number, extra: Partial<MilitaryAirReply> = {}): MilitaryAirReply => validateMilitaryAirWindow({
  family: 'airLosses', window: index, firstHour, lastHour, countedFrom: 10, available: true, observed: [span(firstHour, lastHour)], incomplete: [],
  countries: ['GER', 'ITA'].map(tag => ({ tag, identity: `${tag.length}:${tag}`, present: [span(firstHour, lastHour)], membership: null })), openings: [],
  parts: [], regions: [], buildings: [], notes: [], warnings: [], ...extra });

test('windows join end to end: each part carries its total on, spans and holders are laid together, names and notes once each', () => {
  const one = window(0, 10, 19, {
    parts: [part('ITA', true, [[11, 11, '5']]), part('GER', false, [[12, 12, '999999999999999'], [9, 14, '1000000000000000']])],
    regions: [{ id: '10', name: 'Ten', missing: null }, { id: '9', name: 'Nine', missing: null }],
    causes: [{ cause: 1, name: 'Aircraft combat', missing: null }],
    notes: [{ reason: 'b', events: 1 }, { reason: 'a', events: 2 }], warnings: ['content'] });
  const two = window(1, 20, 29, {
    observed: [span(20, 24)], incomplete: [span(21)], countries: ['GER', 'ITA'].map(tag => ({ tag, identity: `${tag.length}:${tag}`, present: [span(20, 24)], membership: null })),
    parts: [part('GER', false, [[18, 21, '7']]), part(null, true, [[22, 22, '-3']]), part('ITA', true, [])],
    regions: [{ id: '9', name: 'Nine', missing: null }, { id: '2', name: null, missing: 'not named' }],
    notes: [{ reason: 'a', events: 3 }], warnings: ['content', 'other'] });
  const joined = joinMilitaryAir([one, two])!;
  assert.deepEqual(joined.observed, [span(10, 24)]);
  assert.deepEqual(joined.incomplete, [span(21)]);
  assert.deepEqual([joined.firstHour, joined.lastHour, joined.countedFrom], [10, 29, 10]);
  assert.deepEqual(joined.countries.map(country => [country.tag, country.present, country.membership]), [['GER', [span(10, 24)], null], ['ITA', [span(10, 24)], null]]);
  assert.deepEqual(joined.parts.map(item => [item.tag, item.points]), [
    [null, [[22, 22, '-3']]],
    ['GER', [[12, 12, '999999999999999'], [9, 14, '1000000000000000'], [18, 21, '1000000000000007']]],
    ['ITA', [[11, 11, '5']]],
  ], 'the unattributed first, then by holder; a total past a safe number stays exact');
  assert.deepEqual(one.parts.find(item => item.tag === 'GER')!.points.length, 2, 'the window as it landed is left as it was');
  assert.deepEqual(joined.regions.map(name => name.id), ['2', '9', '10'], 'regions by number');
  assert.equal(ledgerCauseName(joined, 1), 'Aircraft combat');
  assert.equal(ledgerCauseName(joined, 9), 'Cause 9');
  assert.deepEqual(joined.warnings, ['5 events: a', '1 events: b', 'content', 'other']);
  assert.equal(joinMilitaryAir([]), null);
});

test('the service\'s top N memberships of each window are laid end to end; a holder ranked in no window has none, never null', () => {
  const ranked = (gerSpans: { firstHour: number; lastHour: number }[], itaSpans: { firstHour: number; lastHour: number }[], firstHour: number, lastHour: number) =>
    [['GER', gerSpans], ['ITA', itaSpans]].map(([tag, membership]) => ({ tag: tag as string, identity: `3:${tag}`, present: [span(firstHour, lastHour)], membership: membership as { firstHour: number; lastHour: number }[] }));
  const joined = joinMilitaryAir([window(0, 10, 19, { countries: ranked([span(12, 19)], [], 10, 19) }), window(1, 20, 29, { countries: ranked([span(20, 24)], [span(25, 29)], 20, 29) })])!;
  assert.deepEqual(joined.countries.map(country => [country.tag, country.membership]), [['GER', [span(12, 24)]], ['ITA', [span(25, 29)]]]);
  const never = joinMilitaryAir([window(0, 10, 19, { countries: ranked([span(12, 19)], [], 10, 19) })])!;
  assert.deepEqual(never.countries.find(country => country.tag === 'ITA')!.membership, []);
});

test('a window whose observed hours reach back to the first counted validates and joins over the window before it', () => {
  const reaching = (index: number, firstHour: number, lastHour: number, since: number) => window(index, firstHour, lastHour, {
    observed: [span(since, lastHour)], countries: ['GER', 'ITA'].map(tag => ({ tag, identity: `${tag.length}:${tag}`, present: [span(since, lastHour)], membership: null })),
    openings: [{ identity: '3:GER', dealt: false, cause: 1, opponentKnown: true, total: '30' }] });
  assert.throws(() => reaching(3, 40, 49, 9), /Invalid/, 'never before the first counted');
  assert.throws(() => window(3, 40, 49, { observed: [span(20, 30), span(40, 49)] }), /Invalid/, 'only the hours running on into the window reach back');
  const joined = joinMilitaryAir([window(1, 20, 29), reaching(2, 30, 39, 10)])!;
  assert.deepEqual(joined.observed, [span(10, 39)], 'a reach over the window before joins it');
  assert.deepEqual(joined.countries.map(country => country.present), [[span(10, 39)], [span(10, 39)]]);
});

test('a race asks the window it shows and, following, the next ahead, never past its span; a view of the span asks it whole', () => {
  const at = (hour: number) => Math.floor(hour / WINDOW_HOURS);
  assert.deepEqual(ledgerWindows(100, 40_000, 9_000, 'shown', false), [3]);
  assert.deepEqual(ledgerWindows(100, 40_000, 9_000, 'shown', true), [3, 4]);
  assert.deepEqual(ledgerWindows(100, 9_100, 9_000, 'shown', true), [3], 'nothing past the span');
  assert.deepEqual(ledgerWindows(6_000, 40_000, 100, 'shown', true), [2, 3], 'never before the span');
  assert.deepEqual(ledgerWindows(6_000, 40_000, 9_000, 'end', true), Array.from({ length: at(40_000) - at(6_000) + 1 }, (_, index) => at(6_000) + index));
});
