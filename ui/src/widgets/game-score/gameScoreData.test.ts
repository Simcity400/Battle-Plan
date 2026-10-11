import assert from 'node:assert/strict';
import test from 'node:test';
import { groupedRows } from '../../charts/table/logic.ts';
import { FAILED_FACT } from '../../dashboard/countryFacts.ts';
import { countryRaceItems } from '../../charts/race/countryRace.ts';
import { SCORE_PARTS, FALLBACK_LABELS, formatMultiplier, formatScore, gameScoreCountryCell, gameScoreCountryKnown, gameScoreFaction, gameScoreIdentityAt, gameScoreLooks, gameScoreModel, gameScoreRace, gameScoreRow, gameScoreRows, gameScoreTooltip, gameScoreTooltips, scoreInsetReason, scoreValue, validateGameScoreReply } from './gameScoreData.ts';
import type { ScorePart } from './gameScoreData';
import type { GameScoreReply, GameScoreCountry as ScoreCountry, GameScoreIdentity as ScoreIdentity, GameScoreSeries as ScoreSeries } from '../../generated/dashboard';
import { countryAliasesAt } from '../../dashboard/countryIdentity.ts';
import { stepRace } from '../../charts/race/race.ts';
import { widgetMetas } from '../definitions.ts';
import { definition, DEFAULT_SORT, OPTIONS } from './definition.ts';
import { resolveOptions, setOption } from '../../dashboard/options.ts';

const series = (raw = '100000', lastHour = 3): ScoreSeries => Object.fromEntries(SCORE_PARTS.map(part => [part,
  [{ firstHour: 1, lastHour, raw, readEachHour: true }]])) as ScoreSeries;
const identity = (name = 'Recorded name'): ScoreIdentity => ({ canonicalSlot: 0, nativeCountry: 1, name, nameMissing: null, color: null, colorMissing: null,
  faction: null, factionMissing: null });
const country = (tag: string): ScoreCountry => ({ tag, identity: [{ firstHour: 1, lastHour: 3, value: identity(tag) }], scores: series() });
const reply = (): GameScoreReply => ({ branchId: 'synthetic', scale: 100000, reads: [1, 2, 3], reasons: [{ kind: 'recording', reason: 'native roster unreadable' }],
  countries: [country('opaque country')], multipliers: series(), warnings: [], scoreWarnings: [], labels: { ...FALLBACK_LABELS, faction: 'Faction' }, hints: {} });

test('Game score validates compressed values, reason indices and every identity without repairing malformed replies', () => {
  const data = reply();
  data.countries[0].scores.army[0].deltas = '1:1 1:r-3';
  assert.equal(validateGameScoreReply(data), data);
  for (const field of ['scale', 'reads', 'labels', 'countries'] as const) {
    assert.throws(() => validateGameScoreReply({ ...data, [field]: null }));
  }
  assert.throws(() => validateGameScoreReply({ ...data, countries: [data.countries[0], data.countries[0]] }));
  data.countries[0].scores.navy = [{ firstHour: 1, lastHour: 3, missing: 1, readEachHour: true }];
  assert.throws(() => validateGameScoreReply(data));
  data.countries[0].scores.navy[0].missing = 0;
  data.countries[0].identity[0].value.color = [256, 0, 0];
  assert.throws(() => validateGameScoreReply(data));
  for (const id of ['opaque', '-1', '01', '18446744073709551616']) {
    const bad = reply();
    bad.countries[0].identity[0].value.faction = { id, name: 'Same name', nameMissing: null, ordinal: 0 };
    assert.throws(() => validateGameScoreReply(bad));
  }
  for (const field of ['canonicalSlot', 'nativeCountry'] as const) {
    const boundary = reply();
    boundary.countries[0].identity[0].value[field] = 0xffff_ffff;
    assert.equal(validateGameScoreReply(boundary), boundary, `${field} accepts the full native u32 range`);
    boundary.countries[0].identity[0].value[field] = 0x1_0000_0000;
    assert.throws(() => validateGameScoreReply(boundary), `${field} refuses an ID outside the native u32 range`);
  }
});

