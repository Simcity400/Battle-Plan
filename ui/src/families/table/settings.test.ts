import assert from 'node:assert/strict';
import test from 'node:test';
import {
  amountCell,
  displayExtra,
  encodeTableSettings,
  endAligned,
  percentCell,
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
  { key: 'owner', label: 'Owner', as: 'flagName', offered: COUNTRY },
  { key: 'occupier', label: 'Occupier', as: 'flagName', offered: COUNTRY },
  { key: 'resistance', label: 'Resistance', as: 'meterAfter', tone: 'red', offered: NUMBERS },
  { key: 'compliance', label: 'Compliance', as: 'meterAfter', tone: 'blue', offered: NUMBERS },
  { key: 'law', label: 'Law', as: 'dot', offered: ['dot', 'outline', 'filled', 'text', 'icon'] },
];
const BASE: TableBase = { density: 'comfortable', overflow: 'tooltip', sort: { column: 'resistance', direction: 'descending' } };
const DEFAULTS = tableDefaults(FIXED, BASE);
const of = (value: unknown) => tableSettingsOf(value, FIXED, BASE);

test('table settings keep density, overflow, widths and sort per placement', () => {
  const changed = updateTableSettings(DEFAULTS, next => {
    next.density = 'compact';
    next.overflow = 'scroll';
    next.columns.owner!.show = false;
    next.columns.state!.width = 240;
    next.sort = { column: 'compliance', direction: 'ascending' };
  });
  const restored = of(encodeTableSettings(changed));
  assert.equal(restored.density, 'compact');
  assert.equal(restored.overflow, 'scroll');
  assert.equal(restored.columns.owner!.show, false);
  assert.equal(restored.columns.state!.width, 240);
  assert.deepEqual(restored.sort, { column: 'compliance', direction: 'ascending' });
});

test('malformed table settings use the family defaults', () => {
  assert.deepEqual(of('{"v":1,"overflow":"scroll","columns":{}}'), DEFAULTS);
});

test('partial column orders retain each default column exactly once', () => {
  const restored = of(JSON.stringify({ ...JSON.parse(encodeTableSettings(DEFAULTS)), order: ['owner', 'owner'] }));
  assert.deepEqual(restored.order, ['owner', 'state', 'occupier', 'resistance', 'compliance', 'law']);
});

test('country and law displays survive a round trip, and a display a column does not offer falls back to its default', () => {
  assert.equal(DEFAULTS.columns.owner!.as, 'flagName');
  const changed = updateTableSettings(DEFAULTS, next => {
    next.columns.owner!.as = 'flag';
    next.columns.occupier!.as = 'name';
    next.columns.law!.as = 'icon';
  });
  const restored = of(encodeTableSettings(changed));
  assert.deepEqual([restored.columns.owner!.as, restored.columns.occupier!.as, restored.columns.law!.as], ['flag', 'name', 'icon']);
  const foreign = updateTableSettings(DEFAULTS, next => {
    next.columns.owner!.as = 'plain';
    next.columns.law!.as = 'meterAfter';
  });
  const fallen = of(encodeTableSettings(foreign));
  assert.deepEqual([fallen.columns.owner!.as, fallen.columns.law!.as], ['flagName', 'dot']);
});

test('a column set known only at run time takes each column’s defaults from its spec', () => {
  const measures = (...keys: string[]): TableColumnDefaults[] => [
    { key: 'country', label: 'Country', as: 'flagName', offered: COUNTRY },
    ...keys.map(key => ({ key, label: key, as: 'plain' as const, offered: NUMBERS })),
  ];
  const base: TableBase = { density: 'comfortable', overflow: 'tooltip', sort: { column: 'country', direction: 'ascending' } };
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

test('displays reserve their room, numbers align right unless bar leads, and rank markers hold until expiry', () => {
  assert.deepEqual(['plain', 'faint', 'meterAfter', 'flagName', 'flag', 'name', 'dot'].map(display => displayExtra({ display: display as never })), [0, 4, 52, 26, 20, 0, 12]);
  assert.deepEqual(['plain', 'meterAfter', 'track', 'meterBefore'].map(display => endAligned({ numeric: true, display: display as never })), [true, true, false, false]);
  assert.equal(endAligned({ numeric: false, display: 'plain' }), false);
  assert.deepEqual(['4449999', '4450000', '12000000', '-1'].map(percentCell), ['44%', '45%', '100%', '0%']);
  assert.deepEqual([173_611n, 0n, -5n, 99_999n].map(amountCell), ['1.73', '-', '!', '0.99']);
  assert.equal(strengthBar('87544'), 88000n);
  const first = rankMarkers(['GER', 'ITA', 'SOV'], ['SOV', 'GER', 'ITA', 'ENG'], 1000, new Map(), 2500);
  assert.deepEqual(Object.fromEntries(first), { SOV: { delta: 2, until: 3500 }, GER: { delta: -1, until: 3500 }, ITA: { delta: -1, until: 3500 } });
  const later = rankMarkers(['SOV', 'GER', 'ITA', 'ENG'], ['SOV', 'GER', 'ENG', 'ITA'], 2000, first, 2500);
  assert.deepEqual(Object.fromEntries(later), { SOV: { delta: 2, until: 3500 }, GER: { delta: -1, until: 3500 }, ENG: { delta: 1, until: 4500 }, ITA: { delta: -1, until: 4500 } });
  assert.deepEqual(Object.fromEntries(rankMarkers(['SOV', 'GER', 'ENG', 'ITA'], ['SOV', 'GER', 'ENG', 'ITA'], 4000, later, 2500)), { ENG: { delta: 1, until: 4500 }, ITA: { delta: -1, until: 4500 } });
});
