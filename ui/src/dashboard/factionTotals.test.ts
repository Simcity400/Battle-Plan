import test from 'node:test';
import assert from 'node:assert/strict';
import type { FactionGroup, FactionTotalsReply } from '../generated/dashboard';
import { factionLabelAt, factionLines, factionMembershipNote, factionMemberLabelLimit, factionMembersAt, factionNameAt, factionNameReasonAt, factionTooltipForms, factionTotalsModel, factionTotalsRequest, factionVisibleMemberTags, validateFactionTotalsReply } from './factionTotals.ts';
import { factionNameAt as recordedNameAt } from './factionNames.ts';
import { countrySelectionFor, countrySelectionMode, countrySelectionModes } from './groups.ts';
import { defaultTemplate } from './model.ts';
import { parseTemplate } from './storage.ts';
import { selectedTags } from './countrySelection.ts';
import { runAt } from '../charts/series.ts';
import { fitTooltip } from '../charts/tooltip.ts';
import { countryRaceTooltipTags } from '../charts/race/countryRace.ts';
import { definition as territoryDefinition } from '../widgets/territory/definition.ts';
import '../widgets/definitions.ts';

const reply = (): FactionTotalsReply => ({ branchId: 'branch', membershipMode: 'hour', warnings: [], unit: 'states', unavailableReason: null, recorded: [], disjoint: [], series: [{
  id: '42', dissolutionHour: null, unavailableReason: null, runs: [
    { firstHour: 11, lastHour: 12, value: 8, startsSegment: true },
    { firstHour: 14, lastHour: 16, value: 5, startsSegment: true },
  ], names: [{ firstHour: 11, lastHour: 13, name: 'First name', reason: null }, { firstHour: 14, lastHour: 16, name: 'Renamed faction', reason: null }],
  countedMembers: [{ tag: 'Opaque/A', firstHour: 11, lastHour: 16 }, { tag: 'New/B', firstHour: 14, lastHour: 16 }],
  members: [{ tag: 'Opaque/A', firstHour: 11, lastHour: 16 }, { tag: 'New/B', firstHour: 14, lastHour: 16 }],
  membership: [{ firstHour: 11, lastHour: 12 }, { firstHour: 15, lastHour: 16 }],
}] });

test('Factions is a shared opt-in country choice with the hourly top-10 default', () => {
  assert.deepEqual(countrySelectionModes({ factions: true }).map(choice => choice.value), ['countries', 'group', 'factions']);
  assert.equal(countrySelectionModes({}).some(choice => choice.value === 'factions'), false);
  const selection = countrySelectionFor('factions', 30);
  assert.deepEqual(selection, { kind: 'factions', count: 10, membership: 'hour' });
  assert.equal(countrySelectionMode(selection), 'factions');
  assert.deepEqual(selectedTags(selection, undefined), []);
  assert.deepEqual(countrySelectionFor('factions', 3), { kind: 'factions', count: 3, membership: 'hour' });
});

test('faction selections and all membership rules survive layout storage, with invalid values refused', () => {
  const template = defaultTemplate();
  for (const membership of ['hour', 'end', 'ever'] as const) {
    template.widgets[0].countries = { kind: 'factions', count: 7, membership };
    assert.deepEqual(parseTemplate(template)?.widgets[0].countries, template.widgets[0].countries);
  }
  for (const invalid of [{ count: 0 }, { count: Number.MAX_SAFE_INTEGER + 1 }, { count: 2.5 }, { membership: 'stale' }]) {
    Object.assign(template.widgets[0], { countries: { kind: 'factions', count: 10, membership: 'hour', ...invalid } });
    assert.equal(parseTemplate(template), null);
  }
});

test('one request adapter wires different per-country measures to shared faction totals', () => {
  const selection = { kind: 'factions', count: 50, membership: 'ever' } as const;
  const territory = { kind: 'territory', measure: 'victoryPoints', rule: 'owned' } as const;
  assert.deepEqual(factionTotalsRequest('branch', selection, territory, 30), { branchId: 'branch', source: territory, membership: 'ever', top: 30 });
});

