import test from 'node:test';
import assert from 'node:assert/strict';
import { fixed5Of, fixed5Text, formatModifier, ideaEffectRows, ideaTooltip, ideaTooltipSources, ideaTargetTags, tooltipText } from './ideaTooltip.ts';
import type { IdeaInfo, ModifierFormat } from '../generated/dashboard';

const format = (key: string, index: number, flags: number, precision: number, label: string, postfix: string | null = null): ModifierFormat =>
  ({ key, index, flags, precision, label, postfix, postfixKey: postfix ? 'MOD_DAILY' : null, feature: 0, source: 'recorded' });
const STABILITY = format('stability_factor', 0x8d, 3, 2, 'Stability');
const CONSUMER = format('consumer_goods_factor', 0x66, 0xd, 1, 'Consumer Goods Factories');
const POWER = format('political_power_gain', 0x25, 2, 2, 'Political Power Gain');
const COMMAND = format('max_command_power', 0x14b, 2, 0, 'Max Command Power');
const EXPERIENCE = format('experience_gain_army', 0x2e, 2, 2, 'Army Experience Gain', 'daily');
const DISABLED = format('disabled_ideas', 0x22a, 0x20, 2, 'Ideas Disabled');
const EXPORT = format('min_export', 0xe0, 0x105, 0, 'Resources to Market');
const WORDS = { yes: 'Yes', no: 'No' };
const shown = (definition: ModifierFormat, value: number) => formatModifier(definition, fixed5Of(value), WORDS);

test('values are written as the game writes them: truncated, at the lower precision, with kept zeros', () => {
  assert.equal(fixed5Text(500_000n, 2), '5.00');
  assert.equal(fixed5Text(12_900n, 2), '0.12');
  assert.equal(fixed5Text(-3_300_000n, 1), '-33.0');
  assert.equal(fixed5Text(1_000_000n, 0), '10');
  assert.equal(fixed5Text(-50_000n, 0), '-0');
  assert.equal(fixed5Text(150_000n, -1), '1.5');
});

test('each flag changes the value as the game shows it', () => {
  assert.deepEqual(shown(STABILITY, 0.05), { text: '+5.00%', tone: 'positive' });
  assert.deepEqual(shown(STABILITY, -0.005), { text: '-0.50%', tone: 'negative' });
  assert.deepEqual(shown(STABILITY, 0.00129), { text: '+0.12%', tone: 'positive' });
  assert.deepEqual(shown(CONSUMER, 0.4), { text: '40.0%', tone: 'neutral' });
  assert.deepEqual(shown(CONSUMER, -0.33), { text: '-33.0%', tone: 'neutral' });
  assert.deepEqual(shown(POWER, 0.2), { text: '+0.20', tone: 'positive' });
  assert.deepEqual(shown(COMMAND, 10), { text: '+10', tone: 'positive' });
  assert.deepEqual(shown(EXPERIENCE, 0.1), { text: '+0.10 daily', tone: 'positive' });
  assert.deepEqual(shown(DISABLED, 1), { text: 'Yes', tone: 'negative' });
  assert.deepEqual(shown(EXPORT, 0), { text: '0%', tone: 'neutral' });
});

const idea = (extra: Partial<IdeaInfo>): IdeaInfo => ({ token: 'idea', category: 'country', slot: null, law: false, designer: false,
  effects: [], targeted: [], equipment: [], research: [], traits: [], tooltips: [], effectsKnown: true, unknown: null, nameMissing: null, ...extra });
const line = (key: string, value: number, hidden = false) => ({ key, fixed5: String(fixed5Of(value)), hidden });
const sources = ideaTooltipSources({
  modifiers: [STABILITY, POWER, EXPORT, CONSUMER],
  texts: [{ key: 'YES', text: 'Yes' }, { key: 'NO', text: 'No' }, { key: 'industry_research', text: 'Industry' },
    { key: 'small_plane_airframe', text: 'Small Airframe' }, { key: 'MY_TOOLTIP', text: 'Something happens' }],
}, tag => ({ POL: 'Poland' })[tag] ?? tag);

