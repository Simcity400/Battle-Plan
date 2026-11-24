import assert from 'node:assert/strict';
import test from 'node:test';
import type { AmountRun, EquipmentStockpileReply, StockVariant } from './types';
import { resolveY } from '../../charts/view.ts';
import { storableToken } from '../../dashboard/options.ts';
import { choiceKey, choiceRow, designAt, hasRecords, stockChoices, typeName, variantKey, balanceAt, typesComplete, variantsComplete, balanceMode, formatPercent, groupTooltip, labelAt, shareOf, shareRuns, stockAt, stockGroups, stockRuns, variantName, dailyAt } from './stockpileData.ts';

const U = 100000;
const run = (firstHour: number, lastHour: number, amount: number): AmountRun => ({ firstHour, lastHour, amountRaw: String(Math.round(amount * U)) });
const variant = (id: string, token: string, archetype: string | null, stock: AmountRun[], daily: AmountRun[] = [],
  names: (Omit<StockVariant['names'][number], 'design'> & { design?: string | null })[] = []): StockVariant =>
  ({ id, token, archetype, names: names.map(name => ({ design: null, ...name })), stock, daily, dailyIc: [], unitCostLow: [], unitCostHigh: [] });
const TRUCKS = 'The game adds the supply system’s trucks to its stock here, which the recording lacks, so what is needed cannot be derived.';

const REPLY: EquipmentStockpileReply = {
  branchId: '1', tag: 'GER', campaignKey: 'c0ffee', decimals: 5, warnings: [], reads: [100, 124, 140],
  stockHours: [{ firstHour: 100, lastHour: 130 }, { firstHour: 140, lastHour: 150 }],
  lineHours: [{ firstHour: 100, lastHour: 123 }, { firstHour: 140, lastHour: 150 }],
  types: [
    { key: 'infantry_equipment', name: 'Infantry Equipment', domain: 'land', needNote: null },
    { key: 'light_tank_chassis', name: 'Light Tank', domain: 'land', needNote: null },
    { key: 'anti_tank_equipment', name: 'Anti-Tank', domain: 'land', needNote: null },
    { key: 'motorized_equipment', name: 'Truck', domain: 'land', needNote: TRUCKS },
    { key: 'cas_equipment', name: 'Close Air Support', domain: 'air', needNote: null },
    { key: 'fighter_equipment', name: 'Fighter', domain: 'air', needNote: null },
  ],
  variants: [
    variant('1326', 'infantry_equipment_1', 'infantry_equipment', [run(100, 123, 145666), run(124, 130, 146000), run(140, 150, 140000)], [run(100, 123, 541.9)]),
    variant('1351', 'light_tank_chassis_1', 'light_tank_chassis', [run(100, 130, 720)], [], [{ firstHour: 90, name: 'Panzer I Ausf. A', known: true }]),
    variant('1354', 'light_tank_chassis_2', 'light_tank_chassis', [run(100, 130, 1396)], [run(100, 123, 15)], [{ firstHour: 90, name: 'Panzer II', known: true }, { firstHour: 120, name: 'Panzer II Ausf. F', known: true }]),
    variant('1329', 'motorized_equipment_1', 'motorized_equipment', [run(100, 130, 5195)]),
    variant('1375', 'cas_1', 'cas_equipment', [run(100, 130, 157)]),
    variant('1400', 'fighter_equipment_1', 'fighter_equipment', [], [run(100, 123, 19.1)]),
  ],
  logistics: [
    { key: 'infantry_equipment', available: [], balance: [run(96, 119, 1549), run(120, 130, 1000)], stock: [run(96, 130, 145666)] },
    { key: 'light_tank_chassis', available: [], balance: [run(96, 130, -373)], stock: [run(96, 130, 720)] },
    { key: 'anti_tank_equipment', available: [], balance: [run(96, 130, -1225)], stock: [] },
    { key: 'motorized_equipment', available: [], balance: [run(96, 130, -40)], stock: [run(96, 130, 5195)] },
  ],
  logisticsReads: [96, 120], lineReads: [100, 140],
  balanceHours: [{ firstHour: 96, lastHour: 130 }],
  aiHours: [{ firstHour: 140, lastHour: 150 }],
  needBoundRaw: String(0x53e2d620fb40),
};

