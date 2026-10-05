import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import type { DecryptionPair, DecryptionReply, DecryptionRun } from '../../generated/dashboard';
import { activeBonusDays, amountText, CIPHER_BROKEN, cipherStatus, decryptionTooltip, observationDate, NATIVE_PROGRESS_UNKNOWN, progressLabel, progressPercent, progressText, raceRows, validateDecryptionReply, valueAt, workingStatus } from './decryptionData.ts';
import { createWidget } from '../../dashboard/model.ts';
import { parseTemplate } from '../../dashboard/storage.ts';
import { defaultTemplate } from '../../dashboard/model.ts';
import { visibleRange } from '../../components/countryList.ts';
import { groupMembers, groupRuns } from '../../dashboard/groups.ts';
import type { CountryGroupReply } from '../../generated/dashboard';
import { definition, OPTIONS } from './definition.ts';
import { sectionFields, resolveOptions } from '../../dashboard/options.ts';
import { nextRows } from '../../charts/rowMotion.ts';
import '../definitions.ts';

function run(firstHour = 100, lastHour = 102, amountRaw = '400000'): DecryptionRun {
  return { firstHour, lastHour, startAmountRaw: amountRaw, stepRawPerDay: '0', thresholdRaw: '800000', thresholdUnknown: null,
    bonusDays: -1, bonusDaysStepPerDay: 0, stateFlags: 1, broken: false, started: { hour: firstHour, upperBound: true } };
}
const decoded = (encoded: DecryptionRun) => valueAt(encoded, encoded.firstHour)!;
function reply(runs = [run()]): DecryptionReply {
  runs = runs.map(encoded => ({ ...encoded, started: { hour: runs[0].firstHour, upperBound: true } }));
  return { branchId: '1', coverage: [{ firstHour: 100, lastHour: 104, complete: true, reason: null }], warnings: [],
    pairs: [{ ownerId: '100', targetId: '200', runs, tagRuns: runs.map(({ firstHour, lastHour }) => ({ firstHour, lastHour,
      ownerTag: 'New decryptor|with spaces', targetTag: 'Renamed target' })), breaks: [] }] };
}

test('payload checks raw signed values, native bytes, ordered runs, coverage and dated observations', () => {
  assert.equal(validateDecryptionReply(reply()).pairs[0].runs[0].startAmountRaw, '400000');
  const mutations: ((data: DecryptionReply) => void)[] = [
    data => { data.pairs[0].runs[0].startAmountRaw = '9223372036854775808'; },
    data => { data.pairs[0].runs[0].bonusDays = NaN; },
    data => { data.pairs[0].runs[0].stateFlags = 65536; },
    data => { data.pairs[0].runs[0].stepRawPerDay = '1.5'; },
    data => { data.pairs[0].runs[0].firstHour = 0.5; },
    data => { data.pairs[0].runs[0].lastHour = 99; },
    data => { data.pairs[0].runs.push(run(102, 103)); },
    data => { data.pairs.push(data.pairs[0]); },
    data => { data.pairs[0].runs[0].started!.hour = 101; },
    data => { data.pairs[0].runs[0].started = null; },
    data => { data.pairs[0].runs[0].started!.upperBound = false; },
    data => { data.coverage[0].firstHour = 101; },
    data => { data.pairs[0].runs[0].thresholdRaw = null; },
    data => { data.pairs[0].runs[0].stateFlags = 0x101; },
  ];
  for (const mutate of mutations) { const data = reply(); mutate(data); assert.throws(() => validateDecryptionReply(data), /malformed/); }
  for (const malformed of [null, [], {}, { ...reply(), pairs: [null] }, { ...reply(), coverage: [null] }]) assert.throws(() => validateDecryptionReply(malformed), /malformed/);
});