test('an idea lists its effects in key order, hides what the game hides, and marks what it cannot format', () => {
  const { rows, note } = ideaEffectRows(idea({
    effects: [line('stability_factor', 0), line('political_power_gain', 0.2), line('min_export', 0),
      line('consumer_goods_factor', 0, true), line('mod_key', 0.3)],
    tooltips: ['MY_TOOLTIP'],
    targeted: [{ target: 'POL', instant: false, lines: [line('stability_factor', 0.5)], tooltips: ['MY_TOOLTIP'] }],
    research: [line('industry', 0.1)],
    equipment: [{ target: 'small_plane_airframe', instant: false, lines: [line('build_cost_ic', -0.05)], tooltips: [] }],
  }), sources);
  assert.deepEqual(rows.map(row => [row.label, row.value ?? null, row.tone ?? null]), [
    ['Political Power Gain:', '+0.20', 'positive'],
    ['Resources to Market:', '0%', 'neutral'],
    ['Modifier name unavailable:', '0.3', 'muted'],
    ['Something happens', null, null],
    ['Against Poland:', null, null],
    ['Stability:', '+50.00%', 'positive'],
    ['Something happens', null, null],
    ['Industry:', '+10%', 'positive'],
    ['Small Airframe:', null, null],
    ['Equipment modifier name unavailable:', '-0.05', 'muted'],
  ]);
  assert.equal(note, undefined);
});

test('traits come first, and a trait content does not define shows only its name', () => {
  const { rows } = ideaEffectRows(idea({ traits: [
    { token: 'captain_of_industry', known: true, unknown: null, name: 'Captain of Industry', lines: [line('political_power_gain', 0.1)], targeted: [], equipment: [], tooltips: [] },
    { token: 'mystery', known: false, unknown: null, name: null, lines: [], targeted: [], equipment: [], tooltips: [] },
  ], effects: [line('stability_factor', 0.02)] }), sources);
  assert.deepEqual(rows.map(row => row.label), ['Captain of Industry', 'Political Power Gain:', 'Trait name unavailable', 'Stability:']);
  assert.ok(rows[0].strong && rows[0].divider);
});

test('a trait keeps its targeted modifiers apart and shows its equipment bonuses', () => {
  const { rows } = ideaEffectRows(idea({ traits: [{ token: 'designer', known: true, unknown: null, name: 'Designer', lines: [], tooltips: [],
    targeted: [{ target: 'POL', instant: false, lines: [line('stability_factor', 0.1)], tooltips: [] }, { target: 'AAA', instant: false, lines: [line('stability_factor', 0.2)], tooltips: [] }],
    equipment: [{ target: 'small_plane_airframe', instant: false, lines: [line('armor_value', 0.05)], tooltips: [] }] }] }), sources);
  assert.deepEqual(rows.map(row => [row.label, row.value ?? null]), [
    ['Designer', null], ['Against Poland:', null], ['Stability:', '+10.00%'], ['Against AAA:', null], ['Stability:', '+20.00%'],
    ['Small Airframe:', null], ['Equipment modifier name unavailable:', '0.05'],
  ]);
});

test('a line the game shows at zero stays, with an unavailable label and its key on hover when it has no name', () => {
  const unnamed = ideaTooltipSources({ modifiers: [{ ...EXPORT, label: null }], texts: [] }, tag => tag);
  const { rows, note } = ideaEffectRows(idea({ effects: [line('min_export', 0)] }), unnamed);
  assert.deepEqual(rows.map(row => [row.label, row.value]), [['Modifier name unavailable:', '0%']]);
  assert.equal(rows[0].lookupKey, 'min_export');
  assert.equal(note, undefined);
});

test('an authored zero without a format stays, and targets are named as they were when taken on', () => {
  const { rows } = ideaEffectRows(idea({ effects: [line('mod_key', 0)] }), sources);
  assert.deepEqual(rows.map(row => [row.label, row.value]), [['Modifier name unavailable:', '0']]);
  const exactRow = ideaEffectRows(idea({ effects: [{ key: 'mod_key', fixed5: '9999', hidden: false }] }), sources);
  assert.equal(exactRow.rows[0].value, '0.09999');
  const dated = ideaTooltipSources({ modifiers: [STABILITY], texts: [] }, tag => tag, (tag, hour) => `${tag} in ${hour}`);
  const tip = ideaTooltip({ title: 'Idea' }, idea({ targeted: [{ target: 'POL', instant: false, lines: [line('stability_factor', 0.1)], tooltips: [] }] }), dated, null, [], undefined, 500);
  assert.equal(tip.rows[0].label, 'Against POL in 500:');
});

test('a long tooltip shows its first rows and counts the rest; its text keeps them all', () => {
  const many = idea({ effects: Array.from({ length: 50 }, (_, index) => line(`mod_${String(index).padStart(2, '0')}`, 1)) });
  const tip = ideaTooltip({ title: 'Idea' }, many, sources, 'A description.', [], undefined, undefined, 10);
  assert.equal(tip.rows.length, 8);
  assert.equal(tip.rows[7].label, '43 more effects');
  const text = tooltipText(ideaTooltip({ title: 'Idea' }, many, sources, 'A description.', [], undefined, undefined, Infinity));
  assert.ok(text.includes('Modifier name unavailable: 1') && text.includes('A description.'));
});