test('groups follow the game’s order, logistics-only types included', () => {
  assert.deepEqual(stockGroups(REPLY, 'type', 110).map(group => [group.key, group.variants.length, group.types.join()]), [
    ['infantry_equipment', 1, 'infantry_equipment'], ['light_tank_chassis', 2, 'light_tank_chassis'], ['anti_tank_equipment', 0, 'anti_tank_equipment'],
    ['motorized_equipment', 1, 'motorized_equipment'], ['cas_equipment', 1, ''], ['fighter_equipment', 1, '']]);
  assert.deepEqual(stockGroups(REPLY, 'domain', 110).map(group => [group.key, group.label, group.types.length]), [['land', 'Land', 4], ['air', 'Air', 0]]);
  const variants = stockGroups(REPLY, 'variant', 110);
  assert.deepEqual(variants.map(group => group.label).slice(0, 3), ['Variant 1326', 'Panzer I Ausf. A', 'Panzer II']);
  assert.ok(variants.every(group => group.types.length === 0));
  assert.equal(variantName(REPLY.variants[2], 125), 'Panzer II Ausf. F');
  assert.equal(variantName(REPLY.variants[1], 50), `Variant ${REPLY.variants[1].id}`);
  assert.deepEqual(stockGroups(REPLY, 'type', 110, ['anti_tank_equipment']).map(group => group.key), ['anti_tank_equipment']);
});

test('the balance is known for a player, marked for an AI country and unknown elsewhere', () => {
  assert.equal(balanceMode(REPLY, 110), 'player');
  assert.equal(balanceMode(REPLY, 145), 'ai');
  assert.equal(balanceMode(REPLY, 135), 'unknown');
  const [infantry, tanks, antiTank, trucks] = stockGroups(REPLY, 'type', 110);
  assert.deepEqual(balanceAt(REPLY, infantry, 110), { balance: 1549n * 100000n, stock: 145666n * 100000n, need: 144117n * 100000n, needNote: null });
  assert.deepEqual(balanceAt(REPLY, tanks, 110)?.need, 1093n * 100000n);
  assert.deepEqual(balanceAt(REPLY, antiTank, 110)?.need, 1225n * 100000n);
  assert.deepEqual(balanceAt(REPLY, trucks, 110), { balance: -40n * 100000n, stock: 5195n * 100000n, need: null, needNote: TRUCKS });
  assert.equal(balanceAt(REPLY, infantry, 145), null);
  assert.equal(balanceAt(REPLY, stockGroups(REPLY, 'variant', 110)[0], 110), null);
  const land = stockGroups(REPLY, 'domain', 110)[0];
  assert.deepEqual(balanceAt(REPLY, land, 110), { balance: (1549n - 373n - 1225n - 40n) * 100000n, stock: (145666n + 720n + 5195n) * 100000n, need: null, needNote: TRUCKS });
  const negative: EquipmentStockpileReply = { ...REPLY, logistics: [{ key: 'infantry_equipment', available: [], balance: [run(96, 130, -5)], stock: [run(96, 130, -2)] }] };
  assert.equal(balanceAt(negative, infantry, 110)?.need, 5n * 100000n);
  const saturated: EquipmentStockpileReply = { ...REPLY, logistics: [{ key: 'infantry_equipment', available: [], balance: [{ firstHour: 96, lastHour: 130, amountRaw: String(-0x53e2d620fb40) }], stock: [] }] };
  assert.equal(balanceAt(saturated, infantry, 110)?.need, null);
});

test('shares of need scale the bars, order the shortages and draw over time', () => {
  const groups = stockGroups(REPLY, 'type', 110);
  const [infantry, tanks, antiTank, trucks] = groups;
  assert.ok(Math.abs(shareOf(balanceAt(REPLY, infantry, 110))! - 1549 / 144117) < 1e-6);
  assert.equal(shareOf(balanceAt(REPLY, antiTank, 110)), -1);
  assert.equal(shareOf(balanceAt(REPLY, trucks, 110)), null);
  assert.equal(shareOf({ balance: 5n, need: 0n }), null);
  assert.equal(formatPercent(-0.3413), '−34%');
  assert.equal(formatPercent(0.0107), '+1%');
  assert.deepEqual(shareRuns(REPLY, infantry).map(item => [item.firstHour, item.lastHour, Math.round(item.value * 100) / 100, item.breakBefore]),
    [[96, 119, 1.07, true], [120, 130, Math.round(1000 / 144666 * 10000) / 100, false]]);
  assert.deepEqual(shareRuns(REPLY, tanks).map(item => [item.firstHour, item.lastHour]), [[96, 130]]);
  assert.deepEqual(shareRuns(REPLY, trucks), []);
});

test('stock and daily output work for AI countries and variants', () => {
  const [infantry, tanks, , , cas, fighters] = stockGroups(REPLY, 'type', 110);
  assert.equal(stockAt(REPLY, tanks, 110), 2116n * 100000n);
  assert.equal(stockAt(REPLY, fighters, 110), 0n);
  assert.equal(stockAt(REPLY, infantry, 135), null);
  assert.equal(dailyAt(REPLY, infantry, 110), 54190000n);
  assert.equal(dailyAt(REPLY, infantry, 126), null);
  assert.equal(stockAt(REPLY, cas, 126), 157n * 100000n);
  assert.deepEqual(stockRuns(REPLY, infantry).map(item => [item.firstHour, item.lastHour, item.value, item.breakBefore]),
    [[100, 123, 145666, true], [124, 130, 146000, false], [140, 150, 140000, true]]);
});