test('neither recorded values nor content multipliers may bridge an unobserved acquisition hour', () => {
  const data = reply();
  data.reads = [1, 3];
  data.multipliers.total[0].readEachHour = false;
  assert.throws(() => validateGameScoreReply(data));
  const split = <T extends { firstHour: number; lastHour: number }>(run: T): T[] => [
    { ...run, lastHour: 1 }, { ...run, firstHour: 3 },
  ];
  data.countries[0].identity = split(data.countries[0].identity[0]);
  for (const part of SCORE_PARTS) {
    data.countries[0].scores[part] = split(data.countries[0].scores[part][0]);
    data.multipliers[part] = split(data.multipliers[part][0]);
  }
  assert.equal(validateGameScoreReply(data), data);
  assert.equal(gameScoreRow(gameScoreModel(data), data.countries[0].tag, 2), null);
});

test('shared content diagnostics stay strings and score refusals retain typed counts', () => {
  const data = reply();
  data.warnings = ['Content bytes are not hash-verified.'];
  data.scoreWarnings = [{ kind: 'recording', reason: 'native roster unreadable', observations: 3 }];
  assert.equal(validateGameScoreReply(data), data);
  assert.throws(() => validateGameScoreReply({ ...data, warnings: data.scoreWarnings }));
  for (const scoreWarnings of [undefined, data.warnings, [{ ...data.scoreWarnings[0], observations: 0 }]]) {
    assert.throws(() => validateGameScoreReply({ ...data, scoreWarnings }));
  }
});

test('scores follow exact hourly reads and row identities; known zero, unknown and gaps stay distinct', () => {
  const data = reply();
  data.countries[0].identity = [
    { firstHour: 1, lastHour: 1, value: identity('Before') }, { firstHour: 2, lastHour: 3, value: identity('After') },
  ];
  data.countries[0].scores.army = [{ firstHour: 1, lastHour: 3, raw: '0', deltas: '1:1 1:-2', readEachHour: true }];
  data.countries[0].scores.navy = [{ firstHour: 1, lastHour: 3, missing: 0, readEachHour: true }];
  data.countries[0].scores.land = [{ firstHour: 1, lastHour: 1, raw: '0', readEachHour: true }, { firstHour: 3, lastHour: 3, raw: '200000', readEachHour: true }];
  data.multipliers.total = [{ firstHour: 1, lastHour: 3, raw: '150000', readEachHour: false }];
  const model = gameScoreModel(data);
  assert.equal(gameScoreRow(model, data.countries[0].tag, 0), null);
  const before = gameScoreRow(model, data.countries[0].tag, 1)!;
  const after = gameScoreRow(model, data.countries[0].tag, 2)!;
  assert.equal(before.identity.name, 'Before');
  assert.equal(after.identity.name, 'After');
  assert.equal(before.cells.army.raw, '0');
  assert.equal(after.cells.army.raw, '100000');
  assert.equal(after.cells.army.readHour, 2);
  assert.equal(after.cells.navy.raw, null);
  assert.equal(after.cells.navy.reason, 'native roster unreadable');
  assert.equal(after.cells.land.raw, null);
  assert.match(after.cells.land.reason!, /gap/);
  assert.equal(after.multipliers.total.readHour, null);
  assert.equal(after.multipliers.total.contentEvaluated, true);
  assert.equal(gameScoreRow(model, data.countries[0].tag, 3)!.cells.army.raw, '-100000');
  const tooltip = gameScoreTooltip(data, after);
  assert.equal(tooltip.title, 'After');
  assert.ok(tooltip.rows.some(row => row.label === 'Content-evaluated; no recorded read date.'));
  assert.ok(tooltip.rows.some(row => row.label === 'Raw fixed5: 100000'));
  for (const part of SCORE_PARTS) assert.ok(tooltip.rows.some(row => row.label === `${data.labels[part]} multiplier`));
  const compact = [...gameScoreTooltips(data, after)].at(-1)!;
  assert.equal(compact.rows.filter(row => row.value?.includes(' · raw ')).length, 10);
  assert.ok(compact.rows.some(row => row.label.includes('Content-evaluated; no recorded read date.')));
});

