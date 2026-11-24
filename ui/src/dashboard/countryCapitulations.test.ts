import assert from 'node:assert/strict';
import { test } from 'node:test';
import { countryCapitulationRows } from './countryCapitulations.ts';
import { tooltipText } from '../charts/tooltip.ts';

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
  assert.match(tooltipText({ title: '', rows }), /Recorded country at 20 capitulated/);
  assert.doesNotMatch(tooltipText({ title: '', rows }), /refused/, 'a flag failure is not read out');
  const empty = countryCapitulationRows(badges.slice(1), () => () => ({ name: '', known: true, flag: 'native flag' }));
  assert.equal(empty[0].label, '\u00d7  capitulated', 'an authoritative empty name stays empty');
  const pending = countryCapitulationRows(badges.slice(1), () => nativeTag => ({ name: nativeTag, flagPending: true }));
  assert.equal(pending[0].label, `\u00d7 ${tag} capitulated`, 'pending names retain the established native-tag fallback');
  assert.doesNotMatch(tooltipText({ title: '', rows: pending }), /Flag/, 'a loading flag is not read out');
});