test('linear encoding round-trips every recorded hourly value and leaves fractional hours and gaps absent', () => {
  // The reads below retain daily native plateaus, including the offset first
  // daily update. Rate changes, an off-grid script change, activation, expiry,
  // threshold changes and missing hours each begin a separate encoded run.
  const encoded = [
    { ...run(101, 172, '100001'), stepRawPerDay: '30001', stateFlags: 0xfe },
    { ...run(173, 196, '190004'), stepRawPerDay: '17003', stateFlags: 0xfe },
    { ...run(197, 210, '207007'), stateFlags: 0xfe },
    { ...run(211, 220, '207008'), stateFlags: 0xfe },
    { ...run(221, 292, '103504'), stepRawPerDay: '17003', stateFlags: 0xabfe,
      bonusDays: 3, bonusDaysStepPerDay: -1, broken: true },
    { ...run(293, 316, '137510'), stepRawPerDay: '17003', stateFlags: 0xfe },
    { ...run(317, 340, '154513'), thresholdRaw: '150000', stateFlags: 0xfe, broken: true },
    { ...run(342, 389, '171516'), stepRawPerDay: '17003', thresholdRaw: '150000', stateFlags: 0xfe, broken: true },
  ];
  const data = reply(encoded);
  data.coverage = [{ firstHour: 101, lastHour: 340, complete: true, reason: null },
    { firstHour: 342, lastHour: 389, complete: true, reason: null }];
  data.pairs[0].breaks = [{ hour: 221, upperBound: false }, { hour: 317, upperBound: false }, { hour: 342, upperBound: true }];
  for (const encoded of data.pairs[0].runs.slice(5, 7)) encoded.started = { hour: 293, upperBound: false };
  data.pairs[0].runs[7].started = { hour: 342, upperBound: true };
  validateDecryptionReply(data);
  // Independent recorded stream: explicit daily observations expanded to the
  // 24 hourly reads that were captured, never synthesized between reads.
  const days: [number, number, string, number, number, boolean][] = [
    [101, 124, '100001', -1, 0, false], [125, 148, '130002', -1, 0, false], [149, 172, '160003', -1, 0, false],
    [173, 196, '190004', -1, 0, false], [197, 210, '207007', -1, 0, false], [211, 220, '207008', -1, 0, false],
    [221, 244, '103504', 3, 0xab, true], [245, 268, '120507', 2, 0xab, true], [269, 292, '137510', 1, 0xab, true],
    [293, 316, '137510', -1, 0, false], [317, 340, '154513', -1, 0, true],
    [342, 365, '171516', -1, 0, true], [366, 389, '188519', -1, 0, true],
  ];
  let readCount = 0;
  for (const [first, last, amount, bonusDays, bonusActive, broken] of days) for (let at = first; at <= last; at++) {
    const row = raceRows(data, at, 'progress')[0];
    assert.ok(row, `missing recorded hour ${at}`);
    assert.equal(row.run.amountRaw, amount, `amount at ${at}`);
    assert.equal(row.run.bonusDays, bonusDays, `countdown at ${at}`);
    assert.equal(row.run.bonusActive, bonusActive, `active native byte at ${at}`);
    assert.equal(row.run.decrypting, 0xfe, `decrypting native byte at ${at}`);
    assert.equal(row.run.broken, broken, `broken at ${at}`);
    readCount++;
  }
  assert.equal(readCount, 288);
  for (const at of [100, 101.5, 340.5, 341, 390, NaN, Infinity]) assert.equal(raceRows(data, at, 'progress').length, 0);
  for (const at of [100, 101.5, 173, NaN, Infinity]) assert.equal(valueAt(encoded[0], at), null);
  assert.equal(valueAt(encoded[0], 124)!.amountRaw, '100001');
  assert.equal(valueAt(encoded[0], 125)!.amountRaw, '130002');
});

test('native progress matches exact rounding, wrapping, sentinel and overflow vectors', () => {
  const vectors = JSON.parse(readFileSync(new URL('../../../../crates/dashboard/src/cryptology/test-values.json', import.meta.url), 'utf8')) as [string, number, string, number | null][];
  for (const [amountRaw, active, thresholdRaw, expected] of vectors) {
    const amount = BigInt(amountRaw), threshold = BigInt(thresholdRaw);
    assert.equal(progressPercent(amount, active, threshold), expected, `${amount}/${threshold}, active ${active}`);
    const encoded = { ...run(100, 100, amount.toString()), thresholdRaw: threshold.toString(), stateFlags: active << 8 | 1 };
    assert.equal(valueAt(encoded, 100)!.progressPercent, expected);
  }
  assert.equal(valueAt({ ...run(), thresholdRaw: null, thresholdUnknown: 'mod target missing', stateFlags: 0x701, broken: true }, 100)!.progressPercent, null);
  const overflow = reply([run(100, 102, (1n << 58n).toString())]);
  overflow.pairs[0].runs[0].thresholdRaw = '-1';
  overflow.pairs[0].runs[0].broken = true;
  overflow.pairs[0].breaks = [{ hour: 100, upperBound: true }];
  assert.equal(decryptionTooltip(raceRows(validateDecryptionReply(overflow), 100, 'progress')[0], 'Owner', 'Target', 800).note, NATIVE_PROGRESS_UNKNOWN);
});

