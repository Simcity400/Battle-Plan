import test from 'node:test';
import assert from 'node:assert/strict';
import type { ExperienceRead, ExperienceReply } from '../../generated/dashboard';
import { clipToSpans, runAt, segments } from '../../charts/series.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { tooltipText } from '../../charts/tooltip.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import { definition, OPTIONS } from './definition.ts';
import { BRANCHES, experienceHolderAt, experienceHolderCount, experienceHolderEnd, experienceHolders, experienceLineEnd, experienceMembership, experienceName, experienceRaw, experienceRuns, experienceSelection, experienceTooltip, formatExperience, validateExperienceReply } from './experienceData.ts';
import { nameReadFrom } from '../../dashboard/nameReads.ts';
import { FAILED_FACT } from '../../dashboard/countryFacts.ts';
import type { ExperienceTooltipCountry } from './experienceData';

const Q = 32_768;
const read = (firstHour: number, lastHour: number, banked: [string, string, string], change: [string, string, string] | null, startsSegment = false): ExperienceRead => ({
  firstHour, lastHour, readHour: firstHour, startsSegment, armyRaw: banked[0], navyRaw: banked[1], airRaw: banked[2],
  change: change && { armyRaw: change[0], navyRaw: change[1], airRaw: change[2] }, changeUnknown: change ? null : 'noMatchingRead',
  gain: null, gainUnknown: 'noCompleteDay',
});
const REPLY: ExperienceReply = { branchId: '1', scale: Q, dayHours: 24, warnings: [], series: [
  { tag: 'opaque!tag', reads: [
    read(100, 123, ['32768', '0', '65536'], null, true),
    read(124, 147, ['0', '16384', '65536'], ['-32768', '16384', '0']),
    read(172, 195, ['81920', '32768', '0'], null, true),
    // Consecutive hours with another tag holder still break the line.
    read(196, 219, ['163840', '0', '0'], null, true),
    read(220, 243, ['196608', '0', '0'], ['32768', '0', '0']),
  ] }, { tag: 'missing', reads: [] },
] };
const colours = { army: 'blue', navy: 'orange', air: 'green' };
const countries = (from = 0, to = 1000): ExperienceTooltipCountry[] => REPLY.series.map(series => ({
  tag: series.tag, reads: clipToSpans(series.reads, [{ firstHour: from, lastHour: to }]), readHours: series.reads.map(read => read.firstHour), colours, recorded: series.reads.length > 0,
}));
const label = (tag: string) => ({ name: tag === 'opaque!tag' ? 'Recorded country' : tag, flag: 'flag-at-hover' });

test('experience replies validate scales, exact raw strings, changes and ordered reads', () => {
  assert.equal(validateExperienceReply(REPLY), REPLY);
  const bad = (change: (reply: ExperienceReply) => void) => {
    const copy = structuredClone(REPLY);
    change(copy);
    assert.throws(() => validateExperienceReply(copy), /malformed/);
  };
  bad(reply => { reply.scale = 0; });
  bad(reply => { reply.dayHours = 0.5; });
  bad(reply => { reply.series[0].reads.reverse(); });
  bad(reply => { reply.series[0].reads[1].firstHour = 123; });
  bad(reply => { reply.series[0].reads[0].armyRaw = '1.5'; });
  bad(reply => { reply.series[0].reads[0].navyRaw = null as unknown as string; });
  bad(reply => { reply.series[0].reads[0].airRaw = 0 as unknown as string; });
  bad(reply => { reply.series[0].reads[0].readHour = 101; });
  bad(reply => { reply.series[0].reads[1].changeUnknown = 'noMatchingRead'; });
  bad(reply => { reply.series[0].reads[0].changeUnknown = 'unsupported' as ExperienceRead['changeUnknown']; });
  assert.throws(() => validateExperienceReply(null));
});