test('chips toggle groups', () => {
  const groups = stockGroups(REPLY, 'type', 110);
  const source = stockChoices(groups, 'equipment type');
  assert.equal(source.lines('anti_tank_equipment'), 1);
  assert.equal(source.lines('mod_rifle'), 0);
});

test('tooltips give the game’s surplus for a player, the note for an AI country and stock for variants', () => {
  const groups = stockGroups(REPLY, 'type', 110);
  const tanks = groups[1];
  const player = groupTooltip(REPLY, tanks, 'type', 110, { title: tanks.label }, 1000);
  assert.deepEqual(player.rows.map(row => [row.label, row.value]), [
    ['In stock', '720'], ['Needed', '1,093'], ['Shortage', '−373'], ['Share of need', '−34%'], ['Output per day', '15'],
    ['Panzer II', '1,396'], ['Panzer I Ausf. A', '720']]);
  const trucks = groupTooltip(REPLY, groups[3], 'type', 110, { title: 'Truck' }, 1000);
  assert.deepEqual(trucks.rows.slice(0, 3).map(row => [row.label, row.value]), [['In stock', '5,195'], ['Needed', 'Unavailable'], ['Shortage', '−40']]);
  assert.equal(trucks.note, 'Need unavailable.');
  const ai = groupTooltip(REPLY, tanks, 'type', 145, { title: tanks.label }, 1000);
  assert.deepEqual(ai.rows.slice(0, 2).map(row => row.label), ['In stock', 'Output per day']);
  assert.doesNotMatch(ai.note ?? '', /AI-controlled/);
  const unknown = groupTooltip(REPLY, tanks, 'type', 135, { title: tanks.label }, 1000);
  assert.deepEqual(unknown.rows.map(row => [row.label, row.value]).slice(0, 2), [['In stock', 'Not recorded'], ['Surplus or shortage', 'Not recorded']]);
  const variant = stockGroups(REPLY, 'variant', 110)[2];
  const byVariant = groupTooltip(REPLY, variant, 'variant', 110, { title: variant.label }, 1000);
  assert.equal(byVariant.rows[0].label, 'Light Tank');
  assert.match(byVariant.note ?? '', /Need is available by equipment type/);
  const short = groupTooltip(REPLY, tanks, 'type', 110, { title: tanks.label }, 150);
  assert.ok(short.rows.length < player.rows.length && short.rows[0].label === 'In stock' && short.spoken);
});

test('an uncomputable daily rate is unknown, never zero or a partial sum', () => {
  const costless: EquipmentStockpileReply = { ...REPLY, variants: [variant('1326', 'infantry_equipment_1', 'infantry_equipment', [run(100, 130, 5)],
    [{ firstHour: 100, lastHour: 123, amountRaw: null }]), variant('1327', 'infantry_equipment_1', 'infantry_equipment', [], [run(100, 123, 541.9)])] };
  const infantry = stockGroups(costless, 'type', 110)[0];
  assert.equal(dailyAt(costless, infantry, 110), null);
  const tip = groupTooltip(costless, infantry, 'type', 110, { title: 'Infantry' }, 1000);
  assert.deepEqual(tip.rows.find(row => row.label === 'Output per day')?.value, 'Not recorded');
});

test('a recorded zero balance shows as zero; a type with nothing but zeros adds no row', () => {
  const zeros: EquipmentStockpileReply = { ...REPLY, logistics: [...REPLY.logistics,
    { key: 'fighter_equipment', available: [], balance: [run(96, 130, 0)], stock: [run(96, 130, 0)] },
    { key: 'cas_equipment', available: [], balance: [run(96, 130, 0)], stock: [] }] };
  const groups = stockGroups(zeros, 'type', 110);
  const fighters = groups.find(group => group.key === 'fighter_equipment')!;
  assert.deepEqual(balanceAt(zeros, fighters, 110), { balance: 0n, stock: 0n, need: 0n, needNote: null });
  const cas = groups.find(group => group.key === 'cas_equipment')!;
  assert.deepEqual(cas.types, ['cas_equipment']);
  const bare: EquipmentStockpileReply = { ...REPLY, logistics: [...REPLY.logistics, { key: 'fighter_equipment', available: [], balance: [run(96, 130, 0)], stock: [] }],
    variants: REPLY.variants.filter(item => item.archetype !== 'fighter_equipment') };
  assert.ok(!stockGroups(bare, 'type', 110).some(group => group.key === 'fighter_equipment'));
});