test('score easing continues through look changes and breaks when a native holder returns after another holder', () => {
  const data = reply();
  const before = identity('Before'), after = { ...identity('After'), color: [12, 34, 56] as [number, number, number],
    faction: { id: '2', name: 'Joined faction', nameMissing: null, ordinal: 0 } };
  data.reads = [1, 2, 3, 4, 5, 6];
  data.countries[0].identity = [
    { firstHour: 1, lastHour: 1, value: before }, { firstHour: 2, lastHour: 2, value: after },
    { firstHour: 3, lastHour: 3, value: { ...after, nativeCountry: 2 } },
    { firstHour: 4, lastHour: 4, value: before },
    { firstHour: 5, lastHour: 5, value: { ...before, nativeCountry: null } },
    { firstHour: 6, lastHour: 6, value: { ...after, nativeCountry: null } },
  ];
  data.countries[0].scores = series('100000', 6);
  data.multipliers = series('100000', 6);
  const model = gameScoreModel(validateGameScoreReply(data));
  const row = (hour: number) => gameScoreRow(model, data.countries[0].tag, hour)!;
  assert.equal(row(2).stretch, row(1).stretch);
  assert.notEqual(row(3).stretch, row(2).stretch);
  assert.notEqual(row(4).stretch, row(1).stretch);
  assert.notEqual(row(6).stretch, row(5).stretch, 'unresolved holders cannot establish continuous native identity');
});

test('table row keys follow native countries through tag changes and distinguish reused or unresolved tags', () => {
  const data = reply();
  const original = country('original'), renamed = country('renamed');
  original.identity = [
    { firstHour: 1, lastHour: 1, value: identity() },
    { firstHour: 2, lastHour: 3, value: { ...identity(), nativeCountry: 2 } },
  ];
  renamed.identity[0].firstHour = 2;
  const unknown = country('unknown'), otherUnknown = country('other unknown');
  unknown.identity[0].value.nativeCountry = null;
  otherUnknown.identity[0].value.nativeCountry = null;
  data.countries = [original, renamed, unknown, otherUnknown];
  const model = gameScoreModel(validateGameScoreReply(data));
  const before = gameScoreRow(model, 'original', 1)!;
  const after = gameScoreRow(model, 'renamed', 2)!;
  assert.equal(after.key, before.key);
  assert.equal(after.stretch, before.stretch, 'native score continuity crosses an alias change');
  assert.notEqual(gameScoreRow(model, 'original', 2)!.key, before.key);
  assert.notEqual(gameScoreRow(model, 'unknown', 2)!.key, gameScoreRow(model, 'other unknown', 2)!.key);
  const items = (rows: typeof before[]) => countryRaceItems(gameScoreRace(rows, false), new Map(), () => '#777', '#777', false);
  const first = stepRace(null, items([before]), 0, 0, 100);
  const reused = gameScoreRow(model, 'original', 2)!;
  const next = stepRace(first, items([after, reused]), 16, 16, 100);
  const kept = next.rows.find(row => row.key === before.key)!;
  assert.equal(kept.phase, 'staying');
  assert.equal(kept.item.tag, 'renamed');
  assert.equal(kept.item.hour, 2);
  assert.equal(next.rows.find(row => row.key === reused.key)!.phase, 'entering');
  const rewind = stepRace(next, items([before]), 32, 16, 100);
  assert.equal(rewind.rows.find(row => row.key === before.key)!.item.tag, 'original');
});