test('banked values hold exactly and gaps and country changes stay broken', () => {
  const reads = REPLY.series[0].reads;
  const runs = experienceRuns(reads, Q, 'army', 'banked');
  assert.equal(experienceRaw(runAt(reads, 123)!, 'army', 'banked'), '32768');
  assert.equal(runAt(runs, 124)?.value, 0);
  assert.equal(runAt(runs, 160), null);
  assert.deepEqual(segments(runs, 100, 243).map(segment => segment.length), [2, 1, 2]);
  assert.deepEqual(experienceRuns(reads, Q, 'army', 'change').map(run => [run.firstHour, run.lastHour, run.value]), [[124, 147, -1], [220, 243, 1]]);
  assert.equal(experienceRaw(reads[0], 'army', 'change'), null);
  assert.equal(experienceRaw(reads[1], 'air', 'change'), '0');
});

test('experience validates signed banked widths independently of wider daily differences', () => {
  const reply = structuredClone(REPLY);
  reply.series[0].reads[0].armyRaw = '-9223372036854775808';
  reply.series[0].reads[1].armyRaw = '9223372036854775807';
  reply.series[0].reads[1].change!.armyRaw = '18446744073709551615';
  assert.equal(validateExperienceReply(reply), reply);
  reply.series[0].reads[0].armyRaw = '-9223372036854775809';
  assert.throws(() => validateExperienceReply(reply), /malformed/);
  reply.series[0].reads[0].armyRaw = '0';
  reply.series[0].reads[1].change!.armyRaw = '-18446744073709551616';
  assert.throws(() => validateExperienceReply(reply), /malformed/);
  reply.series[0].reads[1].change!.armyRaw = '0';
  reply.series.push(reply.series[0]);
  assert.throws(() => validateExperienceReply(reply), /malformed/);
});

test('amounts rescale with the supplied scale once, including signed changes and large raws', () => {
  assert.equal(formatExperience('81920', Q), '2.5');
  assert.equal(formatExperience('-81920', Q, 'change'), '−2.5');
  assert.equal(formatExperience('16384', Q, 'change'), '+0.5');
  assert.equal(formatExperience('0', Q, 'change'), '0');
  assert.equal(formatExperience('15', 10), '1.5');
  assert.equal(formatExperience('9007199254740993', Q), '274,877,906,944');
});

test('tooltips date the original read after range and membership clipping, and name countries at hover', () => {
  const tip = experienceTooltip(REPLY, countries(130, 140), BRANCHES, 'change', 135, label, 2000);
  assert.match(tooltipText(tip), /Recorded country: Army −1/);
  assert.doesNotMatch(tip.note ?? '', /Read |read once a day/);
  assert.match(tip.note ?? '', /Net change includes spending/);
  assert.equal(tip.subtitle, undefined);
  assert.equal(tip.rows[0].countryFlag?.flag, 'flag-at-hover');
  assert.equal(tip.rows[1].value, '+0.5');
  assert.equal(tip.rows[2].value, '0');
  assert.equal(tip.rows[3].value, 'Not in this recording');
  assert.notEqual(tip.title, experienceTooltip(REPLY, countries(), BRANCHES, 'change', 124, label, 2000).title);
});

test('unknown changes and absent hours say why without inventing zero; short tooltips retain spoken content', () => {
  const unknown = experienceTooltip(REPLY, countries().slice(0, 1), ['army'], 'change', 100, label, 2000);
  assert.equal(unknown.rows[0].value, 'Not known');
  assert.match(unknown.note ?? '', /A full day of data is unavailable/);
  const gap = experienceTooltip(REPLY, countries(), BRANCHES, 'banked', 160, label, 2000);
  assert.equal(gap.rows[0].value, 'Not recorded at this time');
  const short = experienceTooltip(REPLY, countries(), BRANCHES, 'change', 135, label, 100);
  assert.ok(short.spoken);
  assert.match(tooltipText(short), /Army −1.*Navy \+0.5.*Air 0/);
});

test('native country flag read status remains available for known values and missing hours', () => {
  const named = (tag: string) => ({ name: tag, flagReason: `Flag unavailable for ${tag}`, flagPending: tag === 'missing' });
  for (const hour of [124, 160]) {
    const tip = experienceTooltip(REPLY, countries(), ['army'], 'banked', hour, named, 2000);
    assert.deepEqual(tip.rows[0].countryFlag, named('opaque!tag'));
    assert.deepEqual(tip.rows[1].countryFlag, named('missing'));
    assert.equal(tip.rows[0].value, hour === 124 ? '0' : 'Not recorded at this time');
    assert.equal(tip.rows[1].value, 'Not in this recording');
  }
});

