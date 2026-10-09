import assert from 'node:assert/strict';
import { test } from 'node:test';
import { SMOOTH_TOTALS_PRESENTATION, factionRaceRequest } from '../../charts/race/factionRaceData.ts';
import { raceSeriesIndex, seriesRaceEntries } from '../../charts/race/series.ts';
import { runAt } from '../../charts/series.ts';
import { widgetDefinition } from '../../dashboard/definitions.ts';
import { factionLabelAt, factionLines, factionMembersAt, factionTooltipForms, factionTotalsModel, factionVisibleMemberTags,
  validateFactionTotalsReply } from '../../dashboard/factionTotals.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import { defaultTemplate } from '../../dashboard/model.ts';
import { parseTemplate } from '../../dashboard/storage.ts';
import { measureChoices } from '../../dashboard/territoryMeasure.ts';
import type { FactionGroup, FactionTotalsReply, TerritoryReply } from '../../generated/dashboard';
import '../definitions.ts';
import { definition, OPTIONS } from './definition.ts';
import { factionStrengthLabel, factionStrengthNote, factionStrengthSource, MEASURES } from './factionStrengthData.ts';

const firstHour = 43_800_000;
const lastHour = firstHour + 48;
const span = { firstHour, lastHour };
const reply = (): FactionTotalsReply => ({ branchId: 'branch', membershipMode: 'hour', unit: 'factories', unavailableReason: null, warnings: [], recorded: [span], disjoint: [span],
  series: [{ id: 'mod:pact', unavailableReason: null, dissolutionHour: null, membership: [span], names: [
    { firstHour, lastHour: firstHour + 23, name: 'Recorded pact', reason: null }, { firstHour: firstHour + 24, lastHour, name: 'Renamed pact', reason: null }],
    runs: [{ firstHour, lastHour: firstHour + 23, value: 17, startsSegment: true },
      { firstHour: firstHour + 25, lastHour, value: 0, startsSegment: true }],
    countedMembers: [{ tag: 'Mod:A', ...span }, { tag: 'Mod:B', firstHour, lastHour: firstHour + 23 }],
    members: [{ tag: 'Mod:A', ...span }, { tag: 'Mod:B', firstHour, lastHour: firstHour + 23 }] }] });

test('Diplomacy registers an uncapped faction race, with only Factories and Territory and hourly members by default', () => {
  const options = resolveOptions(OPTIONS);
  assert.equal(widgetDefinition('faction-strength')?.title, 'Faction strength');
  assert.equal(definition.category, 'Diplomacy');
  assert.equal(definition.countries.max, Number.MAX_SAFE_INTEGER);
  assert.deepEqual(definition.defaultCountries, { kind: 'factions', count: Number.MAX_SAFE_INTEGER, membership: 'hour' });
  assert.deepEqual(MEASURES.map(item => item.value), ['factories', 'territory']);
  const measure = OPTIONS.find(option => option.key === 'measure')!;
  assert.deepEqual(measure.kind === 'choice' && measure.choices.map(item => item.value), ['factories', 'territory']);
  const view = OPTIONS.find(option => option.key === 'view')!;
  assert.deepEqual(view.kind === 'choice' && view.choices.map(item => item.value), ['race', 'lines']);
  assert.equal(options.view, 'race');
  assert.equal(options.measure, 'factories');
  assert.equal(options.count, 'free');
  assert.equal(definition.defaultTime, 'campaign');
  assert.equal(options.dateOverlay, false);
  assert.equal(SMOOTH_TOTALS_PRESENTATION.exact, false);
  assert.equal(SMOOTH_TOTALS_PRESENTATION.held, false);
});