test('script values keep five decimals of the authored text, truncated', () => {
  assert.equal(fixed5Of(0.099999), 9_999n);
  assert.equal(fixed5Of(0.29), 29_000n);
  assert.equal(fixed5Of(-0.1234567), -12_345n);
  assert.equal(fixed5Of(12), 1_200_000n);
  assert.deepEqual(formatModifier(STABILITY, fixed5Of(0.099999), WORDS), { text: '+9.99%', tone: 'positive' });
});


test('the row budget counts wrapped labels, and the exact authored value wins', () => {
  const long = idea({ effects: Array.from({ length: 10 }, (_, index) => line(`a_very_long_modifier_key_that_wraps_twice_${index}_over_the_width`, 1)) });
  const labelled = ideaTooltipSources({ modifiers: long.effects.map((line, index) => ({ ...POWER, key: line.key, index, label: line.key })), texts: [] }, tag => tag);
  const tip = ideaTooltip({ title: 'Idea' }, long, labelled, null, [], undefined, undefined, 13);
  assert.equal(tip.rows.length, 4);
  assert.equal(tip.rows[3].label, '7 more effects');
  const exact = ideaEffectRows(idea({ effects: [{ key: 'stability_factor', fixed5: '9999', hidden: false }] }), sources);
  assert.equal(exact.rows[0].value, '+9.99%');
});

test('a unit whose text is missing is marked unavailable while the value remains visible', () => {
  const noUnitText = ideaTooltipSources({ modifiers: [{ ...EXPERIENCE, postfix: null, postfixKey: 'MOD_DAILY' }], texts: [] }, tag => tag);
  const { rows, note } = ideaEffectRows(idea({ effects: [line('experience_gain_army', 0.1)] }), noUnitText);
  assert.deepEqual(rows.map(row => [row.value, row.tone]), [['+0.10 [unit unavailable]', 'muted']]);
  assert.equal(note, undefined);
});

test('a long title leaves fewer rows for effects', () => {
  const many = idea({ effects: Array.from({ length: 20 }, (_, index) => line(`mod_${String(index).padStart(2, '0')}`, 1)) });
  const short = ideaTooltip({ title: 'Idea' }, many, sources, null, [], undefined, undefined, 25);
  const long = ideaTooltip({ title: 'A title that goes on and on '.repeat(10) }, many, sources, null, [], undefined, undefined, 25);
  assert.ok(long.rows.length < short.rows.length);
  assert.match(long.rows.at(-1)!.label, /more effects$/);
});

test('the engine clamps, hides every hidden key, and marks modifiers that need a game feature', () => {
  const clamped = format('clamped', 0x50, 0x11, 2, 'Clamped');
  const gated = { ...format('agency_upgrade_time', 0x60, 1, 2, 'Agency upgrade time'), feature: 0x20 };
  const engine = ideaTooltipSources({ modifiers: [clamped, gated, STABILITY], texts: [] }, tag => tag);
  const { rows, note } = ideaEffectRows(idea({ effects: [line('clamped', 1.5), line('agency_upgrade_time', -0.1),
    line('stability_factor', 0.2, true)] }), engine);
  assert.deepEqual(rows.map(row => [row.label, row.value, row.tone]), [['Clamped:', '+100.00%', 'negative'], ['Agency upgrade time:', '-10.00% *', 'muted']]);
  assert.equal(note, '* Depends on an enabled game feature.');
  assert.equal(ideaEffectRows(idea({ effects: [line('clamped', -0.5)] }), engine).rows.length, 0);
});

test('effects the content does not define are left out of the tooltip, with no note', () => {
  assert.deepEqual(ideaEffectRows(idea({ effectsKnown: false }), sources).rows, []);
  const gap = 'common/ideas/b.txt is unknown from line 3: x, so what it defines there is unknown';
  assert.deepEqual(ideaEffectRows(idea({ effectsKnown: false, unknown: gap }), sources).rows, [], 'the reason is not shown either');
});

test('dated idea targets include distinct country and trait targets without equipment type names', () => {
  const group = (target: string | null) => ({ target, instant: false, lines: [], tooltips: [] });
  const info = idea({ targeted: [group('opaque | A'), group(null)], equipment: [group('equipment type')],
    traits: [{ token: 'mod_trait', known: true, unknown: null, name: null, lines: [], tooltips: [], equipment: [],
      targeted: [group('opaque | A'), group(''), group('opaque | B')] }] });
  assert.deepEqual(ideaTargetTags(info), ['opaque | A', '', 'opaque | B']);
  assert.deepEqual(ideaTargetTags(null), []);
  assert.deepEqual(ideaTargetTags(undefined), []);
});