test('run validation rejects endpoint overflow and hidden work or break transitions', () => {
  const malformedRuns: DecryptionRun[] = [
    { ...run(100, 148, '9223372036854775806'), stepRawPerDay: '1', broken: true },
    { ...run(100, 124, '-9223372036854775808'), stepRawPerDay: '-1' },
    { ...run(100, 124), bonusDays: 2_147_483_647, bonusDaysStepPerDay: 1 },
    { ...run(100, 124), bonusDays: -2_147_483_648, bonusDaysStepPerDay: -1 },
    { ...run(100, 124, '0'), stepRawPerDay: '1', stateFlags: 0 },
    { ...run(100, 124, '1'), stepRawPerDay: '-1', stateFlags: 0 },
    { ...run(100, 124, '799999'), stepRawPerDay: '1' },
    { ...run(100, 124, '800000'), stepRawPerDay: '-1', broken: true },
    { ...run(), broken: true }, { ...run(), broken: null },
    { ...run(), thresholdRaw: null, thresholdUnknown: 'missing', broken: false },
    { ...run(), stateFlags: -1 }, { ...run(), stateFlags: 0.5 },
    { ...run(), bonusDaysStepPerDay: 2_147_483_648 },
  ];
  for (const encoded of malformedRuns) {
    const data = reply([encoded]); data.coverage[0].lastHour = encoded.lastHour;
    if ((encoded.stateFlags & 0xff) === 0 && BigInt(encoded.startAmountRaw) <= 0n) data.pairs[0].runs[0].started = null;
    if (encoded.broken === true) data.pairs[0].breaks = [{ hour: encoded.firstHour, upperBound: true }];
    assert.throws(() => validateDecryptionReply(data), /malformed/, JSON.stringify(encoded));
  }
  const minimum = { ...run(100, 124, '-9223372036854775808'), stepRawPerDay: '1', bonusDays: -2_147_483_648, bonusDaysStepPerDay: 1 };
  const maximum = { ...run(125, 149, '9223372036854775807'), stepRawPerDay: '-1', bonusDays: 2_147_483_647, bonusDaysStepPerDay: -1, broken: true };
  const data = reply([minimum, maximum]); data.coverage[0].lastHour = 149;
  data.pairs[0].breaks = [{ hour: 125, upperBound: false }];
  validateDecryptionReply(data);
  assert.equal(valueAt(minimum, 124)!.amountRaw, '-9223372036854775807');
  assert.equal(valueAt(maximum, 149)!.amountRaw, '9223372036854775806');
  assert.equal(valueAt(minimum, 124)!.bonusDays, -2_147_483_647);
  assert.equal(valueAt(maximum, 149)!.bonusDays, 2_147_483_646);
});

test('starts and breaks stay behind the playhead, gaps stay gaps, paused native states remain visible', () => {
  const data = reply([run(100, 100), { ...run(102, 102), stateFlags: 0 }, { ...run(103, 104, '800000'), broken: true }]);
  data.pairs[0].runs[1].started = { hour: 102, upperBound: false };
  data.pairs[0].runs[2].started = { hour: 102, upperBound: false };
  data.pairs[0].breaks = [{ hour: 103, upperBound: false }];
  validateDecryptionReply(data);
  assert.equal(raceRows(data, 99, 'progress').length, 0);
  assert.equal(raceRows(data, 101, 'progress').length, 0);
  assert.equal(raceRows(data, 102, 'progress')[0].run.decrypting, 0);
  assert.equal(raceRows(data, 102, 'progress')[0].run.started?.hour, 102);
  assert.equal(cipherStatus(raceRows(data, 102, 'progress')[0]), 'Paused');
  assert.equal(raceRows(data, 103, 'progress')[0].run.started?.hour, 102);
  assert.equal(raceRows(data, 103, 'progress')[0].broken?.hour, 103);
  const future = decryptionTooltip(raceRows(data, 100, 'progress')[0], 'Owner then', 'Target then', 800);
  assert.equal(future.rows.at(-1)?.value, 'Decrypting');
  assert.ok(!JSON.stringify(future).includes('Cipher broken'));
});

