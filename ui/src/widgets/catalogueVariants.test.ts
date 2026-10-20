import test from 'node:test';
import assert from 'node:assert/strict';
import { widgetMetas } from './definitions.ts';
import { validOption } from '../dashboard/options.ts';
import type { WidgetMeta } from '../dashboard/definitions';

const metas: readonly WidgetMeta[] = widgetMetas;

test('every widget view a setting switches to is also its own catalogue card', () => {
  const titles = Object.fromEntries(metas.flatMap(meta => meta.variants?.length ? [[meta.kind, meta.variants.map(variant => variant.initial.title)]] : []));
  assert.deepEqual(titles, {
    research: ['Research list'],
    'political-power': ['Political power: Daily income and costs', 'Political power: Spending that day'],
    'command-power': ['Command power: Daily income and costs', 'Command power: Spending that day'],
    stability: ['Stability & War support: Stability parts', 'Stability & War support: War support parts'],
    'balance-of-power': ['Balance of power: Weekly change'],
    'resource-balance': ['Resources over time'],
    'equipment-stockpile': ['Stockpile over time'],
    fuel: ['Fuel: Fuel gained', 'Fuel: Fuel burned', 'Fuel: Individual current consumption'],
    factories: ['Factories over time', 'Factories: One country by type', 'Factories: Factory types'],
    territory: ['Territory over time', 'Territory share', 'Occupied territory', 'Occupied territory over time'],
    'equipment-losses': ['Equipment losses: Recorded losses'],
    'equipment-produced': ['Equipment produced over time', 'Equipment produced: Equipment types', 'Equipment output (IC)'],
    'dockyard-spending': ['Dockyard spending by month'],
    'occupation-trend': ['Occupation trend race', 'Occupation trend of one country'],
    'air-force-size': ['Air force size over time', 'Air force size: One country by type', 'Air force size: Aircraft types'],
    'navy-size': ['Navy size over time', 'Navy size: One country by class', 'Navy size: Ship classes'],
    'air-kills-losses': ['Air kills & losses over time', 'Aircraft loss causes'],
    'faction-strength': ['Faction strength over time', 'Faction strength: Members', 'Faction territory'],
    'trade-flows': ['Trade flows: Trade contributors', 'Market & lend-lease'],
    'fuel-branches': ['Fuel by branch over time'],
    'international-market': ['Market requests', 'Market inventory'],
    'convoy-health': ['Trade efficiency of imports'],
    'territory-transfers': ['Territory transfer log'],
    wars: ['Wars by country'],
    'world-tension': ['World tension & threat: Country contributions', 'World tension contributions'],
    'special-projects': ['Special project history'],
    'nuclear-arsenal': ['Nuclear arsenal over time', 'Nuclear arsenal: One country by nuke type', 'Nuclear arsenal: Weapon types'],
    'naval-losses': ['Naval losses over time', 'Naval losses: Ships and convoys breakdown'],
    'raid-tally': ['Raid tally over time'],
    'game-score': ['Game score race'],
    casualties: ['Losses race'],
    'casualties-between': ['Casualties between countries: Casualty pairs'],
    'casualties-breakdown': ['Casualties breakdown: Recorded losses'],
    'army-size': ['Army size over time', 'Army composition', 'Army breakdown'],
    'intel-network-coverage': ['Intel network coverage: Pair history'],
    'occupation-reserve-manpower': ['Occupation reserve manpower: Holder breakdown'],
    'weather-by-region': ['Weather by region: Temperature', 'Weather by region: Modifiers'],
    'faction-influence': ['Faction influence ranking'],
    'diplomatic-relations': ['Diplomatic relations: Pair history'],
    'diplomatic-pacts': ['Diplomatic pacts'],
    'division-equipment': ['Division equipment by template'],
    'supply-health': ['Supply health: Supply classification'],
    'land-battles': ['Biggest land battles'],
    'naval-battles': ['Biggest naval battles'],
  });
  for (const meta of metas) for (const variant of meta.variants ?? []) {
    for (const [key, value] of Object.entries(variant.initial.options ?? {})) {
      const field = meta.options?.find(item => item.key === key);
      assert.ok(field && validOption(field, value), `${variant.initial.title} sets a legal ${key}`);
      assert.notEqual(value, field.default, `${variant.initial.title} differs from ${meta.title}`);
    }
  }
});