test('settings default to banked with all branches on and retain standard playback and axes', () => {
  const options = resolveOptions(OPTIONS, {});
  assert.deepEqual([options.measure, ...BRANCHES.map(branch => options[branch]), options.endLabels, options.afterPlayhead], ['banked', true, true, true, true, false]);
  assert.equal(resolveOptions(OPTIONS, { measure: 'daily-gain', navy: 'off' }).measure, 'banked');
  assert.equal(definition.defaultTime, 'follow');
  assert.equal(definition.history, true);
  assert.equal(definition.countries.max, Number.MAX_SAFE_INTEGER);
  assert.equal('aggregate' in definition.countries, false);
});

test('daily gain preserves capped awards and exact wider sums separately from negative net change', () => {
  const reply = structuredClone(REPLY);
  const awarded = reply.series[0].reads[1];
  awarded.gain = { armyRaw: '524288', navyRaw: '0', airRaw: (24n * ((1n << 63n) - 1n)).toString() };
  awarded.gainUnknown = null;
  assert.equal(validateExperienceReply(reply), reply);
  assert.equal(experienceRaw(awarded, 'army', 'gain'), '524288');
  assert.equal(experienceRaw(awarded, 'army', 'change'), '-32768');
  assert.equal(experienceRuns(reply.series[0].reads, Q, 'army', 'gain')[0].value, 16);
  const plotted = [{ ...countries()[0], reads: reply.series[0].reads }];
  const tip = experienceTooltip(reply, plotted, ['army'], 'gain', 124, label, 2000);
  assert.equal(tip.rows[0].value, '16');
  assert.match(tip.note ?? '', /Gain before spending/);
  const missing = experienceTooltip(reply, plotted, ['army'], 'gain', 100, label, 2000);
  assert.equal(missing.rows[0].value, 'Not recorded');
  assert.match(missing.note ?? '', /A full day of data is unavailable/);
  awarded.gain.airRaw = '-1';
  assert.throws(() => validateExperienceReply(reply), /malformed/);
});

test('experience end names belong to the last drawn holder and never a later name or metadata fallback', () => {
  const reads = REPLY.series[0].reads;
  const runs = experienceRuns(reads, Q, 'army', 'banked');
  const requests: number[] = [];
  const named = (hour: number) => { requests.push(hour); return nameReadFrom(hour < 196 ? 'Earlier holder' : 'Later holder', true); };
  const ended = experienceLineEnd('opaque!tag', reads, runs, 'army', 'banked', 100, 160, named)!;
  assert.equal(ended.hour, 147);
  assert.equal(ended.name, 'Earlier holder');
  assert.deepEqual(requests, [147]);
  const replaced = experienceLineEnd('opaque!tag', reads, runs, 'army', 'banked', 100, 219, named)!;
  assert.equal(replaced.name, 'Later holder');
  assert.equal(experienceLineEnd('opaque!tag', reads, runs, 'army', 'banked', 150, 160, named), null);
  for (const value of [undefined, null, FAILED_FACT] as const) {
    const unavailable = experienceLineEnd('opaque!tag', reads, runs, 'army', 'banked', 100, 160, () => nameReadFrom(value, true))!;
    assert.equal(unavailable.name, value === undefined ? 'Loading name…' : value === FAILED_FACT ? 'Couldn’t load country name.' : 'Country name unavailable');
    assert.equal(experienceName('opaque!tag', nameReadFrom(value, true)), unavailable.name);
    assert.equal(unavailable.raw, '0');
  }
  assert.equal(experienceName('opaque!tag', nameReadFrom('', true)), '');
  assert.equal(experienceName('opaque!tag', nameReadFrom('opaque!tag', true)), 'opaque!tag');
});

