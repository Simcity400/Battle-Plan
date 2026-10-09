import assert from 'node:assert/strict';
import { test } from 'node:test';
import { countryCapitulationBadges, countryCapitulationRows } from './countryCapitulations.ts';
import { visibleBadges } from '../charts/badges.ts';
import { tooltipText } from '../charts/tooltip.ts';

const span = (firstHour: number, lastHour: number) => ({ firstHour, lastHour });

test('capitulations use the drawn native holder, exact event coverage and dated membership', () => {
  // The selected identity renamed old -> new. A later reuse of old is excluded.
  const countries = [
    { tag: 'old', identities: [span(10, 12)], segments: [span(10, 20)], membership: null },
    { tag: 'new', identities: [span(13, 20)], segments: [span(13, 20)], membership: [span(15, 20)] },
    { tag: 'empty', identities: [], segments: [span(10, 20)] },
    { tag: 'legacy', segments: [span(16, 18)] },
    { tag: 'former', segments: [span(10, 20)], membership: [] },
  ];
  const events = [['old', 9], ['old', 11], ['old', 17], ['new', 14], ['new', 15], ['new', 21],
    ['empty', 17], ['legacy', 15], ['legacy', 17], ['former', 17], ['unselected', 17]] as const;
  const groups = { capitulations: events.map(([tag, hour]) => ({ tag, hour })) };
  const before = JSON.stringify(countries);
  const badges = countryCapitulationBadges(groups, countries, [10, 20]);
  assert.deepEqual([...badges], [
    ['old', [{ hour: 11, glyph: '\u00d7', label: 'old capitulated', capitulationTag: 'old' }]],
    ['new', [{ hour: 15, glyph: '\u00d7', label: 'new capitulated', capitulationTag: 'new' }]],
    ['legacy', [{ hour: 17, glyph: '\u00d7', label: 'legacy capitulated', capitulationTag: 'legacy' }]],
  ]);
  assert.equal(JSON.stringify(countries), before, 'markers never split or alter the numeric coverage');
  assert.deepEqual(visibleBadges([...badges.values()].flat(), [10, 20], 14).map(badge => badge.hour), [11]);
  assert.deepEqual([...countryCapitulationBadges(undefined, countries, [10, 20])], []);
});

test('all 5,000 selected native countries retain their dated capitulation', () => {
  const countries = Array.from({ length: 5000 }, (_, index) => ({ tag: `opaque actor ${index}`, identities: [span(0, 100)], segments: [] }));
  const groups = { capitulations: countries.map(country => ({ tag: country.tag, hour: 50 })) };
  const badges = countryCapitulationBadges(groups, countries, [0, 100]);
  assert.equal(badges.size, 5000);
  assert.equal([...badges.values()].flat().length, 5000);
  assert.equal(badges.get('opaque actor 4999')?.[0].hour, 50);
});

test('reached capitulations use exact dated names and native flag status without parsing opaque tags', () => {
  const tag = 'capitulated at 20 | opaque actor';
  const generic = { hour: 19, glyph: '*', label: 'Generic event' };
  const badges = [generic, { hour: 20, glyph: '\u00d7', label: `${tag} capitulated`, capitulationTag: tag }];
  const calls: [number, string][] = [];
  const rows = countryCapitulationRows(badges, hour => nativeTag => {
    calls.push([hour, nativeTag]);
    return { name: 'Recorded country at ' + hour, flagReason: 'Exact event-hour flag refused', flagPending: false };
  });
  assert.deepEqual(calls, [[20, tag]], 'generic badges never request a country');
  assert.equal(rows[0].label, '* Generic event');
  assert.equal(rows[0].countryFlag, undefined);
  assert.equal(rows[1].label, '\u00d7 Recorded country at 20 capitulated');
  assert.equal(rows[1].countryFlag?.flagReason, 'Exact event-hour flag refused');
  assert.match(tooltipText({ title: '', rows }), /Recorded country at 20 capitulated.*Exact event-hour flag refused/);
  const empty = countryCapitulationRows(badges.slice(1), () => () => ({ name: '', known: true, flag: 'native flag' }));
  assert.equal(empty[0].label, '\u00d7  capitulated', 'an authoritative empty name stays empty');
  const pending = countryCapitulationRows(badges.slice(1), () => nativeTag => ({ name: nativeTag, flagPending: true }));
  assert.equal(pending[0].label, `\u00d7 ${tag} capitulated`, 'pending names retain the established native-tag fallback');
  assert.match(tooltipText({ title: '', rows: pending }), /Flag.*loading/i);
});