test('Land is the default exact sort and every country is eligible, including 5,000 synthetic rows', () => {
  const data = reply();
  data.countries = Array.from({ length: 5000 }, (_, index) => {
    const item = country(`slot-${String(index).padStart(4, '0')}`);
    item.identity[0].value.canonicalSlot = index;
    item.scores.land[0].raw = (9007199254740992n + BigInt(index)).toString();
    return item;
  });
  data.countries[4].scores.land = [{ firstHour: 1, lastHour: 3, missing: 0, readEachHour: true }];
  const model = gameScoreModel(validateGameScoreReply(data));
  const rows = gameScoreRows(model, model.tags, 2, DEFAULT_SORT);
  assert.equal(rows.length, 5000);
  assert.equal(new Set(rows.map(row => row.key)).size, 5000);
  assert.equal(rows[0].tag, 'slot-4999');
  assert.equal(rows.at(-1)!.tag, 'slot-0004');
  assert.equal(gameScoreRows(model, model.tags, 2, DEFAULT_SORT, 2).length, 2);
  assert.deepEqual(definition.countries, { max: Number.MAX_SAFE_INTEGER, ranked: true, all: true });
  const registered = new Set(widgetMetas.map(meta => meta.kind));
  for (const kind of ['game-score', 'occupation-trend', 'scoreboard', 'casualties'] as const) assert.ok(registered.has(kind));
  assert.equal(resolveOptions(OPTIONS).view, 'table');
  assert.equal(resolveOptions(OPTIONS).split, true);
  assert.equal(resolveOptions(OPTIONS).multiplierPrecision, 'exact');
  const saved = setOption(OPTIONS, {}, 'multiplierPrecision', '2');
  assert.ok(saved);
  assert.equal(formatMultiplier('199999', String(resolveOptions(OPTIONS, saved).multiplierPrecision)), '1.99');
  assert.equal(formatMultiplier('199999'), '1.99999');
  assert.equal(formatMultiplier('100000'), '1');
  assert.equal(formatMultiplier('100010'), '1.0001');
  assert.equal(formatMultiplier('0'), '0');
  assert.equal(formatMultiplier(null), '—');
  assert.equal(setOption(OPTIONS, {}, 'multiplierPrecision', '6'), null);
  assert.equal(formatScore('-199999'), '-1');
});

test('same-name factions keep distinct recorded identities and unknown membership precedes known no faction', () => {
  const data = reply();
  data.countries = ['none', 'unknown', 'a-low', 'a-high', 'b-highest'].map(country);
  for (const [index, item] of data.countries.entries()) {
    item.scores.land[0].raw = String(index * 100000);
    if (index === 1) item.identity[0].value.factionMissing = { kind: 'recording', reason: 'membership unreadable' };
    if (index > 1) item.identity[0].value.faction = { id: index === 4 ? '10' : '2', name: 'Same name', nameMissing: null, ordinal: index };
  }
  const model = gameScoreModel(data);
  const rows = gameScoreRows(model, model.tags, 2, DEFAULT_SORT);
  const ordered = groupedRows(rows, gameScoreFaction, row => scoreValue(row, 'land'), 'descending', row => row.key, row => row.tag);
  assert.deepEqual(ordered.map(row => row.tag), ['a-high', 'a-low', 'b-highest', 'unknown', 'none']);
  assert.equal(gameScoreFaction(rows.find(row => row.tag === 'unknown')!).label, 'Faction not recorded');
  assert.equal(gameScoreTooltip(data, rows.find(row => row.tag === 'unknown')!).rows[0].value, 'Faction not recorded');
  const unnamed = rows.find(row => row.tag === 'a-low')!;
  unnamed.identity.faction!.id = '20';
  unnamed.identity.faction!.name = null;
  unnamed.identity.faction!.nameMissing = { kind: 'content', reason: 'Faction localisation unavailable' };
  assert.equal(gameScoreFaction(unnamed).label, 'Faction name unavailable');
  assert.deepEqual(groupedRows(rows, gameScoreFaction, row => scoreValue(row, 'land'), 'descending', row => row.key).map(row => row.tag),
    ['a-high', 'b-highest', 'a-low', 'unknown', 'none']);
  assert.ok(gameScoreTooltip(data, unnamed).rows.some(row => row.label === 'Faction localisation unavailable'));
});

