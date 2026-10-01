import test from 'node:test';
import assert from 'node:assert/strict';
import type { ExperienceRead, ExperienceReply } from '../../generated/dashboard';
import { clipToSpans, runAt, segments } from '../../charts/series.ts';
import { tooltipText } from '../../charts/tooltip.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import { definition, OPTIONS } from './definition.ts';
import { BRANCHES, experienceRaw, experienceRuns, experienceTooltip, formatExperience, validateExperienceReply } from './experienceData.ts';
import type { ExperienceTooltipCountry } from './experienceData';

const Q = 32_768;
const read = (firstHour: number, lastHour: number, banked: [string, string, string], change: [string, string, string] | null, startsSegment = false): ExperienceRead => ({
  firstHour, lastHour, startsSegment, armyRaw: banked[0], navyRaw: banked[1], airRaw: banked[2],
  change: change && { armyRaw: change[0], navyRaw: change[1], airRaw: change[2] }, changeUnknown: change ? null : 'noMatchingRead',
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
  assert.match(tip.note ?? '', /Read /);
  assert.match(tip.note ?? '', /read once a day/);
  assert.match(tip.note ?? '', /24 hours earlier/);
  assert.match(tip.note ?? '', /spent.*cap/);
  assert.equal(tip.rows[0].icon, 'flag-at-hover');
  assert.equal(tip.rows[1].value, '+0.5');
  assert.equal(tip.rows[2].value, '0');
  assert.equal(tip.rows[3].value, 'Not in this recording');
  assert.equal(tip.title, experienceTooltip(REPLY, countries(), BRANCHES, 'change', 124, label, 2000).title);
});

test('unknown changes and absent hours say why without inventing zero; short tooltips retain spoken content', () => {
  const unknown = experienceTooltip(REPLY, countries().slice(0, 1), ['army'], 'change', 100, label, 2000);
  assert.equal(unknown.rows[0].value, 'Not known');
  assert.match(unknown.note ?? '', /No read exactly a day earlier/);
  const gap = experienceTooltip(REPLY, countries(), BRANCHES, 'banked', 160, label, 2000);
  assert.equal(gap.rows[0].value, 'Not recorded at this time');
  const short = experienceTooltip(REPLY, countries(), BRANCHES, 'change', 135, label, 100);
  assert.ok(short.spoken);
  assert.match(tooltipText(short), /Army −1.*Navy \+0.5.*Air 0/);
});

test('settings default to banked with all branches on and retain standard playback and axes', () => {
  const options = resolveOptions(OPTIONS, {});
  assert.deepEqual([options.measure, ...BRANCHES.map(branch => options[branch]), options.endLabels, options.afterPlayhead], ['banked', true, true, true, true, false]);
  assert.equal(resolveOptions(OPTIONS, { measure: 'daily-gain', navy: 'off' }).measure, 'banked');
  assert.equal(definition.defaultTime, 'campaign');
  assert.equal(definition.history, true);
  assert.equal(definition.countries.max, 16);
  assert.equal('aggregate' in definition.countries, false);
});