test('both measures use the existing faction-total adapters and preserve all three membership modes', () => {
  const defaults = resolveOptions(OPTIONS);
  for (const source of [factionStrengthSource(defaults), factionStrengthSource({ ...defaults, measure: 'territory' })]) {
    for (const membership of ['hour', 'end', 'ever'] as const) {
      const selection = { ...definition.defaultCountries, membership };
      assert.deepEqual(factionRaceRequest('branch', selection, source, 50, definition.countries.max),
        { branchId: 'branch', source, membership, top: 50 });
      const value = { hour: 17, end: 22, ever: 30 }[membership];
      const data = reply();
      data.membershipMode = membership;
      data.unit = source.kind === 'factories' ? 'factories' : 'states';
      data.series[0].runs[0].value = value;
      const counted = membership === 'hour' ? ['Mod:A'] : membership === 'end' ? ['Mod:B'] : ['Mod:A', 'Mod:B'];
      data.series[0].countedMembers = counted.map(tag => ({ tag, ...span }));
      data.series[0].members = [{ tag: 'Mod:A', ...span }];
      const model = factionTotalsModel(validateFactionTotalsReply(data));
      const series = raceSeriesIndex(factionLines(model, span));
      assert.equal(seriesRaceEntries(series, firstHour + 10.9, 50, String)[0].label, String(value));
      const forms = factionTooltipForms(model, ['mod:pact'], firstHour + 10, tag => ({ name: `${tag} at hovered hour` }),
        () => '#888888', String, 'Faction strength', factionStrengthLabel(source, data.unit));
      assert.equal(forms[0].rows[0].value, String(value));
      const memberLabel = { hour: 'Members at this hour', end: 'Counted: members at the end', ever: 'Counted: anyone ever a member' }[membership];
      assert.deepEqual(factionVisibleMemberTags(model, firstHour + 10, ['mod:pact']), counted);
      for (const form of forms) {
        assert.equal(form.rows[1].label, memberLabel);
        assert.equal(form.rows[1].value, String(counted.length));
        assert.ok(form.spoken?.includes(`${memberLabel} (${counted.length})`));
        for (const tag of counted) assert.ok(form.spoken?.includes(`${tag} at hovered hour`));
        if (membership === 'end') assert.ok(!form.spoken?.includes('Mod:A at hovered hour'));
      }
    }
  }
  assert.deepEqual(factionStrengthSource(defaults), { kind: 'factories', mode: 'free', groups: [] });
  assert.deepEqual(factionStrengthSource({ ...defaults, measure: 'territory' }), { kind: 'territory', measure: 'states', rule: 'controlled' });
});

test('the all-factions default and every membership mode survive layout storage without a private count ceiling', () => {
  const template = defaultTemplate();
  template.widgets[0] = { ...template.widgets[0], kind: definition.kind, countries: definition.defaultCountries };
  for (const membership of ['hour', 'end', 'ever'] as const) {
    template.widgets[0].countries = { ...definition.defaultCountries, membership };
    assert.deepEqual(parseTemplate(template)?.widgets[0].countries, template.widgets[0].countries);
  }
});

test('factory formulas and every Territory sub-measure/rule retain the shared labels and area availability', () => {
  for (const [mode, label] of [['free', 'Free after withheld'], ['built', 'Built'], ['topBar', 'Top bar']] as const) {
    const source = factionStrengthSource({ measure: 'factories', count: mode });
    assert.deepEqual(source, { kind: 'factories', mode, groups: [] });
    assert.equal(factionStrengthLabel(source, 'factories'), `${label} factories`);
    assert.equal(factionStrengthNote(source), undefined);
  }
  for (const [measure, label] of [['states', 'states'], ['provinces', 'provinces'], ['victoryPoints', 'victory points'], ['area', 'map area · map cells']] as const) {
    for (const rule of ['controlled', 'owned'] as const) {
      const source = factionStrengthSource({ measure: 'territory', territoryMeasure: measure, rule });
      assert.deepEqual(source, { kind: 'territory', measure, rule });
      assert.equal(factionStrengthLabel(source, 'map cells'), `${rule === 'owned' ? 'Owned' : 'Controlled'} ${label}`);
      assert.equal(Boolean(factionStrengthNote(source)), measure === 'area');
    }
  }
  assert.deepEqual(measureChoices(undefined).map(item => item.value), ['states', 'provinces', 'victoryPoints']);
  assert.equal(measureChoices({ areaUnit: 'map cells' } as TerritoryReply).at(-1)?.value, 'area');
  const factories = resolveOptions(OPTIONS);
  const territory = resolveOptions(OPTIONS, { measure: 'territory' });
  assert.equal(OPTIONS.find(option => option.key === 'territoryMeasure')?.visibleWhen?.(factories), false);
  assert.equal(OPTIONS.find(option => option.key === 'territoryMeasure')?.visibleWhen?.(territory), true);
});