test('a second break displays its own date and jump target after bonus expiry', () => {
  const data = reply([
    run(100, 100),
    { ...run(101, 110, '800000'), broken: true },
    { ...run(111, 120, '100000'), stateFlags: 0x101, bonusDays: 10, broken: true },
    run(121, 124, '100000'),
    { ...run(125, 130, '800000'), broken: true },
  ]);
  data.coverage[0].lastHour = 130;
  data.pairs[0].breaks = [{ hour: 101, upperBound: false }, { hour: 125, upperBound: false }];
  data.pairs[0].runs[3].started = { hour: 121, upperBound: false };
  data.pairs[0].runs[4].started = { hour: 121, upperBound: false };
  validateDecryptionReply(data);
  const first = raceRows(data, 110, 'progress')[0];
  const active = raceRows(data, 115, 'progress')[0];
  const expired = raceRows(data, 124, 'progress')[0];
  const second = raceRows(data, 125, 'progress')[0];
  assert.equal(first.run.started?.hour, 100);
  assert.equal(first.broken?.hour, 101);
  assert.equal(active.run.started?.hour, 100);
  assert.equal(active.broken?.hour, 101);
  assert.equal(expired.run.started?.hour, 121);
  assert.equal(expired.run.broken, false);
  assert.equal(second.run.started?.hour, 121);
  assert.equal(second.broken?.hour, 125);
  assert.notEqual(observationDate(first.broken, first.hour), observationDate(second.broken, second.hour));
  const tip = decryptionTooltip(second, 'Mod decryptor', 'Renamed target', 800);
  assert.equal(tip.rows.at(-1)?.value, `Cipher broken ${observationDate(data.pairs[0].breaks[1], 125)}`);
  assert.equal(tip.rows[1].value, observationDate(second.run.started, 125));
  for (const breaks of [data.pairs[0].breaks.slice(0, 1), [...data.pairs[0].breaks].reverse(),
    [{ hour: 101, upperBound: false }, { hour: 111, upperBound: false }, { hour: 125, upperBound: false }],
    [{ hour: 101, upperBound: false }, { hour: 125, upperBound: true }]]) {
    assert.throws(() => validateDecryptionReply({ ...data, pairs: [{ ...data.pairs[0], breaks }] }), /malformed/);
  }
});

test('Started belongs to the current attempt: paused zero persists, expiry and cleared work end it, new work restarts it', () => {
  const data = reply([
    { ...run(100, 100, '0'), stateFlags: 0 },
    run(101, 101, '0'),
    { ...run(102, 103, '0'), stateFlags: 0 },
    { ...run(104, 104, '800000'), broken: true },
    { ...run(105, 105), stateFlags: 0x100, bonusDays: 1, broken: true },
    { ...run(106, 107), stateFlags: 0 },
    { ...run(108, 108, '400001'), stateFlags: 0 },
    { ...run(109, 110, '0'), stateFlags: 0 },
    run(111, 111, '0'),
    { ...run(112, 112), stateFlags: 0 },
    { ...run(114, 114), stateFlags: 0 },
  ]);
  data.coverage = [{ firstHour: 100, lastHour: 112, complete: true, reason: null }, { firstHour: 114, lastHour: 114, complete: true, reason: null }];
  const encoded = data.pairs[0].runs;
  for (const index of [0, 5, 7]) encoded[index].started = null;
  for (const index of [1, 2, 3, 4]) encoded[index].started = { hour: 101, upperBound: true };
  encoded[6].started = { hour: 108, upperBound: false };
  for (const index of [8, 9]) encoded[index].started = { hour: 111, upperBound: false };
  encoded[10].started = { hour: 114, upperBound: true };
  data.pairs[0].breaks = [{ hour: 104, upperBound: false }];
  validateDecryptionReply(data);
  for (const at of [100, 106, 107, 109, 110, 113]) assert.deepEqual(raceRows(data, at, 'progress'), [], `no current attempt at ${at}`);
  const paused = raceRows(data, 103, 'progress')[0];
  assert.equal(paused.run.amountRaw, '0');
  assert.equal(cipherStatus(paused), 'Paused');
  assert.equal(paused.run.started?.hour, 101);
  assert.match(observationDate(paused.run.started, 103), /^by /);
  for (const [at, start] of [[108, 108], [111, 111], [112, 111], [114, 114]]) {
    const row = raceRows(data, at, 'progress')[0];
    assert.equal(row.run.started?.hour, start);
    assert.equal(decryptionTooltip(row, 'Recorded decryptor', 'Recorded target', 800).rows[1].value, observationDate(row.run.started, at));
  }
  assert.match(observationDate(raceRows(data, 114, 'progress')[0].run.started, 114), /^by /);
  for (const index of [5, 6, 7, 8, 10]) {
    const malformed = structuredClone(data);
    malformed.pairs[0].runs[index].started = { hour: 101, upperBound: false };
    assert.throws(() => validateDecryptionReply(malformed), /malformed/, `stale start at run ${index}`);
  }
});