test('a faction request keeps a data-sized selection beyond the former thirty-faction ceiling', () => {
  const selection = { kind: 'factions', count: 5000, membership: 'end' } as const;
  const source = { kind: 'territory', measure: 'states', rule: 'controlled' } as const;
  assert.equal(factionTotalsRequest('branch', selection, source, territoryDefinition.countries.max)?.top, 5000);
});

test('a summed faction group requests that faction id under the same membership rule', () => {
  const source = { kind: 'territory', measure: 'states', rule: 'controlled' } as const;
  for (const membership of ['hour', 'end', 'ever'] as const) {
    const selection = { kind: 'group', group: { type: 'faction', id: '42' }, aggregate: true, membership } as const;
    assert.deepEqual(factionTotalsRequest('branch', selection, source, 30), { branchId: 'branch', source, membership, top: 1, factionId: '42' });
  }
  assert.equal(factionTotalsRequest('branch', { kind: 'group', group: { type: 'humans' }, aggregate: true, membership: 'hour' }, source, 30), null);
});

test('faction display names use campaign edits, else the recorded name at the hovered hour', () => {
  const model = factionTotalsModel(reply());
  const faction: FactionGroup = { id: '42', name: 'Renamed faction', recordedName: 'first-name-key', firstHour: 11, lastHour: 16,
    nameReason: null, names: reply().series[0].names, members: [], leaders: [], incomplete: [] };
  const factions = new Map([[faction.id, faction]]);
  const recorded = { factions, names: recordedNameAt };
  const edited = { factions, names: () => ({ name: 'User display name', reason: null }) };
  assert.equal(factionNameAt(model, '42', 11, recorded), 'First name');
  assert.equal(factionNameAt(model, '42', 15, recorded), 'Renamed faction');
  assert.equal(factionNameAt(model, '42', 11, edited), 'User display name');
  const forms = factionTooltipForms(model, ['42'], 15, tag => ({ name: tag }), () => '#888888', String, 'Date', 'Factories', undefined, 48, edited);
  assert.equal(forms[0].rows[0].label, 'User display name');
});

test('the shared reply accepts a measure unit and refusal, without Territory area fields', () => {
  const data = { ...reply(), unit: 'factories', unavailableReason: null };
  assert.equal(validateFactionTotalsReply(data), data);
  assert.throws(() => validateFactionTotalsReply({ ...data, unit: 42 }), /malformed/);
});

test('ranked factions and a summed faction group shape the same server totals, including zero and existence gaps', () => {
  const data = reply();
  data.series[0].runs = [{ firstHour: 11, lastHour: 16, value: 0, startsSegment: true }];
  data.series[0].membership = [{ firstHour: 11, lastHour: 16 }];
  const range = { firstHour: 0, lastHour: 20 };
  const ranked = factionLines(factionTotalsModel(data), range);
  const selected = factionLines(factionTotalsModel({ ...data, series: data.series.filter(series => series.id === '42') }), range);
  assert.deepEqual(selected, ranked);
  assert.equal(runAt(selected[0].runs, 10), null);
  assert.equal(runAt(selected[0].runs, 12)?.value, 0);
  assert.equal(runAt(selected[0].runs, 17), null);
});

test('an unavailable faction total hides its raw gap reason and retains membership scope', () => {
  for (const [membershipMode, scope] of [['hour', 'Members at this hour'], ['end', 'Members at the end'], ['ever', 'Anyone ever a member']] as const) {
    const data = reply();
    data.membershipMode = membershipMode;
    data.series[0].runs = [];
    data.series[0].unavailableReason = "This faction's membership was not recorded at the end of this campaign branch";
    const forms = factionTooltipForms(factionTotalsModel(data), ['42'], 15, tag => ({ name: tag }), () => '#888888', String, 'Faction factories', 'Date');
    for (const form of forms) {
      assert.equal(form.rows[0].value, 'Total unavailable');
      assert.equal(form.rows[1].label, scope);
      assert.ok(!form.rows.some(row => row.label === data.series[0].unavailableReason));
      assert.ok(!form.spoken?.includes(data.series[0].unavailableReason!));
    }
  }
});