test('experience tooltip keeps dated flag state without name-status notes', () => {
  for (const nameRead of [undefined, null, FAILED_FACT]) {
    const named = (tag: string) => ({ name: tag, known: nameRead === undefined ? undefined : false,
      nameFailed: nameRead === FAILED_FACT, flagPending: false, flagReason: 'No recorded flag' });
    const tip = experienceTooltip(REPLY, countries(), ['army'], 'banked', 124, named, 2000);
    assert.equal(tip.note, '');
    assert.deepEqual(tip.rows[0].countryFlag, named('opaque!tag'));
    assert.equal(tip.rows[0].value, '0');
  }
});

test('renamed held values keep the supplying read date in their tooltip', () => {
  const reply = structuredClone(REPLY);
  reply.series = [{ tag: 'renamed', identities: [{ firstHour: 110, lastHour: 147, slot: 7, countryId: 9001 }],
    reads: [{ ...reply.series[0].reads[0], firstHour: 110, lastHour: 123, readHour: 100 }] }];
  assert.equal(validateExperienceReply(reply), reply);
  const plotted = reply.series.map(series => ({ tag: series.tag, reads: series.reads,
    readHours: series.reads.map(read => read.readHour), colours, recorded: true }));
  const tip = experienceTooltip(reply, plotted, ['army'], 'banked', 115, label, 2000);
  assert.equal(tip.title, experienceTooltip(REPLY, countries().slice(0, 1), ['army'], 'banked', 115, label, 2000).title);
  assert.doesNotMatch(tip.note ?? '', /Read/);
  assert.equal(tip.rows[0].value, '1');
  reply.series[0].identities![0].countryId = -1;
  assert.throws(() => validateExperienceReply(reply), /malformed/);
});

test('smart groups keep literal tags and missing hourly membership never becomes unbounded history', () => {
  const reply: ExperienceReply = { ...REPLY, series: [
    { tag: 'A', identities: [{ firstHour: 0, lastHour: 11, slot: 1, countryId: 7 }], reads: [read(0, 11, ['32768', '0', '0'], null, true)] },
    { tag: 'B', identities: [{ firstHour: 12, lastHour: 23, slot: 1, countryId: 7 }], reads: [{ ...read(12, 23, ['32768', '0', '0'], null), readHour: 0 }] },
  ] };
  assert.deepEqual(experienceSelection(reply, ['A'], false).map(series => series.tag), ['A', 'B']);
  for (const membership of [null, new Map([['A', [{ firstHour: 0, lastHour: 11 }]]])]) {
    const shown = experienceSelection(reply, ['A'], true);
    assert.deepEqual(shown.map(series => series.tag), ['A']);
    const spans = experienceMembership(membership, 'A');
    const runs = spans === null ? shown[0].reads : clipToSpans(shown[0].reads, spans);
    assert.equal(runAt(runs, 12), null);
  }
  assert.deepEqual(experienceMembership(new Map(), 'A'), []);
  assert.deepEqual(clipToSpans(reply.series[0].reads, experienceMembership(new Map(), 'A')!), []);
});

test('one renamed holder has one tooltip and one end per branch, with the dated current alias', () => {
  const plotted: ExperienceTooltipCountry[] = [
    { tag: 'A', identities: [{ firstHour: 0, lastHour: 11, slot: 1, countryId: 7 }], reads: [read(0, 11, ['32768', '0', '0'], null, true)], readHours: [0], colours, recorded: true },
    { tag: 'B', identities: [{ firstHour: 12, lastHour: 23, slot: 1, countryId: 7 }], reads: [{ ...read(12, 23, ['32768', '0', '0'], null), readHour: 0 }], readHours: [12], colours, recorded: true },
  ];
  const holders = experienceHolders(plotted);
  assert.equal(holders.length, 1);
  assert.equal(experienceHolderCount(plotted), 1);
  const selected = holders.map(holder => experienceHolderAt(holder, 15));
  assert.equal(selected[0].tag, 'B');
  const labels: string[] = [];
  const tip = experienceTooltip(REPLY, selected, BRANCHES, 'banked', 15, tag => { labels.push(tag); return { name: tag }; }, 2000);
  assert.deepEqual(tip.rows.map(row => row.label), ['Army', 'Navy', 'Air']);
  assert.deepEqual(labels, ['B']);
  assert.equal(tip.rows[0].value, '1');
  const endAt = (branch: typeof BRANCHES[number], to: number) => experienceHolderEnd(
    holders[0].parts.map(country => ({ country, runs: experienceRuns(country.reads, Q, branch, 'banked') })),
    branch, 'banked', 0, to, (_hour, tag) => nameReadFrom(tag, true));
  const ends = BRANCHES.map(branch => endAt(branch, 23)!);
  assert.equal(ends.length, 3);
  assert.ok(ends.every(end => end.tag === 'B' && end.hour === 23));
  assert.equal(endAt('army', 10)?.tag, 'A');
});