test('exact hourly values, rosters and dated names survive gaps; end-member dissolution labels use the shared helper', () => {
  const data = reply();
  data.membershipMode = 'end';
  data.series[0].countedMembers = [{ tag: 'Mod:B', ...span }];
  data.series[0].dissolutionHour = firstHour + 48;
  const model = factionTotalsModel(validateFactionTotalsReply(data));
  const runs = factionLines(model, span)[0].runs;
  assert.equal(runAt(runs, firstHour + 24), null);
  assert.equal(runAt(runs, firstHour + 25)?.value, 0);
  assert.equal(runAt(runs, firstHour + 25)?.breakBefore, true);
  assert.deepEqual(factionMembersAt(model, 'mod:pact', firstHour + 25), ['Mod:B']);
  const note = 'last members before it dissolved on 3 Jan 0 · 01:00';
  assert.equal(factionLabelAt(model, 'mod:pact', firstHour + 25), `Renamed pact · ${note}`);
  const forms = factionTooltipForms(model, ['mod:pact'], firstHour + 25, tag => ({ name: `${tag} at hour 25` }),
    () => '#888888', String, 'Faction strength', 'Controlled states');
  for (const form of forms) {
    assert.equal(form.rows[0].label, 'Renamed pact');
    assert.equal(form.rows[0].value, '0');
    assert.ok(form.rows.some(row => row.label === note));
    assert.ok(form.spoken?.includes(note));
    assert.ok(form.spoken?.includes('Counted: members at the end (1): Mod:B at hour 25'));
  }
  const settings = { factions: new Map([['mod:pact', { name: 'old name' } as FactionGroup]]), names: () => ({ name: 'Campaign edit', reason: null }) };
  assert.equal(factionLabelAt(model, 'mod:pact', firstHour + 25, settings), `Campaign edit · ${note}`);
});

function scaleReply(): FactionTotalsReply {
  const data = reply();
  data.series = Array.from({ length: 50 }, (_, faction) => ({ id: `mod:faction:${faction}`, dissolutionHour: null, unavailableReason: null,
    runs: [{ ...span, value: 100, startsSegment: true }], names: [{ ...span, name: `Mod pact ${faction}`, reason: null }], membership: [span],
    countedMembers: Array.from({ length: 100 }, (_, member) => ({ tag: `Mod:${faction}:${member}`, ...span })),
    members: Array.from({ length: 100 }, (_, member) => ({ tag: `Mod:${faction}:${member}`, ...span })) }));
  return data;
}

test('both measures retain all 5,000 counted countries and 50 exact faction totals under every membership mode', () => {
  for (const unit of ['factories', 'states']) for (const membershipMode of ['hour', 'end', 'ever'] as const) {
    const data = { ...scaleReply(), unit, membershipMode };
    const model = factionTotalsModel(validateFactionTotalsReply(data));
    const lines = factionLines(model, span);
    const indexed = raceSeriesIndex(lines);
    const entries = seriesRaceEntries(indexed, firstHour, data.series.length, String);
    assert.equal(entries.length, 50);
    assert.ok(entries.every(entry => entry.label === '100'));
    const ids = entries.map(entry => entry.tag);
    const members = factionVisibleMemberTags(model, firstHour, ids);
    const forms = factionTooltipForms(model, ids, firstHour, tag => ({ name: tag }), () => '#888888', String,
      'Faction strength', unit);
    assert.equal(members.length, 5000);
    assert.equal(new Set(members).size, 5000);
    assert.equal(forms[0].rows.filter(row => row.icon == null && row.indent && row.short && !row.value).length, 5000);
    for (const form of forms) assert.deepEqual(new Set(form.spoken?.match(/Mod:\d+:\d+/g)), new Set(members));
  }
});
