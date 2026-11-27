import { colour } from '../../charts/palette.ts';
import { runAt } from '../../charts/series.ts';
import type { TimeChartModel, ChartInlinePoint, ChartMissingSpan } from '../../families/time-chart/model.ts';
import { hasMeasuredValue, knownValue, notApplicable, notRecorded, partialValue, type Value } from '../../families/value.ts';
import { formatDateOnly, formatScalar, formatScaledPercent } from '../../families/valueFormat.ts';
import type { WarContributionHistoryReply, WarHistoryContributor, WarMeasureKey, WarObservation } from '../../generated/dashboard';
import { MEASURE_LIST } from './definition.ts';
import { observationAt } from '../wars/warsData.ts';

const UNATTRIBUTED = 'Part of this side’s casualties contribution is not attributed to a country';
const coverage = new WeakMap<WarObservation, { listed: readonly Set<string>[]; reason: string }>();

export const HISTORY_SCALES = ['Recorded amount', 'Share of side', 'Share of war'] as const;

export function permyriad(part: bigint | null, whole: bigint | null): bigint | null {
  if (part === null || whole === null || whole <= 0n) return null;
  const rounded = ((part < 0n ? -part : part) * 20_000n + whole) / (whole * 2n);
  return rounded <= 0xffff_ffffn ? rounded : null;
}

function stepAt<T extends readonly [number, ...unknown[]]>(runs: readonly T[], hour: number): T | undefined {
  let low = 0, high = runs.length;
  while (low < high) {
    const middle = (low + high) >>> 1;
    if (runs[middle]![0] <= hour) low = middle + 1; else high = middle;
  }
  return runs[low - 1];
}

export function memberAt(person: WarHistoryContributor, hour: number) {
  const membership = stepAt(person.membership, hour);
  return membership && membership[1] >= hour ? membership : undefined;
}

function exact(runs: readonly [number, string][], hour: number): bigint | null {
  const word = stepAt(runs, hour)?.[1];
  return word === undefined ? null : BigInt(word);
}

export function readContribution(history: WarContributionHistoryReply, person: WarHistoryContributor, measure: number, hour: number): readonly Value<bigint>[] {
  const gap = runAt(history.gaps, hour);
  const membership = memberAt(person, hour);
  const all = (state: Value<bigint>) => [state, state, state];
  if (gap) return all(notRecorded(gap.reason));
  if (!membership) return all(notApplicable('This country was not in the war'));
  const side = membership[2] === 0 ? 0 : 1;
  const ownGap = runAt(history.sides[side].missing, hour);
  if (ownGap) return all(notRecorded(ownGap.reason));
  const observation = observationAt(history.war, hour);
  const group = observation?.contributionGroup;
  let subset = group && coverage.get(observation);
  if (group && !subset) {
    const listed = group.map(tags => new Set(tags));
    const outside = new Set([...observation.viewpointSide, ...observation.opposingSide].filter(tag => !listed[0]!.has(tag) && !listed[1]!.has(tag))).size;
    subset = { listed, reason: outside ? `Figures cover only the countries listed in the recorded group. ${outside} ${outside === 1 ? 'country on the war roster is' : 'countries on the war roster are'} outside this group.` : '' };
    coverage.set(observation, subset);
  }
  if (subset && !subset.listed[side]!.has(membership[3])) return all(notRecorded(subset.reason || 'This country is outside the recorded group'));
  const amount = exact(person.amounts[measure]!, hour);
  const ownTotal = exact(history.sides[side].totals[measure]!, hour);
  const other = side === 0 ? 1 : 0;
  const otherGap = runAt(history.sides[other].missing, hour);
  const otherTotal = exact(history.sides[other].totals[measure]!, hour);
  const part = [subset?.reason, history.measureKeys[measure] === 'casualties' && runAt(person.partialCasualties, hour) && UNATTRIBUTED].filter(Boolean).join(' ');
  const value = (raw: bigint | null, reason: string) => raw === null ? notRecorded(reason) : part ? partialValue(raw, part) : knownValue(raw);
  return [
    value(amount, 'The contributor’s amount was not recorded'),
    value(permyriad(amount, ownTotal), 'This side has no recorded total for this measure'),
    history.measureKeys[measure] === 'participation' ? notApplicable('Participation is already a share of the side')
      : value(permyriad(amount, ownTotal === null || otherGap || otherTotal === null ? null : ownTotal + otherTotal),
        otherGap?.reason ?? 'The war has no recorded total for this measure'),
  ];
}