test('a variant is labelled as recorded at the hour shown, and an unnamed one by its handle', () => {
  const renamed: EquipmentStockpileReply = { ...REPLY, variants: [variant('5', 'tank_1', 'light_tank_chassis', [run(100, 130, 1)], [],
    [{ firstHour: 0, name: 'Original', known: true }, { firstHour: 24, name: 'Renamed', known: true }]), variant('9', '', null, [run(100, 130, 1)])] };
  const groups = stockGroups(renamed, 'variant', Infinity);
  assert.equal(groups.find(group => group.key === 'c0ffee:5:tank_1')!.label, 'Renamed');
  assert.equal(labelAt(groups.find(group => group.key === 'c0ffee:5:tank_1')!, 'variant', 12), 'Original');
  assert.equal(labelAt(groups.find(group => group.key === 'c0ffee:5:tank_1')!, 'variant', 30), 'Renamed');
  assert.equal(labelAt(stockGroups(REPLY, 'type', 110)[0], 'type', 12), 'Infantry Equipment');
  assert.equal(variantName(renamed.variants[1], 110), 'Variant 9');
});

test('an arm counts every type of it for its need, whatever order the record lists them in', () => {
  const zeroTrucks = { key: 'motorized_equipment', available: [], balance: [run(96, 130, 0)], stock: [run(96, 130, 0)] };
  const others = REPLY.logistics.filter(item => item.key !== 'motorized_equipment');
  const variants = REPLY.variants.filter(item => item.archetype !== 'motorized_equipment');
  for (const logistics of [[zeroTrucks, ...others], [...others, zeroTrucks]]) {
    const reply: EquipmentStockpileReply = { ...REPLY, variants, logistics };
    const land = stockGroups(reply, 'domain', 110)[0];
    assert.ok(land.types.includes('motorized_equipment'));
    assert.equal(balanceAt(reply, land, 110)?.need, null);
    assert.ok(!stockGroups(reply, 'type', 110).some(group => group.key === 'motorized_equipment'));
  }
});

test('a variant name is the entry that applies at the hour, then a descriptive variant handle', () => {
  const named = (names: Parameters<typeof variant>[5], token = 'tank_1') => variant('5', token, 'light_tank_chassis', [], [], names);
  assert.equal(variantName(named([{ firstHour: 0, name: null, known: true }, { firstHour: 10, name: 'B', known: true }]), 5), 'Variant 5');
  assert.equal(variantName(named([{ firstHour: 0, name: null, known: true }, { firstHour: 10, name: 'B', known: true }]), 12), 'B');
  assert.equal(variantName(named([{ firstHour: 0, name: 'A', known: true }, { firstHour: 10, name: null, known: true }, { firstHour: 20, name: 'C', known: true }]), 15), 'Variant 5');
  assert.equal(variantName(named([{ firstHour: 0, name: 'A', known: true }, { firstHour: 10, name: null, known: true }, { firstHour: 20, name: 'C', known: true }]), 5), 'A');
  assert.equal(variantName(named([{ firstHour: 0, name: null, known: true }], ''), 5), 'Variant 5');
});


test('a group whose variants cannot be told has unknown stock and production, never zero', () => {
  const unresolved: EquipmentStockpileReply = { ...REPLY, types: [],
    variants: [variant('1', 'rifle_1', null, [run(100, 130, 500)], [run(100, 123, 70)])],
    logistics: [{ key: ':rifle', available: [], balance: [run(96, 130, 10)], stock: [run(96, 130, 500)] }] };
  const groups = stockGroups(unresolved, 'type', 110);
  const archetype = groups.find(group => group.key === ':rifle')!;
  const variantRow = groups.find(group => group.key === ':rifle_1')!;
  assert.equal(variantsComplete(unresolved, archetype, item => item.daily, 110), false);
  assert.equal(dailyAt(unresolved, archetype, 110), null);
  assert.equal(stockAt(unresolved, archetype, 110), null);
  assert.equal(groupTooltip(unresolved, archetype, 'type', 110, { title: 'rifle' }, 1000).rows.find(row => row.label === 'Output per day')?.value, 'Not recorded');
  assert.equal(dailyAt(unresolved, variantRow, 110), null);
  assert.equal(dailyAt(unresolved, stockGroups(unresolved, 'variant', 110)[0], 110), 7000000n);
});

test('a tiny share of need is kept exactly', () => {
  const tiny: EquipmentStockpileReply = { ...REPLY, logistics: [{ key: 'infantry_equipment', available: [], balance: [run(96, 130, 1)], stock: [run(96, 130, 2000001)] }],
    variants: REPLY.variants.filter(item => item.archetype === 'infantry_equipment') };
  const infantry = stockGroups(tiny, 'type', 110)[0];
  const logistics = balanceAt(tiny, infantry, 110)!;
  assert.equal(logistics.need, 2000000n * 100000n);
  assert.equal(shareOf(logistics), 5e-7);
  assert.equal(formatPercent(5e-7), '+0.00005%');
  assert.equal(formatPercent(-0.3413), '−34%');
  assert.ok(shareRuns(tiny, infantry).every(item => item.value > 0));
});