test('an idle first read only establishes an exact later start after complete absence or an observed end', () => {
  const idle = (first: number, last: number, amount = '0') => ({ ...run(first, last, amount), stateFlags: 0, started: null });
  const complete = (firstHour: number, lastHour: number) => ({ firstHour, lastHour, complete: true, reason: null });
  const cases = [
    { name: 'recording start', runs: [idle(100, 102)], coverage: [complete(100, 103)], upperBound: true },
    { name: 'gap', runs: [run(100, 100), idle(102, 104)], coverage: [complete(100, 100), complete(102, 105)], upperBound: true },
    { name: 'complete prior hour', runs: [idle(101, 103)], coverage: [complete(100, 104)], upperBound: false },
    { name: 'partial prior hour', runs: [idle(101, 103)],
      coverage: [{ firstHour: 100, lastHour: 100, complete: false, reason: 'Interrupted read' }, complete(101, 104)], upperBound: true },
    { name: 'observed clear', runs: [run(100, 100), idle(101, 103)], coverage: [complete(100, 104)], upperBound: false },
    { name: 'observed expiry', runs: [{ ...run(100, 100), stateFlags: 0x100, bonusDays: 1, broken: true }, idle(101, 103, '400000')],
      coverage: [complete(100, 104)], upperBound: false },
  ];
  for (const scenario of cases) for (const scripted of [false, true]) {
    const previous = scenario.runs.at(-1)!;
    const startHour = previous.lastHour + 1;
    const nextAmount = scripted ? (BigInt(previous.startAmountRaw) + 1n).toString() : previous.startAmountRaw;
    const data = reply([...scenario.runs, { ...run(startHour, startHour, nextAmount), stateFlags: scripted ? 0 : 1 }]);
    data.coverage = scenario.coverage;
    data.pairs[0].runs.forEach((encoded, index) => { encoded.started = scenario.runs[index]?.started ?? null; });
    const expected = { hour: startHour, upperBound: scenario.upperBound };
    data.pairs[0].runs.at(-1)!.started = expected;
    if (scenario.name === 'observed expiry') data.pairs[0].breaks = [{ hour: 100, upperBound: true }];
    const label = `${scenario.name}, ${scripted ? 'scripted progress' : 'native start'}`;
    validateDecryptionReply(data);
    assert.deepEqual(raceRows(data, startHour, 'progress')[0].run.started, expected, label);
    const wrong = structuredClone(data);
    wrong.pairs[0].runs.at(-1)!.started!.upperBound = !scenario.upperBound;
    assert.throws(() => validateDecryptionReply(wrong), /malformed/, label);
  }
});

test('run validation rejects a hidden new start inside increasing post-expiry residual work', () => {
  const data = reply([{ ...run(100, 100), stateFlags: 0x100, bonusDays: 1, broken: true },
    { ...run(101, 125), stateFlags: 0, started: null, stepRawPerDay: '1' }]);
  data.pairs[0].runs[1].started = null;
  data.pairs[0].breaks = [{ hour: 100, upperBound: true }];
  data.coverage[0].lastHour = 125;
  assert.throws(() => validateDecryptionReply(data), /malformed/);
});

test('a new break after unknown inputs or a recording gap is an upper-bound observation', () => {
  const unknown = { ...run(101, 101), thresholdRaw: null, thresholdUnknown: 'target input missing', broken: null };
  const data = reply([run(100, 100), unknown,
    { ...run(102, 102, '800000'), broken: true },
    { ...run(104, 104, '800000'), broken: true }]);
  data.coverage = [{ firstHour: 100, lastHour: 102, complete: true, reason: null }, { firstHour: 104, lastHour: 104, complete: true, reason: null }];
  data.pairs[0].breaks = [{ hour: 102, upperBound: true }, { hour: 104, upperBound: true }];
  data.pairs[0].runs[3].started = { hour: 104, upperBound: true };
  validateDecryptionReply(data);
  assert.equal(raceRows(data, 103, 'progress').length, 0);
  const row = raceRows(data, 104, 'progress')[0];
  assert.equal(row.run.started?.hour, 104);
  assert.equal(row.broken?.hour, 104);
  assert.match(observationDate(row.broken, row.hour), /^by /);
});

