import assert from 'node:assert/strict';
import test from 'node:test';
import { buildDiagram, decodeTrade } from './tradeData.ts';
import { nodeTooltip, ribbonTooltip, TOOLTIP_LIMIT } from './tradeTooltip.ts';
import type { TradeFlowsReply } from '../../generated/dashboard';

test('a sole trade through either other node names and flags both countries at the hovered hour', () => {
  for (const reverse of [false, true]) {
    const reply: TradeFlowsReply = { branchId: '1', decimals: 0, countries: ['A', 'B', 'C', 'D', 'X'],
      resources: [1, 2].map(id => ({ id, token: `R${id}`, ordinal: 1, name: null, icon: null })),
      reads: [0, 5], known: [{ firstHour: 0, lastHour: 10 }], partial: [], warnings: [],
      flows: [100, 90, 10, 5].map((value, index) => ({ seller: reverse ? 4 : index, buyer: reverse ? index : 4, resource: index === 3 ? 2 : 1,
        firstHours: [0], lastHours: [10], deliveredRaw: [String(value)], factories: [1], convoysAssigned: [0], convoysRequired: [0], deals: [1] })) };
    const diagram = buildDiagram(decodeTrade(reply).flows, 'current', 0, 5, 2);
    const nodeById = new Map(diagram.nodes.map(node => [node.id, node]));
    const link = diagram.links.find(link => link.resource.id === 1 && link.flows.length === 1 && (reverse ? link.flows[0].buyer : link.flows[0].seller) === 'C')!;
    const source = nodeById.get(link.source)!;
    const target = nodeById.get(link.target)!;
    assert.deepEqual((reverse ? target : source).members, ['C', 'D']);
    const asked: [number, readonly string[]][] = [];
    const tip = ribbonTooltip(link, source, target, { measure: 'current', from: 0, hour: 5, read: 5, decimals: 0, viewport: 1000,
      nodeName: node => `${node.members.length} other countries`,
      labelAt: hour => tag => ({ name: `${tag} at ${hour}`, flag: `flag:${tag}:${hour}` }),
      ask: (hour, tags) => { asked.push([hour, tags]); } });
    assert.deepEqual(asked, [[5, ['C', 'X']]]);
    assert.equal(tip.title, `${reverse ? 'X' : 'C'} at 5 → ${reverse ? 'C' : 'X'} at 5`);
    assert.deepEqual(tip.rows.slice(-2).map(row => [row.label, row.value, row.icon]), [
      ['Seller', `${reverse ? 'X' : 'C'} at 5`, `flag:${reverse ? 'X' : 'C'}:5`],
      ['Buyer', `${reverse ? 'C' : 'X'} at 5`, `flag:${reverse ? 'C' : 'X'}:5`],
    ]);
  }
});

test('node tooltips share dated labels and bound their sorted country requests', () => {
  for (const reverse of [false, true]) {
    const countries = [...Array.from({ length: 11 }, (_, index) => `Seller ${String(index).padStart(2, '0')}`), 'X'];
    const reply: TradeFlowsReply = { branchId: '1', decimals: 0, countries,
      resources: [3, 7].map(id => ({ id, token: `R${id}`, ordinal: 1, name: null, icon: null })),
      reads: [0, 5], known: [{ firstHour: 0, lastHour: 23 }], partial: [], warnings: [],
      flows: [...countries.slice(0, -1).map((_, index) => ({ seller: reverse ? 11 : index, buyer: reverse ? index : 11, resource: 7,
        firstHours: [0], lastHours: [23], deliveredRaw: [String(index === 0 ? 100 : 11 - index)], factories: [1], convoysAssigned: [0], convoysRequired: [0], deals: [1] })),
      { seller: reverse ? 11 : 1, buyer: reverse ? 1 : 11, resource: 3,
        firstHours: [0], lastHours: [23], deliveredRaw: ['2'], factories: [1], convoysAssigned: [0], convoysRequired: [0], deals: [1] }] };
    const diagram = buildDiagram(decodeTrade(reply).flows, 'current', 0, 23, 1);
    const node = diagram.nodes.find(node => node.tag == null)!;
    for (const measure of ['current', 'total', 'average'] as const) {
      const asked: [number, readonly string[]][] = [];
      const tipHour = measure === 'current' ? 5 : 23;
      const options = { measure, from: 0, hour: 23, read: 5, decimals: 0, viewport: 1000,
        nodeName: () => 'Other countries', resourceOrder: new Map([[7, 0], [3, 1]]), resourceColour: (id: number) => `colour:${id}`,
        labelAt: (hour: number) => (tag: string) => ({ name: `${tag} at ${hour}`, flag: `flag:${tag}:${hour}` }),
        ask: (hour: number, tags: readonly string[]) => { asked.push([hour, tags]); } };
      const tip = nodeTooltip(node, diagram.links, options);
      assert.equal(TOOLTIP_LIMIT, 8);
      assert.deepEqual(asked, [[tipHour, countries.slice(1, 9)]]);
      assert.equal(tip.title, 'Other countries');
      assert.ok(tip.subtitle?.startsWith(reverse ? 'Buys · ' : 'Sells · '));
      assert.deepEqual(tip.rows.slice(0, 2).map(row => [row.label, row.value, row.swatch]), [['R7', '55', 'colour:7'], ['R3', '2', 'colour:3']]);
      assert.equal(tip.rows[2].value, '57');
      assert.equal(tip.rows[3].label, `Seller 01 at ${tipHour}`);
      assert.equal(tip.rows[3].icon, `flag:Seller 01:${tipHour}`);
      assert.equal(tip.rows.at(-1)?.label, '2 more');
      const country = diagram.nodes.find(node => node.tag === countries[0])!;
      const single = nodeTooltip(country, diagram.links, options);
      assert.equal(single.title, `${countries[0]} at ${tipHour}`);
      assert.equal(single.icon, `flag:${countries[0]}:${tipHour}`);
      assert.deepEqual(asked.at(-1), [tipHour, [countries[0]]]);
      assert.equal(single.rows.at(-1)?.value, '100');
    }
  }
});