test('race ranks Total exactly and scales only Army Navy Air shares inside it; Industry and Land remain tooltip-only', () => {
  const data = reply();
  const values: Record<ScorePart, string> = { total: '1200000', army: '300000', navy: '200000', air: '100000', industry: '9900000', land: '8800000' };
  for (const part of SCORE_PARTS) data.countries[0].scores[part][0].raw = values[part];
  data.multipliers.total[0].raw = '200000';
  const model = gameScoreModel(data);
  const rows = gameScoreRows(model, model.tags, 2, { column: 'total', direction: 'descending' });
  const entries = gameScoreRace(rows, true);
  assert.equal(entries[0].label, '12');
  assert.deepEqual(entries[0].parts?.map(part => part.id), ['army', 'navy', 'air']);
  const [item] = countryRaceItems(entries, new Map(), () => 'country', 'missing', false);
  assert.deepEqual(item.segments.map(part => part.value), [6, 4, 2]);
  assert.equal(gameScoreRace(rows, false)[0].parts, undefined);
  assert.ok(gameScoreTooltip(data, rows[0], true).rows.some(row => row.label === data.labels.industry && row.value === '99'));
  assert.ok(gameScoreTooltip(data, rows[0], true).rows.some(row => row.label === data.labels.land && row.value === '88'));
  rows[0].cells.army.raw = '-1';
  assert.match(scoreInsetReason(rows[0])!, /Signed or wrapped/);
  assert.equal(gameScoreRace(rows, true)[0].parts, undefined);
  rows[0].cells.army.raw = '9223372036854775807';
  assert.match(scoreInsetReason(rows[0])!, /wrapped/);
});

test('race plot shares conserve the native truncated Total, including zero multipliers and known zero parts', () => {
  for (const [partRaws, multiplier, total] of [
    [['1', '1', '1'], '150000', '4'],
    [['100000', '200000', '300000'], '0', '0'],
    [['0', '0', '0'], '200000', '0'],
  ] as const) {
    const data = reply();
    for (const [index, part] of (['army', 'navy', 'air'] as const).entries()) data.countries[0].scores[part][0].raw = partRaws[index];
    data.countries[0].scores.total[0].raw = total;
    data.multipliers.total[0].raw = multiplier;
    const model = gameScoreModel(data);
    const rows = gameScoreRows(model, model.tags, 2, { column: 'total', direction: 'descending' });
    const before = JSON.stringify(rows[0]);
    const [item] = countryRaceItems(gameScoreRace(rows, true), new Map(), () => 'country', 'missing', false);
    assert.equal(item.value, Number(total) / 100000);
    assert.ok(item.segments.every(part => Number.isFinite(part.value) && part.value >= 0));
    assert.ok(Math.abs(item.segments.reduce((sum, part) => sum + part.value, 0) - item.value) <= Number.EPSILON);
    assert.equal(JSON.stringify(rows[0]), before, 'plot proxies leave exact amounts, country identity and read dates intact');
    assert.equal(rows[0].cells.army.raw, partRaws[0]);
    assert.equal(rows[0].cells.army.readHour, 2);
    rows[0].cells.industry = { ...rows[0].cells.industry, raw: null, reason: 'Effective factories were not recorded.' };
    const [partial] = countryRaceItems(gameScoreRace(rows, true), new Map(), () => 'country', 'missing', false);
    assert.equal(partial.value, item.value);
    assert.deepEqual(partial.segments, item.segments, 'unavailable Industry stays outside the Total geometry');
    assert.ok(gameScoreTooltip(data, rows[0], true).rows.some(row => row.label === 'Effective factories were not recorded.'));
  }
});

test('unknown totals never become a zero race bar and easing stretches break at recording gaps', () => {
  const data = reply();
  data.countries[0].scores.total = [{ firstHour: 1, lastHour: 1, raw: '100000', readEachHour: true },
    { firstHour: 2, lastHour: 2, missing: 0, readEachHour: true }, { firstHour: 3, lastHour: 3, raw: '200000', readEachHour: true }];
  const model = gameScoreModel(data);
  const at = (hour: number) => gameScoreRows(model, model.tags, hour, DEFAULT_SORT);
  assert.equal(gameScoreRace(at(2), true).length, 0);
  assert.notEqual(gameScoreRace(at(1), true)[0].stretch, gameScoreRace(at(3), true)[0].stretch);
});

