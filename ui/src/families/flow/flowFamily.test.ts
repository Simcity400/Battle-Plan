import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../boundedView.ts';
import type { CountryFlowReply } from '../../generated/dashboard';
import { projectCountryFlow, type CountryFlowText } from './countryFlow.ts';
import { deriveFlowAudit, findFlowTooltip } from './model.ts';
import { hitFlow, layoutFlow } from './painter.ts';

const HOUR = 60_810_000;
const TEXT: CountryFlowText = {
  leftHeader: 'Imports from',
  rightHeader: 'Exports to',
  unitNote: 'per day',
  groupLeftHeader: 'Exporters',
  groupRightHeader: 'Importers',
  notRecorded: 'Trade was not recorded at this date.',
  none: 'No trade at this date.',
  others: count => `${count} other partners`,
  ribbonTitle: (from, to) => `${from} → ${to}`,
};
const parts = [
  { key: 'oil', name: 'Oil', nameMissing: null, ordinal: 0, icon: null },
  { key: 'steel', name: 'Steel', nameMissing: null, ordinal: 1, icon: null },
];
const base: CountryFlowReply = {
  countries: [], subject: null, decimals: 5, parts, incoming: [], outgoing: [], incomingTotal: null, outgoingTotal: null,
  links: [], senders: [], receivers: [], table: [], largest: '0', largestLink: '0', missing: null, partial: null, warnings: [],
};
const amount = (value: number) => String(value * 100_000);
const view = (reply: unknown) => createBoundedReadView({ hour: HOUR, panels: new Map([['main', reply]]) });

const centred: CountryFlowReply = {
  ...base,
  countries: ['USA', 'HOL', 'RAJ', 'ENG'],
  subject: 0,
  incoming: [
    { from: 1, to: 0, others: null, amounts: [{ part: 0, amount: amount(20) }, { part: 1, amount: amount(6) }], total: amount(26), missing: null },
    { from: 2, to: 0, others: null, amounts: [{ part: 1, amount: null }], total: null, missing: 'A deal\'s daily delivery was not recorded at this hour' },
    { from: null, to: 0, others: 14, amounts: [{ part: 0, amount: amount(14) }], total: amount(14), missing: null },
  ],
  outgoing: [{ from: 0, to: 3, others: null, amounts: [{ part: 0, amount: amount(74) }], total: amount(74), missing: null }],
  incomingTotal: null,
  outgoingTotal: amount(74),
  largest: amount(74),
  largestLink: amount(74),
};

test('a country-centred flow: partners by the product\'s order, an unrecorded partner hatched with its reason, the rest folded, never a zero', () => {
  const model = projectCountryFlow(view(centred), TEXT);
  assert.equal(model.mode, 'centred');
  assert.deepEqual(model.left.map(node => [node.key, node.numeric, node.figure]), [
    ['HOL', 26, '26'],
    ['RAJ', null, 'Not recorded'],
    ['\u0000others-in', 14, '14'],
  ]);
  assert.equal(model.left[2]!.label, '14 other partners');
  assert.equal(model.right[0]!.numeric, 74);
  assert.equal(model.notRecordedKey, true, 'a hatched partner adds the not-recorded key');
  const ribbon = model.ribbons.find(item => item.from === 'HOL')!;
  assert.deepEqual(ribbon.bands, [{ part: 0, numeric: 20 }, { part: 1, numeric: 6 }]);
  assert.equal(findFlowTooltip(model, 'ribbon:RAJ>USA')?.missingReason, 'A deal\'s daily delivery was not recorded at this hour');
  assert.ok(deriveFlowAudit(model).some(row => row.key === 'node:RAJ' && row.value === 'Not recorded'));
});

test('the layout gives every ribbon its width at both ends, keeps labels apart, and hit-tests what it draws', () => {
  const model = projectCountryFlow(view(centred), TEXT);
  const layout = layoutFlow(model, 846, 360);
  const hol = layout.nodes.find(box => box.node.key === 'HOL')!;
  const holRibbon = layout.ribbons.find(box => box.ribbon.from === 'HOL')!;
  assert.ok(Math.abs((holRibbon.a1 - holRibbon.a0) - (holRibbon.b1 - holRibbon.b0)) < 0.01, 'a ribbon keeps its width');
  assert.ok(Math.abs(hol.height - 26 * layout.scale) < 0.01);
  const lefts = layout.nodes.filter(box => box.node.side === 'left');
  for (let i = 1; i < lefts.length; i++) assert.ok(lefts[i]!.slotTop >= lefts[i - 1]!.slotTop + lefts[i - 1]!.slotHeight, 'label slots never overlap');
  const middleX = (holRibbon.x0 + holRibbon.x1) / 2;
  const middleY = (holRibbon.a0 + holRibbon.b0 + holRibbon.a1 + holRibbon.b1) / 4;
  assert.equal(hitFlow(layout, middleX, middleY), 'ribbon:HOL>USA');
  assert.equal(hitFlow(layout, hol.x - 20, hol.y + hol.height / 2), 'node:HOL');
  assert.ok(layout.bodyTop + 26 * layout.scale <= 360);
});

test('a group subject is a bipartite flow of the product\'s senders and receivers', () => {
  const reply: CountryFlowReply = {
    ...base,
    countries: ['ITA', 'SOV', 'SPD', 'SPR'],
    senders: [
      { from: 0, to: null, others: null, amounts: [{ part: 0, amount: amount(9) }], total: amount(9), missing: null },
      { from: 1, to: null, others: null, amounts: [{ part: 0, amount: amount(4) }], total: amount(4), missing: null },
    ],
    receivers: [
      { from: null, to: 2, others: null, amounts: [{ part: 0, amount: amount(9) }], total: amount(9), missing: null },
      { from: null, to: 3, others: null, amounts: [{ part: 0, amount: amount(4) }], total: amount(4), missing: null },
    ],
    links: [
      { from: 0, to: 2, others: null, amounts: [{ part: 0, amount: amount(9) }], total: amount(9), missing: null },
      { from: 1, to: 3, others: null, amounts: [{ part: 0, amount: amount(4) }], total: amount(4), missing: null },
    ],
    largest: amount(9),
    largestLink: amount(9),
  };
  const model = projectCountryFlow(view(reply), TEXT);
  assert.equal(model.mode, 'bipartite');
  assert.deepEqual(model.left.map(node => node.key), ['ITA', 'SOV']);
  assert.deepEqual(model.right.map(node => node.key), ['SPD', 'SPR']);
  assert.deepEqual(model.ribbons.map(ribbon => ribbon.key), ['ITA>SPD', 'SOV>SPR']);
  assert.equal(model.leftHeader, 'Exporters');
});

test('not recorded and empty are different states', () => {
  assert.equal(projectCountryFlow(view({ ...base, missing: 'Trade flows were not read at this hour' }), TEXT).emptyKind, 'unavailable');
  const none = projectCountryFlow(view({ ...base, countries: ['USA'], subject: 0 }), TEXT);
  assert.equal(none.empty, 'No trade at this date.');
  assert.equal(none.emptyKind, undefined);
});