test('faction member previews reserve room for the supplied measure tooltip title', () => {
  assert.ok(factionMemberLabelLimit(800, 1, 'Faction factories') > factionMemberLabelLimit(800, 1, 'A very long measure title '.repeat(30)));
});

test('faction lines preserve gaps, identity breaks and top-N hours while names and rosters stay dated', () => {
  const model = factionTotalsModel(validateFactionTotalsReply(reply()));
  const lines = factionLines(model, { firstHour: 10, lastHour: 20 });
  assert.deepEqual(lines, [{ tag: '42', runs: [
    { firstHour: 11, lastHour: 12, value: 8, breakBefore: true },
    { firstHour: 15, lastHour: 16, value: 5 },
  ] }]);
  assert.equal(runAt(lines[0].runs, 13), null);
  assert.equal(runAt(lines[0].runs, 14), null);
  assert.equal(factionNameAt(model, '42', 13), 'First name');
  assert.equal(factionNameAt(model, '42', 14), 'Renamed faction');
  assert.equal(factionNameAt(model, '42', 10), 'Unnamed faction');
  assert.deepEqual(factionMembersAt(model, '42', 13), ['Opaque/A']);
  assert.deepEqual(factionMembersAt(model, '42', 14), ['New/B', 'Opaque/A']);
  assert.deepEqual(factionMembersAt(model, '42', 17), []);
  assert.deepEqual(factionVisibleMemberTags(model, 14, ['42', '42']), ['New/B', 'Opaque/A']);
});

test('faction hover names members at its own hour and retains exact totals in the compact form', () => {
  const model = factionTotalsModel(reply());
  const forms = factionTooltipForms(model, ['42'], 15, tag => ({ name: `${tag} at hour 15`, flag: 'flag-at-15' }),
    () => '#888888', value => String(value), 'Date', 'Measure');
  assert.deepEqual(forms[0].rows.map(row => row.label), ['Renamed faction', 'Members at this hour', 'New/B at hour 15', 'Opaque/A at hour 15']);
  assert.deepEqual(forms[1].rows, [{ label: 'Renamed faction', value: '5', swatch: '#888888', strong: true, short: true },
    { label: 'Members at this hour', value: '2', indent: true, tone: 'muted', short: true }]);
  assert.equal(forms[0].rows[2].countryFlag?.flag, 'flag-at-15');
  assert.deepEqual(factionTooltipForms(model, ['42'], 13, tag => ({ name: tag }), () => '#888888', String, 'Date', 'Measure')[0].rows,
    [{ label: 'Not recorded', tone: 'muted' }]);
});

test('faction contract rejects stale or overlapping value and name runs, unsafe values and duplicate identities', () => {
  const good = reply();
  assert.equal(validateFactionTotalsReply(good), good);
  for (const mutate of [
    (data: FactionTotalsReply) => { data.series.push(data.series[0]); },
    (data: FactionTotalsReply) => { data.series[0].dissolutionHour = 1.5; },
    (data: FactionTotalsReply) => { data.series[0].runs[1].firstHour = 12; },
    (data: FactionTotalsReply) => { data.series[0].runs[0].value = Infinity; },
    (data: FactionTotalsReply) => { data.series[0].names[1].firstHour = 13; },
    (data: FactionTotalsReply) => { Object.assign(data.series[0].names[0], { reason: undefined }); },
    (data: FactionTotalsReply) => { Object.assign(data.series[0].runs[0], { startsSegment: null }); },
  ]) {
    const invalid = structuredClone(good); mutate(invalid);
    assert.throws(() => validateFactionTotalsReply(invalid), /malformed/);
  }
});