test('tag history can change inside one numeric run and must preserve every gap', () => {
  const data = reply();
  data.pairs[0].tagRuns = [
    { ...data.pairs[0].tagRuns[0], lastHour: 100 },
    { firstHour: 101, lastHour: 102, ownerTag: 'Renamed decryptor | mod', targetTag: 'New target name' },
  ];
  validateDecryptionReply(data);
  assert.equal(data.pairs[0].runs.length, 1);
  assert.equal(raceRows(data, 100, 'progress')[0].tags.ownerTag, 'New decryptor|with spaces');
  assert.equal(raceRows(data, 101, 'progress')[0].tags.ownerTag, 'Renamed decryptor | mod');
  assert.equal(raceRows(data, 101, 'progress', new Set(['New target name']), 'target').length, 1);
  for (const tags of [[{ ...data.pairs[0].tagRuns[0], lastHour: 101 }, data.pairs[0].tagRuns[1]],
    [data.pairs[0].tagRuns[0]], [{ ...data.pairs[0].tagRuns[0], firstHour: 99, lastHour: 102 }]]) {
    assert.throws(() => validateDecryptionReply({ ...data, pairs: [{ ...data.pairs[0], tagRuns: tags }] }), /malformed/);
  }
});

test('unknown progress uses the exact decimal amount and keeps all five fractional digits when needed', () => {
  assert.equal(amountText(decoded(run())), '4');
  assert.equal(amountText(decoded(run(100, 102, '400001'))), '4.00001');
  assert.equal(amountText(decoded(run(100, 102, '-400001'))), '−4.00001');
  assert.equal(amountText(decoded(run(100, 102, '9223372036854775807'))), '92,233,720,368,547.75807');
});

test('row and tooltip wording share progress and status with their intended capitalization', () => {
  const active = decoded(run()), paused = { ...active, decrypting: 0 };
  const unknown = { ...paused, progressPercent: null };
  assert.equal(progressLabel(active), '50%');
  assert.equal(progressText(active), progressLabel(active));
  assert.equal(progressLabel(unknown), 'Progress unknown');
  assert.equal(progressText(unknown), `${progressLabel(unknown)} · ${amountText(unknown)}`);
  assert.equal(workingStatus(active), 'decrypting');
  assert.equal(workingStatus(active, true), 'Decrypting');
  assert.equal(workingStatus(paused), 'paused');
  assert.equal(workingStatus(paused, true), 'Paused');
  const data = reply([{ ...run(), broken: true, startAmountRaw: '800000' }]);
  data.pairs[0].breaks = [{ hour: 100, upperBound: true }];
  const row = raceRows(validateDecryptionReply(data), 100, 'progress')[0];
  assert.equal(cipherStatus(row), `${CIPHER_BROKEN} ${observationDate(row.broken, row.hour)}`);
});

test('endpoint filter is hidden for All while keeping its saved option for a narrowed selection', () => {
  const options = resolveOptions(OPTIONS, { countrySide: 'target' });
  assert.deepEqual(sectionFields(OPTIONS, 'countries', options, { kind: 'all' }), []);
  assert.deepEqual(sectionFields(OPTIONS, 'countries', options, { kind: 'tags', tags: ['Mod target'] }).map(field => field.key), ['countrySide']);
  assert.equal(options.countrySide, 'target');
});

test('a leaving race keeps its recorded content and hour while the current rows move on', () => {
  const data = reply([{ ...run(), broken: true, startAmountRaw: '800000' }]);
  data.pairs[0].breaks = [{ hour: 100, upperBound: true }];
  const row = raceRows(validateDecryptionReply(data), 101, 'progress')[0];
  const shown = nextRows([], [{ key: row.key, item: row }], 0, 450, true);
  const leaving = nextRows(shown, [], 100, 450)[0];
  assert.equal(leaving.phase, 'leaving');
  assert.equal(leaving.item, row);
  assert.equal(leaving.item.hour, 101);
  assert.equal(leaving.item.run.started?.hour, 100);
  assert.equal(leaving.item.broken?.hour, 100);
  assert.equal(decryptionTooltip(leaving.item, 'Old decryptor', 'Old target', 800).rows.at(-1)?.value,
    `Cipher broken ${observationDate(row.broken, 101)}`);
});

test('unrepresentable tags retain race data with distinct country identities', () => {
  const data = reply(); data.pairs[0].tagRuns[0] = { ...data.pairs[0].tagRuns[0], ownerTag: null, targetTag: null };
  data.pairs.push({ ...data.pairs[0], ownerId: '101' });
  validateDecryptionReply(data);
  const rows = raceRows(data, 100, 'decryptor');
  assert.equal(rows.length, 2);
  assert.equal(rows[0].run.amountRaw, '400000');
  assert.equal(new Set(rows.map(row => row.key)).size, 2);
  assert.equal(raceRows(data, 100, 'progress', new Set(['A']), 'target').length, 0);
  const malformed = reply();
  malformed.pairs[0].tagRuns[0].ownerTag = '';
  assert.throws(() => validateDecryptionReply(malformed), /malformed/);
});