test('a variant of unknown type makes every type and arm total unknown, never zero or partial', () => {
  const unread: EquipmentStockpileReply = { ...REPLY,
    variants: [...REPLY.variants, variant('77', '', null, [run(100, 130, 100)], [run(100, 123, 70)])] };
  const infantry = stockGroups(unread, 'type', 110)[0];
  assert.equal(infantry.key, 'infantry_equipment');
  assert.equal(stockAt(unread, infantry, 110), null);
  assert.equal(dailyAt(unread, infantry, 110), null);
  assert.equal(groupTooltip(unread, infantry, 'type', 110, { title: 'Infantry' }, 1000).rows.find(row => row.label === 'Output per day')?.value, 'Not recorded');
  assert.equal(stockAt(unread, stockGroups(unread, 'domain', 110)[0], 110), null);
  const rows = stockGroups(unread, 'variant', 110);
  assert.equal(dailyAt(unread, rows.find(row => row.key === 'c0ffee:77:')!, 110), 7000000n);
  assert.equal(stockAt(unread, rows.find(row => row.key === 'c0ffee:1351:light_tank_chassis_1')!, 110), 72000000n);
  assert.equal(stockAt(unread, infantry, 140), 140000n * 100000n);
  const zero: EquipmentStockpileReply = { ...REPLY, variants: [...REPLY.variants, variant('78', '', null, [run(100, 130, 0)])] };
  assert.equal(stockAt(zero, stockGroups(zero, 'type', 110)[0], 110), 145666n * 100000n);
  assert.deepEqual(stockRuns(unread, infantry).map(item => [item.firstHour, item.lastHour]), [[140, 150]]);
});

test('a recorded negative stock is summed as recorded, never clamped to zero', () => {
  const negative: EquipmentStockpileReply = { ...REPLY, variants: [...REPLY.variants, variant('9', 'cas_2', 'cas_equipment', [run(100, 130, -200)])] };
  const cas = stockGroups(negative, 'type', 110).find(group => group.key === 'cas_equipment')!;
  assert.equal(stockAt(negative, cas, 110), -43n * 100000n);
});

test('the spoken tooltip lists every variant even where the eye sees eight', () => {
  const many: EquipmentStockpileReply = { ...REPLY, variants: [...REPLY.variants.filter(item => item.archetype !== 'infantry_equipment'),
    ...Array.from({ length: 10 }, (_, index) => variant(String(2000 + index), 'infantry_equipment_1', 'infantry_equipment', [run(100, 130, 100 - index)], [],
      [{ firstHour: 0, name: `Rifle ${index}`, known: true }]))] };
  const infantry = stockGroups(many, 'type', 110)[0];
  const tip = groupTooltip(many, infantry, 'type', 110, { title: 'Infantry' }, 10000);
  assert.ok(tip.rows.some(row => row.label === '2 more variants'));
  assert.ok(tip.spoken?.includes('Rifle 9') && tip.spoken.includes('91'));
});

test('an arm is unknown wherever an unresolved logistics type could belong to it', () => {
  const reply: EquipmentStockpileReply = { ...REPLY, variants: [],
    logistics: [{ key: 'light_tank_chassis', available: [], balance: [run(96, 130, -50)], stock: [run(96, 130, 10)] },
      { key: ':mod_gun', available: [], balance: [run(96, 130, -100)], stock: [run(96, 130, 20)] }] };
  const arms = stockGroups(reply, 'domain', 110);
  const land = arms.find(group => group.key === 'land')!;
  assert.equal(typesComplete(reply, land, 110), false);
  assert.equal(balanceAt(reply, land, 110), null);
  assert.equal(shareOf(balanceAt(reply, land, 110)), null);
  assert.deepEqual(shareRuns(reply, land), []);
  const tip = groupTooltip(reply, land, 'domain', 110, { title: 'Land' }, 1000);
  assert.equal(tip.rows.find(row => row.label === 'Surplus or shortage')?.value, 'Not known');
  assert.equal(tip.note, 'Total unavailable.');
  assert.equal(balanceAt(reply, arms.find(group => group.key === 'unresolved')!, 110)?.balance, -100n * 100000n);
  assert.equal(balanceAt(reply, stockGroups(reply, 'type', 110).find(group => group.key === 'light_tank_chassis')!, 110)?.balance, -50n * 100000n);
  const zero: EquipmentStockpileReply = { ...reply, logistics: [reply.logistics[0], { key: ':mod_gun', available: [], balance: [run(96, 130, 0)], stock: [run(96, 130, 0)] }] };
  assert.equal(balanceAt(zero, stockGroups(zero, 'domain', 110).find(group => group.key === 'land')!, 110)?.balance, -50n * 100000n);
});

