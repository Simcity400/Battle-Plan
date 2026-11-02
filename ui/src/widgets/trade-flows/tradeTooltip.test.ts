import assert from 'node:assert/strict';
import test from 'node:test';
import { buildDiagram, decodeTrade, identityCountryAt } from './tradeData.ts';
import { nodeTooltip, ribbonTooltip, TOOLTIP_LIMIT } from './tradeTooltip.ts';
import type { TradeFlowsReply } from '../../generated/dashboard';
import type { NameAt } from '../../dashboard/countryNamesAt';

const identity = (slot: number) => JSON.stringify([slot, slot + 1]);
const identities = (countries: readonly string[], lastHour: number) => countries.map((tag, slot) => ({ tag, runs: [{ firstHour: 0, lastHour, slot, countryId: slot + 1 }] }));

test('a sole trade through either other node names and flags both countries at the hovered hour', () => {
  for (const reverse of [false, true]) {
    const reply: TradeFlowsReply = { branchId: '1', decimals: 0, countries: ['A', 'B', 'C', 'D', 'X'],
      countryIdentities: identities(['A', 'B', 'C', 'D', 'X'], 10),
      resources: [1, 2].map(id => ({ id, token: `R${id}`, ordinal: 1, name: null, icon: null })),
      reads: [0, 5], known: [{ firstHour: 0, lastHour: 10 }], partial: [], warnings: [],
      flows: [100, 90, 10, 5].map((value, index) => ({ seller: reverse ? 4 : index, buyer: reverse ? index : 4, resource: index === 3 ? 2 : 1,
        sellerIdentity: identity(reverse ? 4 : index), buyerIdentity: identity(reverse ? index : 4), sellerTags: [reverse ? 4 : index], buyerTags: [reverse ? index : 4],
        firstHours: [0], lastHours: [10], deliveredRaw: [String(value)], factories: [1], convoysAssigned: [0], convoysRequired: [0], deals: [1] })) };
    const data = decodeTrade(reply);
    const diagram = buildDiagram(data.flows, 'current', 0, 5, 2);
    const nodeById = new Map(diagram.nodes.map(node => [node.id, node]));
    const link = diagram.links.find(link => link.resource.id === 1 && link.flows.length === 1 && (reverse ? link.flows[0].buyer : link.flows[0].seller) === 'C')!;
    const source = nodeById.get(link.source)!;
    const target = nodeById.get(link.target)!;
    assert.deepEqual((reverse ? target : source).members, [identity(2), identity(3)]);
    const asked: (readonly NameAt[])[] = [];
    const tip = ribbonTooltip(link, source, target, { measure: 'current', from: 0, hour: 5, decimals: 0, viewport: 1000,
      nodeName: node => `${node.members.length} other countries`,
      labelAt: hour => tag => ({ name: `${tag} at ${hour}`, flag: `flag:${tag}:${hour}` }),
      countryAt: (id, hour) => identityCountryAt(data, id, hour), ask: wanted => { asked.push(wanted); } });
    assert.deepEqual(asked, [[{ tag: 'C', hour: 5 }, { tag: 'X', hour: 5 }]]);
    assert.equal(tip.title, `${reverse ? 'X' : 'C'} at 5 → ${reverse ? 'C' : 'X'} at 5`);
    assert.ok(!tip.rows.some(row => row.label === 'Seller' || row.label === 'Buyer'));
    const failure = new Error('Synthetic trade country name decoder failure');
    const failed = ribbonTooltip(link, source, target, { measure: 'current', from: 0, hour: 5, decimals: 0, viewport: 1000,
      nodeName: node => `${node.members.length} other countries`,
      labelAt: () => () => ({ name: 'Couldn’t load country name.', nameFailure: failure }),
      countryAt: (id, hour) => identityCountryAt(data, id, hour), ask() {} });
    assert.equal(failed.detail, failure.message);
    assert.match(failed.title, /Couldn’t load country name/);
  }
});