test('native identity projections reuse manual aliases without decoding score fields or filling gaps', () => {
  const data = reply();
  const old = country('opaque old'), renamed = country('opaque renamed'), unrelated = country('other'), unknown = country('unknown identity');
  old.identity[0].lastHour = 1;
  renamed.identity[0].firstHour = 3;
  unrelated.identity[0].value.nativeCountry = 9;
  unknown.identity[0].value.nativeCountry = null;
  data.countries = [old, renamed, unrelated, unknown];
  const model = gameScoreModel(data);
  assert.deepEqual(countryAliasesAt(model.identities, ['opaque old'], 1), ['opaque old']);
  assert.deepEqual(countryAliasesAt(model.identities, ['opaque old'], 2), []);
  assert.deepEqual(countryAliasesAt(model.identities, ['opaque old'], 3), ['opaque renamed']);
  assert.deepEqual(countryAliasesAt(model.identities, ['unknown identity'], 2), []);
  renamed.scores.total[0].raw = 'not a score';
  assert.equal(gameScoreIdentityAt(model, 'opaque renamed', 3)?.name, 'opaque renamed');
  assert.equal(gameScoreIdentityAt(model, 'opaque renamed', 2), null);
  assert.equal(model.tags.length, 4);
});

test('product-owned race looks preserve exact dated identity, empty names, colors and pending flags', () => {
  const data = reply();
  const tag = data.countries[0].tag;
  data.countries[0].identity = [
    { firstHour: 1, lastHour: 1, value: { ...identity('Before'), color: [12, 34, 56] } },
    { firstHour: 2, lastHour: 3, value: { ...identity(''), nativeCountry: 2, color: [65, 43, 21] } },
  ];
  const model = gameScoreModel(validateGameScoreReply(data));
  const requested: [string, number][] = [];
  const flagAt = (tag: string, hour: number) => { requested.push([tag, hour]); return hour === 1 ? { url: 'old-flag' } : undefined; };
  const before = gameScoreLooks(model, 1, flagAt);
  const after = gameScoreLooks(model, 2.9, flagAt);
  assert.equal(before.name(tag), 'Before');
  assert.equal(after.name(tag), '', 'known empty names do not become tags');
  assert.equal(after.name(tag, 1), 'Before', 'tooltip identities use their own exact hour');
  assert.deepEqual(before.color(tag), [12, 34, 56]);
  assert.deepEqual(after.color(tag), [65, 43, 21]);
  assert.equal(after.matches(tag, 1), false, 'the color hook cannot carry an earlier holder color to this read');
  assert.equal(after.matches(tag, 2), true);
  assert.equal(before.flag(tag), 'old-flag');
  assert.equal(after.flag(tag), undefined, 'a pending current-hour flag never reuses the older flag');
  assert.deepEqual(requested, [[tag, 1], [tag, 2]]);
  const pending = gameScoreLooks(null, 2, flagAt);
  assert.equal(pending.name(tag), 'Country name unavailable');
  assert.equal(pending.color(tag), null);
  assert.equal(pending.flag(tag), undefined);
  assert.equal(pending.matches(tag, 2), false);
  const other = reply();
  other.countries[0].identity[0].value.name = 'Other source';
  other.countries[0].identity[0].value.color = [1, 2, 3];
  const switched = gameScoreLooks(gameScoreModel(other), 2, () => ({ url: 'other-flag' }));
  assert.equal(switched.name(tag), 'Other source');
  assert.deepEqual(switched.color(tag), [1, 2, 3]);
  assert.equal(switched.flag(tag), 'other-flag');
});