export function readingText(value: Value<bigint>, scale: number): string {
  if (!hasMeasuredValue(value)) return `${value.kind === 'notApplicable' ? '' : 'Not recorded. '}${value.reason}`;
  const text = scale === 0 ? formatScalar(value.value, { decimals: 5, fixedDecimals: true }) : formatScaledPercent(BigInt(value.value), 100n);
  return value.kind === 'partial' ? `${text} partial. ${value.reason}` : text;
}

export function historyChart(history: WarContributionHistoryReply, people: readonly WarHistoryContributor[], measure: WarMeasureKey, scale: number, surface: string): TimeChartModel {
  const index = history.measureKeys.indexOf(measure);
  const { firstHour, lastHour, observations, participants, capitulations } = history.war;
  const until = Math.max(lastHour, history.gaps.at(-1)?.lastHour ?? lastHour);
  const series = people.map((person, ordinal) => {
    const dated = [...history.gaps, ...history.sides.flatMap(side => side.missing), ...person.partialCasualties, ...observations];
    const edges = new Set([firstHour, ...dated.flatMap(run => [run.firstHour, run.lastHour + 1]),
      ...person.membership.flatMap(([first, last]) => [first, last + 1]), ...person.amounts[index]!.map(run => run[0]),
      ...(scale ? history.sides.flatMap(side => side.totals[index]!.map(run => run[0])) : [])]);
    const hours = [...edges].filter(hour => firstHour <= hour && hour <= until).sort((a, b) => a - b);
    const missingSpans: ChartMissingSpan[] = [], partialSpans: ChartMissingSpan[] = [];
    const denominator = scale === 0 ? 100_000n : 100n;
    const points: ChartInlinePoint[] = [];
    hours.forEach((hour, i) => {
      const read = readContribution(history, person, index, hour)[scale]!;
      const span = { firstHour: hour, lastHour: (hours[i + 1] ?? until + 1) - 1 };
      if (read.kind === 'notRecorded') missingSpans.push({ ...span, reason: read.reason });
      if (read.kind === 'partial') partialSpans.push({ ...span, reason: read.reason });
      const previous = points.at(-1);
      if (!hasMeasuredValue(read) && previous && previous.value !== null && previous.hour < hour - 1) points.push({ ...previous, hour: hour - 1 });
      points.push(hasMeasuredValue(read) ? { hour, value: Number(read.value) / Number(denominator), raw: BigInt(read.value), denominator } : { hour, value: null });
    });
    const end = points.at(-1)!;
    if (end.value !== null && end.hour < until) points.push({ ...end, hour: until });
    const name = person.membership.at(-1)![3];
    const color = colour(surface, ordinal);
    return { id: `${person.slot}:${person.id}`, key: name, name, color, role: 'primary' as const, step: true, points, missingSpans, partialSpans,
      absentReason: 'This country was not in the war', identities: person.membership.map(([firstHour, lastHour, , name]) => ({ firstHour, lastHour, name, color })) };
  });
  const includes = (tag: string, hour: number) => people.some(person => memberAt(person, hour)?.[3] === tag);
  const markers = [
    ...participants.flatMap((run, i) => [
      ...(includes(run.tag, run.firstHour) ? [{ id: `join-${i}`, hour: run.firstHour, title: `${run.tag} ${run.joinedBy ? 'in war by' : 'joined'} ${formatDateOnly(run.firstHour)}` }] : []),
      ...(run.lastHour < lastHour && includes(run.tag, run.lastHour) ? [{
        id: `leave-${i}`, hour: run.lastHour, title: `${run.tag} ${run.leftBy ? 'last seen' : 'left'} ${formatDateOnly(run.lastHour)}`,
      }] : []),
    ]),
    ...capitulations.filter(event => includes(event.tag, event.hour)).map((event, i) => ({ id: `capitulation-${i}`, hour: event.hour, title: `${event.tag} capitulated` })),
  ];
  return { kind: 'war-contribution-history', measureTitle: MEASURE_LIST.find(item => item.key === measure)!.label, mode: 'lines', hour: until,
    extent: [Math.min(firstHour, until - 1), until], yAxis: scale ? { format: 'percent' } : { format: 'number', exactDecimals: 5 },
    series, gaps: history.gaps, markers, legend: series.length > 1 ? series.map(({ id, name, color }) => ({ id, label: name, color })) : undefined };
}