test('5,000 countries in 50 factions build shared series and query only the requested roster', () => {
  const data = reply();
  data.series = Array.from({ length: 50 }, (_, faction) => ({
    id: String(faction), dissolutionHour: null, unavailableReason: null, runs: [{ firstHour: 0, lastHour: 99, value: 100, startsSegment: true }],
    names: [{ firstHour: 0, lastHour: 99, name: `Recorded faction ${faction}`, reason: null }], membership: [{ firstHour: 0, lastHour: 99 }],
    countedMembers: Array.from({ length: 100 }, (_, country) => ({ tag: `Mod:${faction * 100 + country}`, firstHour: 0, lastHour: 99 })),
    members: Array.from({ length: 100 }, (_, country) => ({ tag: `Mod:${faction * 100 + country}`, firstHour: 0, lastHour: 99 })),
  }));
  const model = factionTotalsModel(validateFactionTotalsReply(data));
  assert.equal(factionLines(model, { firstHour: 0, lastHour: 99 }).length, 50);
  const queried = model.series.get('25')!.members;
  queried.comparisons = 0;
  assert.equal(factionMembersAt(model, '25', 50).length, 100);
  assert.ok(queried.comparisons < 10, `roster lookup used ${queried.comparisons} comparisons`);
  const comparisons = queried.comparisons;
  assert.equal(factionMembersAt(model, '25', 50).length, 100);
  assert.equal(queried.comparisons, comparisons, 'label redraws reuse the recorded roster');
  assert.equal(factionNameAt(model, '25', 50), 'Recorded faction 25');
});

test('a 5,000-member hover requests its whole roster and fits only its visual preview to the window', () => {
  const data = reply();
  data.series[0].countedMembers = data.series[0].members = Array.from({ length: 5000 }, (_, index) => ({ tag: `Opaque:${index}`, firstHour: 11, lastHour: 16 }));
  const model = factionTotalsModel(data);
  const requested = factionVisibleMemberTags(model, 15, ['42', '42']);
  assert.equal(requested.length, 5000);
  const wanted = new Set(requested);
  const label = (tag: string) => {
    assert.ok(wanted.has(tag), `unrequested member ${tag}`);
    return tag === 'Opaque:4999' ? { name: `${tag}: name not recorded` } : { name: `${tag} at hour 15`, flag: 'dated-flag' };
  };
  const narrowLimit = factionMemberLabelLimit(800, 1, 'Date');
  const tallerLimit = factionMemberLabelLimit(1600, 1, 'Date');
  assert.ok(tallerLimit > 48 && tallerLimit > narrowLimit);
  for (const [height, limit] of [[800, narrowLimit], [1600, tallerLimit]]) {
    const forms = factionTooltipForms(model, ['42'], 15, label, () => '#888888', String, 'Date', 'Territory', undefined, limit);
    for (const form of forms) {
      assert.equal(form.rows[0].value, '5');
      assert.equal(form.rows[1].value, '5000');
    }
    const fitted = fitTooltip(forms, height);
    assert.ok(fitted.rows.some(row => row.countryFlag?.flag === 'dated-flag'));
    assert.ok(fitted.rows.some(row => row.label === `${5000 - limit} more members`));
    assert.ok(fitted.spoken?.includes('Opaque:999 at hour 15'));
    assert.ok(fitted.spoken?.includes('Opaque:4999: name not recorded'));
  }
  const full = factionTooltipForms(model, ['42'], 15, label, () => '#888888', String, 'Date', 'Territory')[0];
  assert.equal(full.rows.length, 5002);
  assert.equal(full.rows.at(-1)?.label, 'Opaque:999 at hour 15');
});


test('a dissolved faction keeps last-roster totals and the dated note in shared labels and tooltips', () => {
  const data = reply();
  data.series[0].dissolutionHour = 43_800_024;
  const model = factionTotalsModel(validateFactionTotalsReply(data));
  const note = 'last members before it dissolved on 2 Jan 0 · 01:00';
  assert.equal(factionMembershipNote(model, '42'), note);
  assert.equal(factionLabelAt(model, '42', 15), `Renamed faction · ${note}`);
  assert.equal(factionNameAt(model, '42', 15), 'Renamed faction');
  const forms = factionTooltipForms(model, ['42'], 15, tag => ({ name: tag }), () => '#888888', String, 'Faction factories', 'Date');
  for (const form of forms) {
    assert.equal(form.rows[0].value, '5');
    assert.equal(form.rows[1].label, note);
    assert.ok(form.spoken?.includes(note));
  }
  data.series[0].dissolutionHour = null;
  assert.equal(factionMembershipNote(factionTotalsModel(data), '42'), null);
});