test('node tooltips share dated labels and bound their sorted country requests', () => {
  for (const reverse of [false, true]) {
    const countries = [...Array.from({ length: 11 }, (_, index) => `Seller ${String(index).padStart(2, '0')}`), 'X'];
    const reply: TradeFlowsReply = { branchId: '1', decimals: 0, countries,
      countryIdentities: identities(countries, 23),
      resources: [3, 7].map(id => ({ id, token: `R${id}`, ordinal: 1, name: id === 7 ? 'Mod resource seven' : 'Mod resource three', icon: null })),
      reads: [0, 5], known: [{ firstHour: 0, lastHour: 23 }], partial: [], warnings: [],
      flows: [...countries.slice(0, -1).map((_, index) => ({ seller: reverse ? 11 : index, buyer: reverse ? index : 11, resource: 7,
        sellerIdentity: identity(reverse ? 11 : index), buyerIdentity: identity(reverse ? index : 11), sellerTags: [reverse ? 11 : index], buyerTags: [reverse ? index : 11],
        firstHours: [0], lastHours: [23], deliveredRaw: [String(index === 0 ? 100 : 11 - index)], factories: [1], convoysAssigned: [0], convoysRequired: [0], deals: [1] })),
      { seller: reverse ? 11 : 1, buyer: reverse ? 1 : 11, resource: 3,
        sellerIdentity: identity(reverse ? 11 : 1), buyerIdentity: identity(reverse ? 1 : 11), sellerTags: [reverse ? 11 : 1], buyerTags: [reverse ? 1 : 11],
        firstHours: [0], lastHours: [23], deliveredRaw: ['2'], factories: [1], convoysAssigned: [0], convoysRequired: [0], deals: [1] }] };
    const data = decodeTrade(reply);
    const diagram = buildDiagram(data.flows, 'current', 0, 23, 1);
    const node = diagram.nodes.find(node => node.tag == null)!;
    for (const measure of ['current', 'total', 'average'] as const) {
      const asked: (readonly NameAt[])[] = [];
      const tipHour = 23;
      const options = { measure, from: 0, hour: 23, decimals: 0, viewport: 1000,
        nodeName: () => 'Other countries', resourceOrder: new Map([[7, 0], [3, 1]]), resourceColour: (id: number) => `colour:${id}`,
        labelAt: (hour: number) => (tag: string) => ({ name: `${tag} at ${hour}`, flag: `flag:${tag}:${hour}` }),
        countryAt: (id: string, hour: number) => identityCountryAt(data, id, hour), ask: (wanted: readonly NameAt[]) => { asked.push(wanted); } };
      const tip = nodeTooltip(node, diagram.links, options);
      const failure = new Error('Synthetic folded trade name decoder failure');
      const ribbon = diagram.links.find(link => link.flows.length > 1)!;
      const failed = ribbonTooltip(ribbon, diagram.nodes.find(node => node.id === ribbon.source)!, diagram.nodes.find(node => node.id === ribbon.target)!,
        { ...options, ask() {}, labelAt: () => () => ({ name: 'Couldn’t load country name.', nameFailure: failure }) });
      const failedNames = failed.rows.filter(row => row.label.includes('Couldn’t load country name.'));
      assert.ok(failedNames.length > 0);
      assert.ok(failedNames.every(row => row.detail === failure.message));
      assert.equal(TOOLTIP_LIMIT, 8);
      assert.deepEqual(asked, [countries.slice(1, 9).map(tag => ({ tag, hour: tipHour }))]);
      assert.equal(tip.title, 'Other countries');
      assert.equal(tip.countryFlag, undefined, 'an aggregate keeps its flagless header');
      assert.ok(tip.subtitle?.startsWith(reverse ? 'Buys · ' : 'Sells · '));
      assert.deepEqual(tip.rows.slice(0, 2).map(row => [row.label, row.value, row.swatch]), [['Mod resource seven', '55', 'colour:7'], ['Mod resource three', '2', 'colour:3']]);
      assert.equal(tip.rows[2].value, '57');
      assert.equal(tip.rows[3].label, `Seller 01 at ${tipHour}`);
      assert.equal(tip.rows[3].countryFlag?.flag, `flag:Seller 01:${tipHour}`);
      assert.equal(tip.rows.at(-1)?.label, '2 more');
      const country = diagram.nodes.find(node => node.tag === countries[0])!;
      const single = nodeTooltip(country, diagram.links, options);
      assert.equal(single.title, `${countries[0]} at ${tipHour}`);
      assert.equal(single.countryFlag?.flag, `flag:${countries[0]}:${tipHour}`);
      assert.deepEqual(asked.at(-1), [{ tag: countries[0], hour: tipHour }]);
      assert.equal(single.rows.at(-1)?.value, '100');
    }
  }
});