test('an arm’s share line breaks where an unresolved type makes it unknown', () => {
  const reply: EquipmentStockpileReply = { ...REPLY, variants: [], balanceHours: [{ firstHour: 0, lastHour: 71 }], logisticsReads: [0, 24, 48],
    logistics: [{ key: 'light_tank_chassis', available: [], balance: [run(0, 71, -50)], stock: [run(0, 71, 10)] },
      { key: ':mod_gun', available: [], balance: [run(24, 47, -100)], stock: [] }] };
  const land = stockGroups(reply, 'domain', 30).find(group => group.key === 'land')!;
  assert.deepEqual(shareRuns(reply, land).map(item => [item.firstHour, item.lastHour, Math.round(item.value * 100) / 100, item.breakBefore]),
    [[0, 23, -83.33, true], [48, 71, -83.33, true]]);
});

test('log scale applies to a share chart only where every share shown is positive (the kit’s rule)', () => {
  const view = { x: null, y: null };
  assert.equal(resolveY(view as never, { yMin: null, yMax: null, log: true }, [5, 50]).log, true);
  assert.equal(resolveY(view as never, { yMin: null, yMax: null, log: true }, [-10, 50]).log, false);
  assert.ok(resolveY(view as never, { yMin: null, yMax: null, log: false }, [5, 50]).range[0] <= 0);
});

test('a modded campaign’s types, arms and default lines come from its own data', () => {
  const modded: EquipmentStockpileReply = { ...REPLY, logistics: [], balanceHours: [], logisticsReads: [], aiHours: [{ firstHour: 96, lastHour: 150 }],
    types: [
      { key: 'mod_walker_equipment', name: null, domain: 'land', needNote: null },
      { key: 'mod_hovertank_chassis', name: 'Hover Tank', domain: 'land', needNote: null },
      { key: 'mod_airship_equipment', name: 'Airship', domain: 'air', needNote: null },
    ],
    variants: [
      variant('1', 'mod_walker_equipment_1', 'mod_walker_equipment', [run(100, 130, 500)], [], [{ firstHour: 90, name: null, known: true }]),
      variant('2', 'mod_hovertank_chassis_1', 'mod_hovertank_chassis', [run(100, 130, 40)]),
      variant('3', 'mod_airship_equipment_1', 'mod_airship_equipment', [run(100, 130, 7)]),
    ] };
  const groups = stockGroups(modded, 'type', 110);
  assert.deepEqual(groups.map(group => group.key), ['mod_walker_equipment', 'mod_hovertank_chassis', 'mod_airship_equipment']);
  assert.equal(typeName(modded, 'mod_walker_equipment'), 'Equipment type name unavailable');
  assert.equal(variantName(modded.variants[0], 110), 'Variant 1');
  assert.deepEqual(stockGroups(modded, 'domain', 110).map(group => [group.key, group.variants.length]), [['land', 2], ['air', 1]]);
});

test('an arm\u2019s need is unknown while an unresolved type is listed, even at zero', () => {
  const reply: EquipmentStockpileReply = { ...REPLY, variants: [],
    logistics: [{ key: 'light_tank_chassis', available: [], balance: [run(96, 130, -50)], stock: [run(96, 130, 10)] },
      { key: ':mod_truck', available: [], balance: [run(96, 130, 0)], stock: [run(96, 130, 0)] }] };
  const land = stockGroups(reply, 'domain', 110).find(group => group.key === 'land')!;
  const logistics = balanceAt(reply, land, 110)!;
  assert.deepEqual([logistics.balance, logistics.stock, logistics.need], [-50n * 100000n, 10n * 100000n, null]);
  assert.match(logistics.needNote ?? '', /Need unavailable/);
  assert.equal(shareOf(logistics), null);
  assert.equal(balanceAt(reply, stockGroups(reply, 'type', 110).find(group => group.key === 'light_tank_chassis')!, 110)?.need, 60n * 100000n);
});

test('recorded production shows without any stockpile read', () => {
  const productionOnly: EquipmentStockpileReply = { ...REPLY, reads: [], stockHours: [], logistics: [], logisticsReads: [], balanceHours: [], aiHours: [{ firstHour: 96, lastHour: 150 }] };
  assert.equal(hasRecords(productionOnly), true);
  const infantry = stockGroups(productionOnly, 'type', 110)[0];
  assert.equal(stockAt(productionOnly, infantry, 110), null);
  assert.equal(dailyAt(productionOnly, infantry, 110), 54190000n);
  assert.equal(hasRecords({ ...productionOnly, lineReads: [], aiHours: [] }), false);
});