test('bonus badge requires a broken row and retains the native signed active countdown', () => {
  assert.equal(activeBonusDays(decoded({ ...run(), bonusDays: 25 })), null);
  const unknown: DecryptionRun = { ...run(), thresholdRaw: null, thresholdUnknown: 'target inputs not recorded',
    stateFlags: 7 << 8, bonusDays: 23, broken: true };
  const data = reply([unknown]); data.pairs[0].breaks = [{ hour: 100, upperBound: true }];
  validateDecryptionReply(data);
  const tip = decryptionTooltip(raceRows(data, 100, 'progress')[0], 'Custom owner', 'Custom target', 800);
  assert.equal(tip.title, 'Custom owner → Custom target');
  assert.equal(tip.rows[0].value, 'Progress unknown · 4');
  assert.equal(tip.rows[2].value, '23');
  assert.equal(tip.rows[3].divider, true);
  assert.match(tip.rows[3].value!, /Cipher broken by/);
  assert.equal(activeBonusDays(decoded({ ...unknown, bonusDays: -1 })), -1);
  assert.equal(activeBonusDays(decoded({ ...unknown, stateFlags: 0 })), null);
  assert.equal(activeBonusDays({ ...decoded(unknown), broken: false }), null);
  assert.equal(activeBonusDays({ ...decoded(unknown), broken: null }), null);
});

test('progress order, decryptor grouping and either endpoint filter treat tags as opaque', () => {
  const data = reply();
  const add = (ownerId: string, targetId: string, ownerTag: string, targetTag: string, amount: string): DecryptionPair => ({
    ...data.pairs[0], ownerId, targetId, runs: [run(100, 102, amount)],
    tagRuns: [{ firstHour: 100, lastHour: 102, ownerTag, targetTag }],
  });
  data.pairs = [add('1', '2', 'A|B', 'C', '200000'), add('2', '3', 'A', 'B|C', '700000'), add('1', '3', 'A|B', 'B|C', '600000')];
  validateDecryptionReply(data);
  assert.deepEqual(raceRows(data, 100, 'progress').map(row => row.pair.ownerId), ['2', '1', '1']);
  assert.deepEqual(raceRows(data, 100, 'decryptor').map(row => row.tags.ownerTag), ['A', 'A|B', 'A|B']);
  assert.deepEqual(raceRows(data, 100, 'progress', new Set(['A|B']), 'decryptor').map(row => row.run.progressPercent), [75, 25]);
  assert.equal(raceRows(data, 100, 'progress', new Set(['B|C']), 'target').length, 2);
  assert.equal(new Set(raceRows(data, 100, 'progress').map(row => row.key)).size, 3);
});

test('broken rows sort first, then unknown progress follows known values with deterministic native tag ties', () => {
  const data = reply();
  const add = (ownerId: string, ownerTag: string, encoded: DecryptionRun): DecryptionPair => ({
    ...data.pairs[0], ownerId, runs: [encoded],
    tagRuns: [{ firstHour: 100, lastHour: 102, ownerTag, targetTag: 'Target' }],
  });
  const unknown = { ...run(100, 102, '0'), thresholdRaw: null, thresholdUnknown: 'Inputs not recorded', broken: null };
  const unknownBroken = { ...unknown, stateFlags: 0x101, broken: true };
  data.pairs = [add('1', '\u{10000}', { ...unknown, startAmountRaw: '900000000' }), add('2', '\ue000', unknown),
    add('3', 'aaa', run()), add('4', 'AAA', run()), add('5', 'Known zero', run(100, 102, '0')),
    { ...add('6', 'AAA unknown broken', unknownBroken), breaks: [{ hour: 100, upperBound: true }] }];
  validateDecryptionReply(data);
  const expected = ['AAA unknown broken', 'AAA', 'aaa', 'Known zero', '\ue000', '\u{10000}'];
  assert.deepEqual(raceRows(data, 100, 'progress').map(row => row.tags.ownerTag), expected);
  data.pairs.reverse();
  assert.deepEqual(raceRows(data, 100, 'progress').map(row => row.tags.ownerTag), expected);
  assert.deepEqual(raceRows(data, 100, 'decryptor').map(row => row.tags.ownerTag), ['AAA', 'AAA unknown broken', 'Known zero', 'aaa', '\ue000', '\u{10000}']);
});