test('native country flag presence and completed absence use the exact supplying read', () => {
  const model = gameScoreModel(reply());
  const tag = model.tags[0];
  const requested: [string, number][] = [];
  const reason = 'The recorded content does not define this country flag.';
  const looks = gameScoreLooks(model, 2.9, (tag, hour) => {
    requested.push([tag, hour]);
    return { reason };
  });
  const completed = looks.countryFlag(tag);
  assert.equal(completed?.flag, undefined);
  assert.equal(completed?.flagReason, reason);
  assert.equal(completed?.flagPending, false, 'a completed native refusal is not pending');
  assert.deepEqual(requested, [[tag, 2]], 'metadata is read at the actual integer supplying hour');
  assert.equal(looks.countryFlag(tag, 1), undefined, 'a different read cannot reuse current country flag state');
  assert.equal(looks.countryFlag('unknown country'), undefined, 'generic or unknown actors have no native flag frame');
  assert.deepEqual(requested, [[tag, 2]], 'unknown actors and mismatched reads do not request flag metadata');
  const missing = gameScoreLooks(model, 2, () => null).countryFlag(tag);
  assert.equal(missing?.flag, undefined);
  assert.equal(missing?.flagPending, false);
  assert.ok(missing?.flagReason);
  assert.equal(gameScoreLooks(model, 2, () => undefined).countryFlag(tag)?.flagPending, true);
  const failed = gameScoreLooks(model, 2, () => FAILED_FACT).countryFlag(tag);
  assert.equal(failed?.flagPending, false);
  assert.ok(failed?.flagReason);
});

test('unresolved native identity DTOs stay flagless and never ask for flag metadata', () => {
  for (const [canonicalSlot, nativeCountry] of [[null, null], [0, null], [null, 1]]) {
    const data = reply();
    const tag = data.countries[0].tag;
    const missing = data.reasons[0];
    data.countries[0].identity[0].value = {
      canonicalSlot, nativeCountry, name: null, nameMissing: missing,
      color: null, colorMissing: missing, faction: null, factionMissing: missing,
    };
    const model = gameScoreModel(validateGameScoreReply(data));
    assert.notEqual(gameScoreIdentityAt(model, tag, 2), null, 'the unresolved DTO exists at this read');
    assert.equal(gameScoreCountryKnown(gameScoreIdentityAt(model, tag, 2)), false);
    let reads = 0;
    const flagAt = () => { reads++; return { url: 'must not be requested' }; };
    const looks = gameScoreLooks(model, 2, flagAt);
    assert.equal(looks.countryFlag(tag), undefined);
    assert.equal(looks.flag(tag), undefined);
    assert.equal(gameScoreCountryCell(gameScoreRow(model, tag, 2)!, flagAt), null, 'the table uses a flagless name cell');
    assert.equal(reads, 0);
  }
});

test('table country metadata retains its row read and survives unavailable score values', () => {
  const data = reply();
  const tag = data.countries[0].tag;
  data.countries[0].identity[0].value.name = '';
  for (const part of SCORE_PARTS) data.countries[0].scores[part] = [{ firstHour: 1, lastHour: 3, missing: 0, readEachHour: true }];
  const model = gameScoreModel(validateGameScoreReply(data));
  const row = gameScoreRow(model, tag, 1)!;
  assert.equal(row.cells.total.raw, null);
  const requested: [string, number][] = [];
  const reason = 'This row has no artwork in its recorded content.';
  const cell = gameScoreCountryCell(row, (tag, hour) => { requested.push([tag, hour]); return { reason }; });
  assert.equal(cell?.name, '', 'a known empty name remains empty');
  assert.equal(cell?.flagReason, reason);
  assert.equal(cell?.flagPending, false);
  assert.deepEqual(requested, [[tag, 1]], 'a leaving table row keeps its own supplying hour');
  assert.equal(gameScoreCountryCell(row, () => undefined)?.flagPending, true);
  const failed = gameScoreCountryCell(row, () => FAILED_FACT);
  assert.equal(failed?.flagPending, false);
  assert.ok(failed?.flagReason);
  assert.equal(gameScoreCountryCell(row, () => ({ url: 'row flag' }))?.flag, 'row flag');
  const current = gameScoreLooks(model, 2.9, () => ({ url: 'current flag' })).countryFlag(tag);
  assert.equal(current?.flag, 'current flag', 'score-only refusal does not remove a known native country flag');
});
