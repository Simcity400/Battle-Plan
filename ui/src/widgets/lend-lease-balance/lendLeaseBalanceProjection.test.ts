import assert from 'node:assert/strict';
import test from 'node:test';
import { assertBarsModelValid, formatBarsValueField, type BarsDataRow } from '../../families/bars/model.ts';
import { createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import type { IdentityProvider } from '../../families/plan.ts';
import type { LendLeaseBalanceReply } from '../../generated/dashboard';
import { lendLeaseBalanceProjection, type LendLeaseBalanceOptions } from './lendLeaseBalanceProjection.ts';

const identity: IdentityProvider = {
  resolve(kind, key, atHour) {
    const name = ({ SOV: 'Soviet Union', ETH: 'Ethiopia', MOD: 'Modland' } as Record<string, string>)[key];
    return {
      kind, key, atHour,
      holder: { status: 'ready', value: key },
      tag: { status: 'ready', value: key },
      name: name ? { status: 'ready', value: name } : { status: 'absent', reason: 'Name not recorded' },
      colour: { status: 'ready', value: '#c0392b' },
      flagChoice: { status: 'absent', reason: 'Flag not recorded' },
      validInterval: [0, 1000],
      nextChangeAt: Infinity,
    };
  },
};

const reply = (rows: LendLeaseBalanceReply['rows'], missing: string | null = null): LendLeaseBalanceReply =>
  ({ holder: null, scale: 100000, rows, missing, rankMissing: null, unlisted: null, warnings: [] });

const view = (data: LendLeaseBalanceReply) =>
  createBoundedReadView({ hour: displayedHour(500), panels: new Map([['main', data]]), identity });

const TOP: LendLeaseBalanceOptions = { measure: 'industry', chosen: null, membership: null, unobserved: [] };

const sides = (row: BarsDataRow) => [row.left?.value.value, row.right?.value.value];

test('the product\'s order is kept, received is the left side and sent the right, a recorded zero is zero', () => {
  const model = lendLeaseBalanceProjection.project(view(reply([
    { slot: 3, tag: 'SOV', values: ['552700000', '0', '391200000', '0'] },
    { slot: 9, tag: 'ETH', values: ['0', '0', '678150000', '0'] },
  ])), TOP);
  assertBarsModelValid(model, 500);
  assert.equal(model.form, 'two-sided');
  assert.equal(model.leftHeader, 'Industry received');
  assert.equal(model.rightHeader, 'Industry sent');
  const rows = model.rows as BarsDataRow[];
  assert.deepEqual(rows.map(row => row.label), [{ kind: 'known', value: 'Soviet Union' }, { kind: 'known', value: 'Ethiopia' }], 'no client reordering');
  assert.deepEqual(sides(rows[0]!), [{ kind: 'known', value: 391200n }, { kind: 'known', value: 552700n }]);
  assert.deepEqual(sides(rows[1]!), [{ kind: 'known', value: 678150n }, { kind: 'zero', value: 0n }]);
  assert.deepEqual([formatBarsValueField(rows[0]!.left!.value).text, formatBarsValueField(rows[1]!.left!.value).text], ['3,912', '6,781.5']);
  assert.equal(rows[1]!.right?.numeric, 0, 'a known zero draws no bar');
  assert.equal(rows[0]!.left?.numeric, 3912);
});

test('amounts keep the game fixed-point truncation and a sub-hundredth amount is never shown as zero', () => {
  const model = lendLeaseBalanceProjection.project(view(reply([{ slot: 3, tag: 'SOV', values: ['153413999', '0', '7', '0'] }])), TOP);
  const row = model.rows[0] as BarsDataRow;
  assert.equal(formatBarsValueField(row.right!.value).text, '1,534.13');
  assert.equal(row.left?.value.value.kind, 'known');
  assert.equal(formatBarsValueField(row.left!.value).text, '0.00007');
});

test('the fuel measure reads the fuel columns', () => {
  const model = lendLeaseBalanceProjection.project(view(reply([{ slot: 3, tag: 'SOV', values: ['1', '200000', '3', '400000'] }])), { ...TOP, measure: 'fuel' });
  const row = model.rows[0] as BarsDataRow;
  assert.equal(model.leftHeader, 'Fuel received');
  assert.deepEqual([row.left?.numeric, row.right?.numeric], [4, 2]);
});

test('unrecorded histories are the empty state with their reason, never a row of zeros', () => {
  const model = lendLeaseBalanceProjection.project(view(reply([], 'Lend-lease histories are not recorded at this hour: unreadable')), TOP);
  assert.equal(model.rows.length, 0);
  assert.equal(model.emptyKind, 'unavailable');
  assert.match(model.empty ?? '', /not recorded/);
});

test('a chosen country the reply does not hold is not recorded and comes last; a nameless slot is marked missing', () => {
  const model = lendLeaseBalanceProjection.project(view(reply([
    { slot: 4, tag: null, values: ['100000', '0', '0', '0'] },
    { slot: 3, tag: 'SOV', values: ['0', '0', '0', '0'] },
  ])), { ...TOP, chosen: ['SOV', 'MOD', '\u00004'] });
  const rows = model.rows as BarsDataRow[];
  assert.deepEqual(rows.map(row => [row.key, typeof row.label !== 'string' && row.label.kind === 'notRecorded']), [['\u00004', true], ['\u00003', false], ['MOD', false]]);
  assert.equal(rows[2]!.left?.value.value.kind, 'notRecorded');
  assert.equal(rows[2]!.left?.numeric, null);
  assert.deepEqual(sides(rows[1]!), [{ kind: 'zero', value: 0n }, { kind: 'zero', value: 0n }]);
});

test('a group whose members were not recorded at the hour says so instead of drawing rows', () => {
  const model = lendLeaseBalanceProjection.project(view(reply([{ slot: 3, tag: 'SOV', values: ['1', '0', '0', '0'] }])),
    { ...TOP, chosen: ['SOV'], membership: { SOV: [[0, 1000]] }, unobserved: [[400, 600]] });
  assert.equal(model.rows.length, 0);
  assert.equal(model.emptyKind, 'unavailable');
});