test('a dissolved faction with an incomplete last roster keeps its dated note beside the gap warning', () => {
  const data = reply();
  data.series[0].runs = [];
  data.series[0].dissolutionHour = 43_800_024;
  data.series[0].unavailableReason = "This faction's membership was not recorded at the end of this campaign branch";
  const model = factionTotalsModel(validateFactionTotalsReply(data));
  const note = 'last members before it dissolved on 2 Jan 0 · 01:00';
  const forms = factionTooltipForms(model, ['42'], 15, tag => ({ name: tag }), () => '#888888', String, 'Faction factories', 'Date');
  for (const form of forms) {
    assert.equal(form.rows[0].value, 'Total unavailable');
    assert.ok(!form.rows.some(row => row.label === data.series[0].unavailableReason));
    assert.ok(form.rows.some(row => row.label === note));
    assert.ok(form.spoken?.includes(note));
  }
});

test('every membership mode names and requests exactly the counted set at the hovered hour', () => {
  for (const [membershipMode, tags, heading] of [
    ['hour', ['Opaque/A'], 'Members at this hour'],
    ['end', ['New/B'], 'Counted: members at the end'],
    ['ever', ['New/B', 'Opaque/A'], 'Counted: anyone ever a member'],
  ] as const) {
    const data = reply(); data.membershipMode = membershipMode;
    data.series[0].countedMembers = tags.map(tag => ({ tag, firstHour: 11, lastHour: 16 }));
    const model = factionTotalsModel(validateFactionTotalsReply(data));
    assert.deepEqual(factionMembersAt(model, '42', 11), tags);
    assert.deepEqual(factionVisibleMemberTags(model, 11, ['42']), tags);
    const forms = factionTooltipForms(model, ['42'], 11, tag => ({ name: tag, flag: 'hovered-flag' }), () => '#888888', String, 'Measure', 'Date');
    assert.equal(forms[0].rows[1].label, heading);
    assert.equal(forms[0].rows[1].value, String(tags.length));
    assert.deepEqual(forms[0].rows.slice(2).map(row => row.label), tags);
    assert.ok(forms[0].spoken?.includes(heading));
    assert.equal(forms[0].rows[2].countryFlag?.flag, 'hovered-flag');
  }
  assert.throws(() => validateFactionTotalsReply({ ...reply(), membershipMode: 'guessed' }), /malformed/);
  const malformed = reply(); Object.assign(malformed.series[0], { countedMembers: null });
  assert.throws(() => validateFactionTotalsReply(malformed), /malformed/);
});

test('the shared hour-first race tooltip requests the counted members at its hovered hour', () => {
  const model = factionTotalsModel(reply());
  const related = (hour: number, ids: readonly string[]) => factionVisibleMemberTags(model, hour, ids);
  assert.deepEqual(countryRaceTooltipTags(11, ['42'], related), ['42', 'Opaque/A']);
  assert.deepEqual(countryRaceTooltipTags(14, ['42', '42'], related), ['42', 'New/B', 'Opaque/A']);
  assert.equal(factionNameAt(model, '42', 11), 'First name');
  assert.equal(factionNameAt(model, '42', 14), 'Renamed faction');
});


