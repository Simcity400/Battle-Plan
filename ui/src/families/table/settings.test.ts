import assert from 'node:assert/strict';
import test from 'node:test';
import {
  displayExtra,
  encodeTableSettings,
  endAligned,
  rankMarkers,
  strengthBar,
  tableDefaults,
  tableSettingsOf,
  updateTableSettings,
  type TableBase,
  type TableColumnDefaults,
} from './settings.ts';

const NUMBERS = ['plain', 'faint', 'track', 'meterAfter', 'meterBefore', 'underline'] as const;
const COUNTRY = ['flagName', 'name', 'flag'] as const;
const FIXED: readonly TableColumnDefaults[] = [
  { key: 'state', label: 'State', as: 'plain', offered: ['plain'] },
  { key: 'owner', label: 'Owner', as: 'flagName', offered: COUNTRY, showNamesDefault: true },
  { key: 'occupier', label: 'Occupier', as: 'flag', offered: COUNTRY, showNamesDefault: false },
  { key: 'resistance', label: 'Resistance', as: 'track', tone: 'red', offered: NUMBERS },
  { key: 'compliance', label: 'Compliance', as: 'track', tone: 'grey', offered: NUMBERS },
  { key: 'law', label: 'Law', as: 'text', offered: ['dot', 'text', 'icon'] },
];
const BASE: TableBase = { density: 'normal', barsBehind: 'all', showRank: false, sort: { column: 'resistance', direction: 'descending' } };
const DEFAULTS = tableDefaults(FIXED, BASE);
const of = (value: unknown) => tableSettingsOf(value, FIXED, BASE);

test('table settings keep density, bars, rank, widths and sort per placement', () => {
  const changed = updateTableSettings(DEFAULTS, next => {
    next.density = 'compact';
    next.barsBehind = 'sorted';
    next.showRank = true;
    next.columns.owner!.show = false;
    next.columns.occupier!.showNames = true;
    next.columns.state!.width = 240;
    next.sort = { column: 'compliance', direction: 'ascending' };
  });
  const restored = of(encodeTableSettings(changed));
  assert.equal(restored.density, 'compact');
  assert.equal(restored.barsBehind, 'sorted');
  assert.equal(restored.showRank, true);
  assert.equal(restored.columns.owner!.show, false);
  assert.equal(restored.columns.occupier!.showNames, true);
  assert.equal(restored.columns.occupier!.as, 'flagName');
  assert.equal(restored.columns.state!.width, 240);
  assert.deepEqual(restored.sort, { column: 'compliance', direction: 'ascending' });
});

test('malformed or v1 table settings use the family defaults', () => {
  assert.deepEqual(of('{"v":1,"overflow":"scroll","columns":{}}'), DEFAULTS);
  assert.deepEqual(of('{"v":2,"density":"huge","overflow":"scroll","columns":{}}'), DEFAULTS);
});

test('partial column orders retain each default column exactly once', () => {
  const restored = of(JSON.stringify({ ...JSON.parse(encodeTableSettings(DEFAULTS)), order: ['owner', 'owner'] }));
  assert.deepEqual(restored.order, ['owner', 'state', 'occupier', 'resistance', 'compliance', 'law']);
});

test('country and law displays survive a round trip, and a display a column does not offer falls back to its default', () => {
  assert.equal(DEFAULTS.columns.owner!.as, 'flagName');
  assert.equal(DEFAULTS.columns.occupier!.as, 'flag');
  const changed = updateTableSettings(DEFAULTS, next => {
    next.columns.owner!.showNames = false;
    next.columns.law!.as = 'icon';
  });
  const restored = of(encodeTableSettings(changed));
  assert.deepEqual([restored.columns.owner!.as, restored.columns.law!.as], ['flag', 'icon']);
  const foreign = updateTableSettings(DEFAULTS, next => {
    next.columns.owner!.as = 'plain';
    next.columns.law!.as = 'meterAfter';
  });
  const fallen = of(encodeTableSettings(foreign));
  assert.deepEqual([fallen.columns.owner!.as, fallen.columns.law!.as], ['flagName', 'text']);
});