test('a variant\u2019s name is not known from a boundary until the next read', () => {
  const renamed = variant('5', 'tank_1', 'light_tank_chassis', [], [], [{ firstHour: 0, name: 'Old name', known: true },
    { firstHour: 10, name: null, known: false }, { firstHour: 20, name: 'New name', known: true }]);
  assert.equal(variantName(renamed, 5), 'Old name');
  assert.equal(variantName(renamed, 15), 'Variant 5');
  assert.equal(variantName(renamed, 25), 'New name');
});

test('excluded types never make a filtered arm unknown; variants of unknown type still do', () => {
  const reply: EquipmentStockpileReply = { ...REPLY, variants: [],
    logistics: [{ key: 'infantry_equipment', available: [], balance: [run(96, 130, 5)], stock: [run(96, 130, 10)] },
      { key: ':mod_gun', available: [], balance: [run(96, 130, -100)], stock: [run(96, 130, 20)] },
      { key: ':mod_truck', available: [], balance: [run(96, 130, 0)], stock: [run(96, 130, 0)] }] };
  const land = stockGroups(reply, 'domain', 110, ['infantry_equipment']).find(group => group.key === 'land')!;
  assert.deepEqual(land.scope, ['infantry_equipment']);
  assert.equal(typesComplete(reply, land, 110), true);
  assert.deepEqual(balanceAt(reply, land, 110), { balance: 5n * 100000n, stock: 10n * 100000n, need: 5n * 100000n, needNote: null });
  assert.equal(balanceAt(reply, stockGroups(reply, 'domain', 110).find(group => group.key === 'land')!, 110), null);
  assert.equal(typesComplete(reply, stockGroups(reply, 'domain', 110, ['infantry_equipment', ':mod_gun']).find(group => group.key === 'land')!, 110), false);
  const untyped: EquipmentStockpileReply = { ...REPLY, variants: [...REPLY.variants, variant('77', '', null, [run(100, 130, 100)])] };
  const rifles = stockGroups(untyped, 'type', 110, ['infantry_equipment'])[0];
  assert.equal(stockAt(untyped, rifles, 110), null);
});

test('a saved variant selects only the same design of the same campaign', () => {
  const tank = variant('1351', 'light_tank_chassis_1', 'light_tank_chassis', [run(100, 130, 1)], [], [{ firstHour: 90, name: 'Tank', known: true, design: 'd1' }]);
  const plane = variant('1351', 'fighter_equipment_1', 'fighter_equipment', [run(100, 130, 1)], [], [{ firstHour: 90, name: 'Plane', known: true, design: 'd2' }]);
  assert.equal(variantKey(REPLY, tank), 'c0ffee:1351:light_tank_chassis_1');
  const other: EquipmentStockpileReply = { ...REPLY, variants: [plane] };
  const source = stockChoices(stockGroups(other, 'variant', 110), 'variant');
  assert.equal(source.lines(choiceKey(stockGroups(REPLY, 'variant', 110)[1], 110) ?? variantKey(REPLY, tank)), 0);
  assert.equal(source.lines(choiceKey(stockGroups(other, 'variant', 110)[0], 110)!), 1);
  const campaignB: EquipmentStockpileReply = { ...REPLY, campaignKey: 'beef', variants: [tank] };
  const inB = stockChoices(stockGroups(campaignB, 'variant', 110), 'variant');
  assert.equal(inB.lines(`${variantKey(REPLY, tank)}/d1`), 0);
  assert.equal(inB.lines(choiceKey(stockGroups(campaignB, 'variant', 110)[0], 110)!), 1);
  assert.notEqual(variantKey(REPLY, tank), variantKey(campaignB, tank));
});

test('every group key a setting stores is storable, unresolved types and their arm included', () => {
  const reply: EquipmentStockpileReply = { ...REPLY, campaignKey: 'c0ffee'.repeat(5) + 'ab',
    variants: [...REPLY.variants, variant('77', 'mod_gun_1', null, [run(100, 130, 1)])],
    logistics: [...REPLY.logistics, { key: ':mod_gun', available: [], balance: [run(96, 130, -100)], stock: [run(96, 130, 20)] }] };
  for (const grouping of ['type', 'variant', 'domain'] as const) {
    const keys = stockGroups(reply, grouping, 110).map(group => group.key);
    assert.ok(keys.length > 0);
    for (const key of keys) assert.ok(storableToken(key), `${grouping} key ${key}`);
  }
  assert.ok(stockGroups(reply, 'type', 110).some(group => group.key === ':mod_gun'));
  assert.ok(stockGroups(reply, 'domain', 110).some(group => group.key === 'unresolved'));
  assert.equal(typeName(reply, ':mod_gun'), 'Equipment type name unavailable');
  const included = stockGroups(reply, 'type', 110, [':mod_gun']);
  assert.deepEqual(included.map(group => group.key), [':mod_gun']);
});