test('progress ordering is total when native wrapping and non-positive thresholds disagree with exact ratios', () => {
  const data = reply();
  const add = (ownerId: string, ownerTag: string, amount: string, threshold: string): DecryptionPair => ({
    ...data.pairs[0], ownerId, runs: [{ ...run(100, 102, amount), thresholdRaw: threshold }],
    tagRuns: [{ firstHour: 100, lastHour: 102, ownerTag, targetTag: 'Target' }],
  });
  // The old comparator made A precede C by their displayed 100% and tag,
  // C precede B by displayed percent, and B precede A by its higher exact ratio.
  const a = add('1', 'A', '99500', '100000');
  const b = add('2', 'B', '900000000000000000', '901000000000000000');
  const c = add('3', 'C', '-1', '0');
  const permutations = [[a, b, c], [a, c, b], [b, a, c], [b, c, a], [c, a, b], [c, b, a]];
  for (const pairs of permutations) {
    data.pairs = pairs;
    validateDecryptionReply(data);
    for (const at of [100, 101, 102]) {
      const rows = raceRows(data, at, 'progress');
      assert.deepEqual(rows.map(row => row.tags.ownerTag), ['A', 'C', 'B']);
      assert.deepEqual(rows.map(row => row.run.progressPercent), [100, 100, 1]);
    }
  }
  data.pairs = [add('1', 'A exact low', '503', '1000'), add('2', 'Z exact high', '504', '1000'),
    add('3', 'A zero', '-1', '0'), add('4', 'B negative', '-11', '-10'), add('5', 'Z positive', '99500', '100000')];
  validateDecryptionReply(data);
  assert.deepEqual(raceRows(data, 100, 'progress').map(row => row.tags.ownerTag),
    ['Z positive', 'A zero', 'B negative', 'Z exact high', 'A exact low']);
});

test('5000-country sparse reply validates and windows; All persists and explicit country picks are bounded', () => {
  const data = reply();
  data.pairs = Array.from({ length: 5_000 }, (_, index) => ({ ...data.pairs[0], targetId: String(index + 1),
    tagRuns: [{ ...data.pairs[0].tagRuns[0], targetTag: `Mod country ${index} | spaces` }] }));
  validateDecryptionReply(data);
  const rows = raceRows(data, 100, 'progress');
  assert.equal(rows.length, 5_000);
  const range = visibleRange(rows.length, 100_000, 400, 52);
  assert.ok(range.end - range.start < 24);
  const widget = createWidget('decryption', 'main', 'decryption-test');
  assert.deepEqual(widget.countries, { kind: 'all' });
  assert.equal(definition.countries.max, 256);
  const template = defaultTemplate();
  template.widgets[0] = { ...widget, id: template.widgets[0].id };
  assert.deepEqual(parseTemplate(JSON.parse(JSON.stringify(template)))!.widgets[0].countries, { kind: 'all' });
  template.widgets[0].countries = { kind: 'tags', tags: rows.slice(0, 256).map(row => row.tags.targetTag!) };
  const restored = parseTemplate(JSON.parse(JSON.stringify(template)))!;
  const countrySelection = restored.widgets[0].countries;
  assert.equal(countrySelection.kind, 'tags');
  if (countrySelection.kind === 'tags') assert.equal(countrySelection.tags.length, 256);
  template.widgets[0].countries = { kind: 'tags', tags: rows.slice(0, 257).map(row => row.tags.targetTag!) };
  assert.equal(parseTemplate(JSON.parse(JSON.stringify(template))), null);
});

test('smart group selection changes with its recorded membership and keeps unknown apart from empty', () => {
  const groups = { factions: [{ id: '1', members: [{ tag: 'Mod target', firstHour: 100, lastHour: 101 },
    { tag: 'New member', firstHour: 102, lastHour: 104 }] }] } as CountryGroupReply;
  const runs = groupRuns(groups, { type: 'faction', id: '1' })!;
  assert.deepEqual(groupMembers(runs, 'hour', 100, 104), ['Mod target']);
  assert.deepEqual(groupMembers(runs, 'hour', 102, 104), ['New member']);
  assert.equal(groupMembers(runs, 'hour', 101, 104, Infinity, [{ firstHour: 101, lastHour: 101 }]), null);
  assert.deepEqual(groupMembers(runs, 'ever', 102, 104), ['Mod target', 'New member']);
});