test('holder gaps and reused tags stay distinct without borrowing a replacement name or flag', () => {
  const plotted: ExperienceTooltipCountry[] = [
    { tag: 'A', identities: [{ firstHour: 0, lastHour: 10, slot: 1, countryId: 7 }, { firstHour: 12, lastHour: 23, slot: 2, countryId: 8 }],
      reads: [read(0, 10, ['32768', '0', '0'], null, true), read(12, 23, ['65536', '0', '0'], null, true)], readHours: [0, 12], colours, recorded: true },
    { tag: 'B', identities: [{ firstHour: 16, lastHour: 23, slot: 1, countryId: 7 }],
      reads: [read(16, 23, ['98304', '0', '0'], null)], readHours: [16], colours, recorded: true },
  ];
  const holders = experienceHolders(plotted);
  assert.equal(holders.length, 2);
  const knownReads: string[] = [];
  const at = holders.map(holder => experienceHolderAt(holder, 14));
  assert.deepEqual(at.map(country => [country.tag, country.identityKnown]), [['A', false], ['A', true]]);
  const tip = experienceTooltip(REPLY, at, ['army'], 'banked', 14, tag => {
    knownReads.push(tag); return { name: 'Replacement', flag: 'replacement flag', known: true };
  }, 2000);
  assert.deepEqual(knownReads, ['A']);
  assert.equal(tip.rows[0].value, 'Not recorded at this time');
  assert.equal(tip.rows[0].countryFlag?.flag, undefined);
  assert.equal(tip.rows[1].value, '2');
  assert.equal(tip.rows[1].countryFlag?.flag, 'replacement flag');
  assert.deepEqual(holders.map(holder => experienceHolderAt(holder, 18).tag), ['B', 'A']);
});

test('experience holder projection keeps all 5000 synthetic identities', () => {
  const plotted = Array.from({ length: 5000 }, (_, index) => ({ tag: `country ${index}`,
    identities: [{ firstHour: 0, lastHour: 1, slot: index, countryId: index + 1 }], reads: [read(0, 1, ['1', '0', '0'], null, true)],
    readHours: [0], colours, recorded: true }));
  assert.equal(experienceHolderCount(plotted), 5000);
  assert.equal(experienceHolders(plotted).length, 5000);
});


test('staggered comparison reads keep exact values without read-date notes', () => {
  const plotted = [100, 104].map((hour, index) => ({ tag: ['A', 'B'][index],
    reads: [read(hour, 120, [index === 0 ? '32768' : '65536', '0', '0'], null, true)],
    readHours: [hour], colours, recorded: true }));
  const tip = experienceTooltip(REPLY, plotted, ['army'], 'banked', 110, tag => ({ name: `Country ${tag}`, known: true }), 2000);
  assert.equal(tip.title, formatDateTime(110));
  assert.deepEqual(tip.rows.map(row => row.label), ['Country A: Army', 'Country B: Army']);
  assert.deepEqual(tip.rows.map(row => row.value), ['1', '2']);
  assert.doesNotMatch(tip.note ?? '', /Read/);
  const gap = experienceTooltip(REPLY, [{ ...plotted[0], reads: [] }, plotted[1]], ['army'], 'banked', 110,
    tag => ({ name: `Country ${tag}`, known: true }), 2000);
  assert.equal(gap.rows[0].value, 'Not recorded at this time');
  assert.equal(gap.rows[1].value, '2');
  assert.doesNotMatch(gap.note ?? '', /Read/);
});
