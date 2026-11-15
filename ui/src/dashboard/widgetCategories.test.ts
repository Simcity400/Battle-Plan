import test from 'node:test';
import assert from 'node:assert/strict';
import { catalogueEntries, matchesWidget, placedCount, widgetGroups } from './widgetCategories.ts';
import type { Page } from './pages.ts';

// The test's own page table, so grouping is checked whatever the built-in pages hold.
const page = (id: string, label: string, icon: Page['icon'], ...kinds: string[]): Page => ({ id, label, icon, panels: kinds.map((kind, row) => [kind, 0, row, 12, 1]) });
const pages = [
  page('overview', 'Overview', 'grid', 'map', 'territory', 'wars'), page('politics', 'Politics', 'parliament', 'political-power', 'idea-timeline'),
  page('diplomacy', 'Diplomacy', 'handshake', 'wars'), page('research', 'Research', 'flask', 'research'),
  page('air', 'Air', 'plane', 'aces'), page('occupation', 'Occupation', 'flag', 'territory'),
];

const widget = (kind: string, title: string, category?: string) => ({ kind, title, description: `${title} over time.`, category });

test('the picker groups widgets by subject in the sidebar page order', () => {
  const groups = widgetGroups([
    widget('aces', 'Aces'), widget('map', 'Campaign map'), widget('research', 'Research'),
    widget('territory', 'Territory'), widget('wars', 'Wars timeline'), widget('faction-strength', 'Faction strength', 'Diplomacy'),
    widget('unplaced', 'Unplaced'), widget('mod-widget', 'Mod widget', 'Logistics'), widget('political-power', 'Political power'),
  ], pages);
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
  const groups = widgetGroups([widget('idea-timeline', 'Idea timeline'), widget('political-power', 'Political power')], pages);
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

test('a widget keeps its own card and lists each of its views right after it', () => {
  const view = { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'race',
    choices: [{ value: 'race', label: 'Race' }, { value: 'lines', label: 'Lines over time' }] } as const;
  const show = { key: 'show', section: 'options', label: 'Show', kind: 'choice', default: 'main',
    choices: [{ value: 'main', label: 'Country totals' }, { value: 'breakdown', label: 'Ship classes' }] } as const;
  const navy = { ...widget('navy-size', 'Navy size', 'Navy'), options: [view, show], variants: [
    { key: 'view=lines', initial: { title: 'Navy size over time', options: { view: 'lines' } } },
    // A view may spell out a value that equals its field's default; stored widgets never hold one.
    { key: 'show=breakdown', initial: { title: 'Navy size: Ship classes', options: { show: 'breakdown', view: 'race' } } }] };
  const [navyGroup] = catalogueEntries(widgetGroups([navy, widget('ships-sunk', 'Ships sunk', 'Navy')]), new Map());
  assert.deepEqual(navyGroup.map(entry => [entry.key, entry.title, entry.variant]), [
    ['navy-size', 'Navy size', false], ['navy-size:view=lines', 'Navy size over time', true],
    ['navy-size:show=breakdown', 'Navy size: Ship classes', true], ['ships-sunk', 'Ships sunk', false]]);
  assert.equal(navyGroup[0].initial, undefined, 'the own card adds the widget as it always did');
  assert.deepEqual(navyGroup[1].initial, { title: 'Navy size over time', options: { view: 'lines' } }, 'a view card adds its title and options');
  const placed = [
    { kind: 'navy-size' }, { kind: 'navy-size', options: { view: 'race', show: 'main' } }, // the default view, stored or not
    { kind: 'navy-size', options: { view: 'lines' } }, { kind: 'navy-size', options: { view: 'lines', show: 'breakdown' } },
    { kind: 'navy-size', options: { show: 'breakdown' } }, { kind: 'navy-size', options: { view: 'unknown' } }, { kind: 'ships-sunk' }];
  assert.deepEqual(navyGroup.map(entry => placedCount(entry, placed)), [3, 2, 1, 1],
    'each card counts the copies showing its view, reading a missing or illegal value as the default');

  const ideas = { ...widget('idea-timeline', 'Idea timeline'), variants: [{ key: 'view=list', initial: { title: 'Idea list', options: { view: 'list' } } }] };
  const [politics] = catalogueEntries(widgetGroups([ideas], pages), new Map([['idea-timeline', { reason: null,
    presets: [{ key: 'laws', initial: { title: 'Laws', options: { category: 'laws' } } }] }]]));
  assert.deepEqual(politics.map(entry => entry.key), ['idea-timeline:laws', 'idea-timeline:view=list'], 'content presets still replace the own card; views follow them');
});