test('a saved variant choice is its design: another design under the handle is shown unavailable, never drawn', () => {
  const tank = (designs: [number, string | null][]) => variant('1351', 'light_tank_chassis_1', 'light_tank_chassis', [run(100, 130, 5)], [],
    designs.map(([firstHour, design], index) => ({ firstHour, name: `Design ${index}`, known: true, design })));
  const branchA: EquipmentStockpileReply = { ...REPLY, variants: [tank([[90, 'aaaa'], [120, 'cccc']])] };
  const rowsA = stockGroups(branchA, 'variant', 110);
  const row = rowsA[0];
  assert.equal(designAt(row.variants[0], 100), 'aaaa');
  assert.equal(designAt(row.variants[0], 125), 'cccc');
  assert.equal(designAt(row.variants[0], 10), 'aaaa');
  const chosenEarly = choiceKey(row, 100)!;
  assert.equal(chosenEarly, `${row.key}#aaaa`);
  assert.equal(choiceRow(chosenEarly), row.key);
  const inA = stockChoices(rowsA, 'variant');
  assert.equal(inA.lines(chosenEarly), 1);
  assert.equal(inA.lines(choiceKey(row, 125)!), 1);
  const branchB: EquipmentStockpileReply = { ...REPLY, variants: [tank([[90, 'bbbb']])] };
  const rowsB = stockGroups(branchB, 'variant', 110);
  assert.equal(rowsB[0].key, row.key, 'same campaign, handle and token');
  assert.equal(stockChoices(rowsB, 'variant').lines(chosenEarly), 0);
  assert.equal(stockChoices(rowsB, 'variant').lines(row.key), 0);
  const unrecorded: EquipmentStockpileReply = { ...REPLY, variants: [tank([[90, null]])] };
  const rowsU = stockGroups(unrecorded, 'variant', 110);
  assert.equal(choiceKey(rowsU[0], 110), null);
  assert.equal(stockChoices(rowsU, 'variant').lines(chosenEarly), 0);
  const types = stockGroups(REPLY, 'type', 110);
  assert.equal(choiceKey(types[0], 110), types[0].key);
});

test('native available uses the same write for supply types, including signed need and missing observations', () => {
  const row = REPLY.logistics.find(item => item.key === 'motorized_equipment')!;
  const trucks = stockGroups(REPLY, 'type', 110).find(group => group.key === 'motorized_equipment')!;
  const available = (amountRaw: string | null): EquipmentStockpileReply => ({ ...REPLY,
    logistics: REPLY.logistics.map(item => item === row ? { ...item, available: [{ firstHour: 96, lastHour: 130, amountRaw }] } : item),
  });
  const exact = balanceAt(available('300000'), trucks, 110)!;
  assert.equal(exact.need, 43n * 100000n);
  assert.equal(exact.needNote, null);
  const missing = balanceAt(available(null), trucks, 110)!;
  assert.equal(missing.need, null);
  assert.equal(missing.needNote, TRUCKS);
  const signed = { ...available('0'), logistics: [{ ...row, balance: [run(96, 130, 5)], available: [run(96, 130, 0)] }] };
  assert.equal(balanceAt(signed, trucks, 110)!.need, -5n * 100000n);
});


test('a mod convoy needs its observed pool, never the recorded production stock', () => {
  const key = 'mod_convoy';
  const reply: EquipmentStockpileReply = { ...REPLY,
    types: [{ key, name: 'Mod convoy', domain: 'naval', needNote: 'Native convoy pool observation missing.' }],
    logistics: [{ key, balance: [run(96, 130, -3)], stock: [run(96, 130, 9000)], available: [run(96, 130, 11)] }],
  };
  const group = stockGroups(reply, 'type', 110).find(group => group.key === key)!;
  assert.equal(balanceAt(reply, group, 110)!.need, 14n * 100000n);
  const missing = { ...reply, logistics: [{ ...reply.logistics[0], available: [] }] };
  assert.equal(balanceAt(missing, group, 110)!.need, null);
});


test('native signed inverse subtraction also applies to older ordinary stock', () => {
  const maximum = '9223372036854775807';
  const record = { key: 'infantry_equipment', available: [],
    balance: [{ firstHour: 96, lastHour: 130, amountRaw: '-9223372036854275809' }],
    stock: [{ firstHour: 96, lastHour: 130, amountRaw: maximum }],
  };
  const reply: EquipmentStockpileReply = { ...REPLY, logistics: [record] };
  const group = stockGroups(reply, 'type', 110).find(group => group.key === record.key)!;
  assert.equal(balanceAt(reply, group, 110)!.need, -500000n);
  const observed = { ...reply, logistics: [{ ...record, available: [{ firstHour: 96, lastHour: 130, amountRaw: maximum }] }] };
  assert.equal(balanceAt(observed, group, 110)!.need, -500000n);
});