test('Territory faction labels and tooltips use branch template names, gaps, refusals and campaign edits', () => {
  const data = reply();
  data.series[0].runs = [{ firstHour: 11, lastHour: 16, value: 5, startsSegment: true }];
  // Campaign summaries and the totals model cannot replace this branch's dated names.
  const faction: FactionGroup = { id: '42', name: 'Latest branch template', nameReason: null, recordedName: '',
    firstHour: 11, lastHour: 16, members: [], leaders: [], incomplete: [], names: [
      { firstHour: 11, lastHour: 11, name: 'Opening template', reason: null },
      { firstHour: 13, lastHour: 14, name: null, reason: 'Faction template mod_pact is not in the recorded content' },
      { firstHour: 15, lastHour: 16, name: 'Latest branch template', reason: null },
    ] };
  const model = factionTotalsModel(validateFactionTotalsReply(data));
  const factions = new Map([[faction.id, faction]]);
  const names = { factions, names: recordedNameAt };
  for (const [hour, label, reason] of [
    [11, 'Opening template', null], [12, 'Unnamed faction', 'Not recorded'],
    [13, 'Unnamed faction', faction.names[1].reason], [15, 'Latest branch template', null],
  ] as const) {
    assert.equal(factionNameAt(model, '42', hour, names), label);
    assert.equal(factionLabelAt(model, '42', hour, names), label);
    assert.equal(factionNameReasonAt(model, '42', hour, names), reason);
    for (const form of factionTooltipForms(model, ['42'], hour, tag => ({ name: tag }), () => '#888888', String,
      'Territory', 'Date', undefined, Infinity, names)) {
      assert.equal(form.rows[0].label, label);
      assert.equal(form.rows[0].value, '5');
      assert.equal(form.rows.some(row => row.label === 'Name'), false);
      if (reason) {
        assert.ok(!form.rows.some(row => row.value === reason));
        assert.ok(!form.spoken?.includes(reason));
      }
    }
    const edited = { factions, names: (item: FactionGroup, hour?: number) => recordedNameAt(item, hour, 'My pact') };
    assert.equal(factionLabelAt(model, '42', hour, edited), 'My pact');
    assert.equal(factionNameReasonAt(model, '42', hour, edited), null);
  }
});

test('faction totals retain the main name reason when a roster-only faction has no public group yet', () => {
  const data = reply();
  data.series[0].names = [{ firstHour: 11, lastHour: 16, name: null, reason: 'LexicalTokens not in this recording' }];
  const model = factionTotalsModel(validateFactionTotalsReply(data));
  const settings = { factions: new Map<string, FactionGroup>(), names: recordedNameAt };
  assert.equal(factionNameAt(model, '42', 15, settings), 'Unnamed faction');
  assert.equal(factionNameReasonAt(model, '42', 15, settings), 'LexicalTokens not in this recording');
  assert.equal(factionNameReasonAt(model, '42', 17, settings), 'Not recorded');
});

test('faction tooltips keep member flag metadata without repeating flag status in spoken text', () => {
  const pending = { name: 'Pending member', flagPending: true };
  const failed = { name: 'Failed member', flagReason: 'Member flag request failed' };
  const [full] = factionTooltipForms(factionTotalsModel(reply()), ['42'], 15,
    tag => tag === 'New/B' ? pending : failed, () => '#888888', String, 'Date', 'Measure');
  assert.equal(full.countryFlag, undefined);
  assert.equal(full.rows[0].countryFlag, undefined);
  assert.equal(full.rows[2].countryFlag, pending);
  assert.equal(full.rows[3].countryFlag, failed);
  assert.match(full.spoken!, /Failed member/);
  assert.doesNotMatch(full.spoken!, /Flag|request failed/);
  assert.match(full.spoken!, /Pending member/);
  const forms = factionTooltipForms(factionTotalsModel(reply()), ['42'], 15,
    tag => tag === 'New/B' ? { name: 'Missing member' } : { name: 'Loaded member', flag: 'dated.png' },
    () => '#888888', String, 'Date', 'Measure', undefined, 0);
  assert.equal(forms[0].spoken, forms[1].spoken);
  assert.match(forms[1].spoken!, /Missing member/);
  assert.match(forms[1].spoken!, /Loaded member/);
  assert.doesNotMatch(forms[1].spoken!, /Loaded member: Flag/);
  assert.equal(forms[0].rows[0].value, '5');
  assert.equal(full.rows[0].value, '5');
});