test('a column set known only at run time takes each column’s defaults from its spec', () => {
  const measures = (...keys: string[]): TableColumnDefaults[] => [
    { key: 'country', label: 'Country', as: 'flagName', offered: COUNTRY, showNamesDefault: true },
    ...keys.map(key => ({ key, label: key, as: 'plain' as const, offered: NUMBERS })),
  ];
  const base: TableBase = { density: 'normal', barsBehind: 'all', sort: { column: 'country', direction: 'ascending' } };
  const first = tableSettingsOf('', measures('factories', 'stability'), base);
  assert.deepEqual(first.order, ['country', 'factories', 'stability']);
  assert.deepEqual(first.columns.stability, { show: true, as: 'plain', width: null });

  const styled = encodeTableSettings(updateTableSettings(first, next => {
    next.columns.stability = { show: true, as: 'track', tone: 'green', width: 160 };
    next.sort = { column: 'stability', direction: 'descending' };
  }));
  const without = tableSettingsOf(styled, measures('factories', 'casualties'), base);
  assert.deepEqual(without.columns.casualties, { show: true, as: 'plain', width: null });
  assert.deepEqual(without.order, ['country', 'factories', 'stability', 'casualties']);
  assert.deepEqual(without.sort, base.sort);
  assert.deepEqual(without.columns.stability, { show: true, as: 'track', tone: 'green', width: 160 });

  const back = tableSettingsOf(encodeTableSettings(without), measures('stability'), base);
  assert.deepEqual(back.columns.stability, { show: true, as: 'track', tone: 'green', width: 160 });

  const foreign = JSON.stringify({ ...JSON.parse(styled), columns: { ...JSON.parse(styled).columns, stability: { show: true, as: 'dot', width: null }, gone: { show: 'yes' } } });
  assert.equal(tableSettingsOf(foreign, measures('stability'), base).columns.stability!.as, 'plain');
  assert.equal(tableSettingsOf(foreign, measures('factories'), base).columns.gone, undefined);
});

test('displays reserve their room, numeric columns align right, and rank markers hold until expiry', () => {
  assert.deepEqual(['plain', 'faint', 'meterAfter', 'flagName', 'flag', 'name', 'dot'].map(display => displayExtra({ display: display as never })), [0, 8, 64, 28, 20, 0, 12]);
  assert.deepEqual(['plain', 'meterAfter', 'track', 'meterBefore'].map(display => endAligned({ numeric: true, display: display as never })), [true, true, true, true]);
  assert.equal(endAligned({ numeric: false, display: 'plain' }), false);
  assert.equal(endAligned({ numeric: true, display: 'plain', cellKind: 'icons' }), false, 'an icon strip starts at the left, and so does its header');
  assert.equal(strengthBar('87544'), 88000n);
  const first = rankMarkers(['GER', 'ITA', 'SOV'], ['SOV', 'GER', 'ITA', 'ENG'], 1000, new Map(), 2500);
  assert.deepEqual(Object.fromEntries(first), { SOV: { delta: 2, until: 3500 }, GER: { delta: -1, until: 3500 }, ITA: { delta: -1, until: 3500 } });
  const later = rankMarkers(['SOV', 'GER', 'ITA', 'ENG'], ['SOV', 'GER', 'ENG', 'ITA'], 2000, first, 2500);
  assert.deepEqual(Object.fromEntries(later), { SOV: { delta: 2, until: 3500 }, GER: { delta: -1, until: 3500 }, ENG: { delta: 1, until: 4500 }, ITA: { delta: -1, until: 4500 } });
  assert.deepEqual(Object.fromEntries(rankMarkers(['SOV', 'GER', 'ENG', 'ITA'], ['SOV', 'GER', 'ENG', 'ITA'], 4000, later, 2500)), { ENG: { delta: 1, until: 4500 }, ITA: { delta: -1, until: 4500 } });
});