test('retired and current holders sharing a tag use their separate supplying dates in total tooltips', () => {
  const reply: TradeFlowsReply = { branchId: '1', decimals: 0, countries: ['A', 'B'],
    countryIdentities: [{ tag: 'A', runs: [{ firstHour: 0, lastHour: 23, slot: 0, countryId: 1 }, { firstHour: 24, lastHour: 47, slot: 1, countryId: 2 }] },
      { tag: 'B', runs: [{ firstHour: 0, lastHour: 47, slot: 2, countryId: 3 }] }],
    resources: [{ id: 1, token: 'mod-resource', ordinal: 1, name: null, icon: null }],
    reads: [0, 24], known: [{ firstHour: 0, lastHour: 47 }], partial: [], warnings: [],
    flows: [0, 1].map(slot => ({ seller: 0, buyer: 1, sellerIdentity: identity(slot), buyerIdentity: identity(2), sellerTags: [0], buyerTags: [1],
      resource: 1, firstHours: [slot * 24], lastHours: [slot * 24 + 23], deliveredRaw: ['10'], factories: [1], convoysAssigned: [0], convoysRequired: [0], deals: [1] })) };
  const data = decodeTrade(reply);
  const diagram = buildDiagram(data.flows, 'total', 0, 47, 2);
  const nodes = new Map(diagram.nodes.map(node => [node.id, node]));
  const requests: (readonly NameAt[])[] = [];
  const titles = diagram.links.map(link => ribbonTooltip(link, nodes.get(link.source)!, nodes.get(link.target)!, {
    measure: 'total', from: 0, hour: 47, decimals: 0, viewport: 1000, nodeName: () => 'Other',
    countryAt: (id, at) => identityCountryAt(data, id, at), labelAt: at => tag => ({ name: `${tag} at ${at}` }),
    ask: wanted => requests.push(wanted),
  }).title);
  assert.deepEqual(titles, ['A at 23 → B at 47', 'A at 47 → B at 47']);
  assert.deepEqual(requests, [[{ tag: 'A', hour: 23 }, { tag: 'B', hour: 47 }], [{ tag: 'A', hour: 47 }, { tag: 'B', hour: 47 }]]);
  const headers = diagram.nodes.filter(node => node.identity).map(node => nodeTooltip(node, diagram.links, {
    measure: 'total', from: 0, hour: 47, decimals: 0, viewport: 1000, nodeName: () => 'Other',
    resourceOrder: undefined, resourceColour: () => undefined,
    countryAt: (id, at) => identityCountryAt(data, id, at),
    labelAt: at => tag => ({ name: `${tag} at ${at}`, flagReason: `Unavailable ${tag} at ${at}`, flagPending: false }), ask() {},
  }));
  assert.deepEqual(headers.filter(tip => tip.title.startsWith('A')).map(tip => tip.countryFlag?.flagReason),
    ['Unavailable A at 23', 'Unavailable A at 47'], 'retired and current holders preserve their distinct flag refusal dates');
  assert.ok(headers.every(tip => tip.countryFlag && tip.countryFlag.flagPending === false && tip.icon === undefined));
  const unresolved = diagram.nodes.find(node => node.identity)!;
  const missing = nodeTooltip(unresolved, diagram.links, {
    measure: 'total', from: 0, hour: 47, decimals: 0, viewport: 1000, nodeName: () => 'Other',
    resourceOrder: undefined, resourceColour: () => undefined, countryAt: () => null,
    labelAt: () => { throw new Error('An unresolved holder has no native country flag read.'); },
    ask: wanted => assert.deepEqual(wanted, []),
  });
  assert.equal(missing.title, 'Country name unavailable');
  assert.equal(missing.countryFlag, undefined);
});
