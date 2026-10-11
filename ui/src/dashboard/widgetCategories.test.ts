import test from 'node:test';
import assert from 'node:assert/strict';
import { catalogueEntries, matchesWidget, placedCount, widgetGroups } from './widgetCategories.ts';

const widget = (kind: string, title: string, category?: string) => ({ kind, title, description: `${title} over time.`, category });

test('the picker groups widgets by subject in the sidebar page order', () => {
  const groups = widgetGroups([
    widget('aces', 'Aces'), widget('map', 'Campaign map'), widget('research', 'Research'),
    widget('territory', 'Territory'), widget('wars', 'Wars timeline'), widget('faction-strength', 'Faction strength', 'Diplomacy'),
    widget('unplaced', 'Unplaced'), widget('mod-widget', 'Mod widget', 'Logistics'), widget('political-power', 'Political power'),
  ]);
  assert.deepEqual(groups.map(group => [group.label, group.items.map(item => item.kind)]), [
    ['Overview', ['map']],
    ['Politics', ['political-power']],
    ['Diplomacy', ['wars', 'faction-strength']],
    ['Research', ['research']],
    ['Air', ['aces']],
    ['Occupation', ['territory']],
    ['Logistics', ['mod-widget']],
    ['Other', ['unplaced']],
  ], 'a subject page wins over Overview, a declared category wins over pages, and empty groups are left out');
  assert.deepEqual(groups.slice(0, 2).map(group => [group.page, group.icon]), [['overview', 'grid'], ['politics', 'parliament']], 'page groups share the page’s icon and colour');
  assert.equal(groups.at(-2)?.page, undefined, 'a category no page names has no page');
});

test('picker search needs every word somewhere in the title, description or category', () => {
  assert.equal(matchesWidget('', 'Aces'), true);
  assert.equal(matchesWidget('  air  kills ', 'Air kills & losses', 'Air kills over time.', 'Air'), true);
  assert.equal(matchesWidget('NAVAL', 'Ships sunk log', 'Each naval loss.'), true);
  assert.equal(matchesWidget('diplomacy', 'Faction strength', undefined, 'Diplomacy'), true);
  assert.equal(matchesWidget('air navy', 'Air kills & losses'), false);
});

test('a content-derived widget becomes one card per preset, and each card counts its own copies on the page', () => {
  const groups = widgetGroups([widget('idea-timeline', 'Idea timeline'), widget('political-power', 'Political power')]);
  const presets = new Map([['idea-timeline', { reason: null, presets: [
    { key: 'laws', initial: { title: 'Laws', options: { category: 'laws' } } },
    { key: 'mod council,elite:20:seat', initial: { title: 'Extra council', options: { category: 'mod council,elite:20:seat' } } }] }]]);
  const [politics] = catalogueEntries(groups, presets);
  assert.deepEqual(politics.map(entry => [entry.key, entry.title]), [
    ['idea-timeline:laws', 'Laws'], ['idea-timeline:mod council,elite:20:seat', 'Extra council'], ['political-power', 'Political power']],
    'the generic widget gives way to its presets');
  const placed = [{ kind: 'idea-timeline', options: { category: 'laws' } }, { kind: 'idea-timeline', options: { category: 'laws' } }, { kind: 'idea-timeline', options: {} }, { kind: 'political-power' }];
  assert.deepEqual(politics.map(entry => placedCount(entry, placed)), [2, 0, 1]);
  const [loading] = catalogueEntries(groups, new Map([['idea-timeline', { presets: [], reason: 'Loading this campaign’s idea categories…' }]]));
  assert.deepEqual(loading.map(entry => [entry.key, entry.note]), [['idea-timeline', 'Loading this campaign’s idea categories…'], ['political-power', null]],
    'until its content is known the widget keeps its own card, saying why');
});
