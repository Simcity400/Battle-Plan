import { artTooltipItem } from '../../families/art/adapters.ts';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import { countryRowPresentation, type BarsCaptionSide, type BarsDataRow, type BarsModel, type BarsOppositeRow, type BarsPickRow, type BarsProjection } from '../../families/bars/model.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { knownValue, notRecorded, partialValue, zeroValue, type Value } from '../../families/value.ts';
import { formatScalar, formatShare, notRecordedLabel } from '../../families/valueFormat.ts';
import type { WarContributionGroup, WarMeasure, WarMeasureKey, WarMeasureMember, WarsReply } from '../../generated/dashboard';
import { DEFAULT_MEASURE, MEASURE_LIST } from './definition.ts';
import { chosenGroup, leadOf, savedWar, sideCountries, warContributionSources, warCountries, warsAux, type WarChoiceOptions, type WarSide } from './warContributionPlan.ts';

export const MEASURE_VIEW = 'measure';

const UNATTRIBUTED = 'Part of this side’s casualties contribution is not attributed to a country';
const NO_SHARE = 'No part of this measure was recorded for either side';

export interface MeasureOptions extends WarChoiceOptions {
  readonly measure: WarMeasureKey;
}

function shareOf(member: WarMeasureMember, participation: boolean, missing: string): Value<number> {
  const permyriad = participation ? member.shareOfSide : member.shareOfWar;
  if (permyriad === null) return notRecorded(missing);
  if (member.partial) return partialValue(permyriad / 10000, UNATTRIBUTED);
  return member.value === '0' ? zeroValue() : knownValue(permyriad / 10000);
}

function numericOf(value: Value<number>): number | null {
  return value.kind === 'known' || value.kind === 'partial' ? Number(value.value) : value.kind === 'zero' ? 0 : null;
}

function rowsOf(readView: BoundedReadView, group: WarContributionGroup, measure: WarMeasure, sides: readonly [WarSide, WarSide], hour: number): BarsDataRow[] {
  const participation = measure.key === 'participation';
  const missing = group.reason ?? NO_SHARE;
  const membersOf = (side: WarSide) => measure[side].total === null
    ? sideCountries(group, side) : measure[side].members;
  const half = (side: WarSide, member: string | WarMeasureMember): BarsOppositeRow => {
    const tag = typeof member === 'string' ? member : member.tag;
    const who = countryRowPresentation(readView, tag, undefined, false, hour);
    const value = typeof member === 'string' ? notRecorded(missing) : shareOf(member, participation, missing);
    const numeric = numericOf(value);
    return {
      key: `${side}:${tag}`, subject: tag, label: who.label, labelMissing: who.labelMissing, labelLoading: who.labelLoading,
      media: who.media, color: who.color, value: { value, format: value.kind === 'zero' ? { decimals: 0 } : 'share' }, numeric,
      ...(typeof member !== 'string' ? { tooltip: {
        title: who.labelMissing ? notRecordedLabel('Name') : who.label, hour,
        get items() {
          const flag = artTooltipItem('Flag', who.media.art);
          return [
            { label: 'Recorded amount', value: formatScalar(BigInt(member.value), { decimals: 5 }) },
            value.kind === 'notRecorded' ? { label: 'Share', missingReason: value.reason } : { label: 'Share', value: formatShare(numeric!) },
            ...(flag ? [flag] : []),
          ];
        },
        ...(member.partial ? { note: UNATTRIBUTED } : {}),
      } } : {}),
    };
  };
  const left = membersOf(sides[0]);
  const right = membersOf(sides[1]);
  return Array.from({ length: Math.max(left.length, right.length) }, (_, index) => ({
    kind: 'data',
    ...(left[index] !== undefined ? half(sides[0], left[index]!) : { key: `${sides[0]}:none:${index}`, label: '' }),
    ...(right[index] !== undefined ? { rightSide: half(sides[1], right[index]!) } : {}),
  }));
}

function pickRows(readView: BoundedReadView, group: WarContributionGroup, sides: readonly [WarSide, WarSide], hour: number): BarsPickRow[] {
  const colour = (side: WarSide) => countryRowPresentation(readView, leadOf(group, side), undefined, false, hour).color;
  const leftColour = colour(sides[0]);
  const rightColour = colour(sides[1]);
  const byKey = new Map(group.measures.map(measure => [measure.key, measure]));
  return MEASURE_LIST.flatMap(item => {
    const measure = byKey.get(item.key);
    if (!measure) return [];
    const fraction = (side: WarSide) => (measure[side].shareOfWar === null ? null : measure[side].shareOfWar! / 10000);
    return [{
      key: item.key,
      label: item.label,
      left: { fraction: fraction(sides[0]), ...(leftColour ? { color: leftColour } : {}) },
      right: { fraction: fraction(sides[1]), ...(rightColour ? { color: rightColour } : {}) },
      ...(group.reason ? { tooltip: { title: item.label, fullText: item.label, hour, note: group.reason } } : {}),
      ...(item.key === 'participation' ? { divided: true } : {}),
    }];
  });
}

function captionSides(readView: BoundedReadView, group: WarContributionGroup, measure: WarMeasure, sides: readonly [WarSide, WarSide], hour: number): BarsCaptionSide[] {
  if (measure.key === 'participation') return [];
  return sides.map(side => {
    const who = countryRowPresentation(readView, leadOf(group, side), undefined, false, hour);
    const share = measure[side].shareOfWar;
    return { media: who.media, label: who.label, value: share === null ? 'Not recorded' : `${formatShare(share / 10000)}${measure[side].partial ? ' partial' : ''}` };
  });
}

export const warContributionMeasureProjection: BarsProjection<MeasureOptions> = registerFamilyProjection({
  kind: 'war-contribution',
  family: 'bars' as const,
  project(readView, options): BarsModel {
    const hour = readView.hour;
    const chosen = chosenGroup(readView, options);
    if (chosen.kind === 'loading') return { form: 'two-sided', twoSidedLayout: 'split-sides', hour, rows: [] };
    if (chosen.kind === 'blank') return { form: 'two-sided', twoSidedLayout: 'split-sides', hour, rows: [], empty: chosen.empty, emptyKind: chosen.emptyKind };
    const { group, reply, sides } = chosen;
    const measure = group.measures.find(item => item.key === options.measure) ?? group.measures.find(item => item.key === DEFAULT_MEASURE) ?? group.measures[0];
    if (!measure) return { form: 'two-sided', twoSidedLayout: 'split-sides', hour, rows: [], empty: 'This war’s contributions were not recorded at this time.', emptyKind: 'unavailable' };
    const listed = MEASURE_LIST.find(item => item.key === measure.key)!;
    const caption = measure.key === 'participation'
      ? `${listed.label} · Each country’s share of its side’s participation`
      : `${listed.label} · Each country’s share of all ${listed.whole} in this war`;
    return {
      form: 'two-sided',
      twoSidedLayout: 'split-sides',
      hour,
      dataHours: [reply.hour],
      rows: rowsOf(readView, group, measure, sides, reply.hour),
      rowHeight: 26,
      barHeight: 12,
      caption,
      captionSides: captionSides(readView, group, measure, sides, reply.hour),
      pickList: { rows: pickRows(readView, group, sides, reply.hour), selectedKey: measure.key },
      ...(reply.warnings.length ? { warnings: reply.warnings } : {}),
    };
  },
}, MEASURE_VIEW) as BarsProjection<MeasureOptions>;

const isMeasure = (value: unknown): value is WarMeasureKey => MEASURE_LIST.some(item => item.key === value);

export const warContributionMeasureDeclaration: BarsWidgetDeclaration<MeasureOptions, WarsReply> = {
  family: 'bars',
  kind: 'war-contribution',
  label: 'War contribution',
  subject: 'war contributions',
  emptyCampaign: 'Load a campaign to see war contributions.',
  icon: 'swords',
  countries: warCountries,
  aux: warsAux,
  sources: warContributionSources,
  projection: warContributionMeasureProjection,
  projectionOptions: ctx => ({ wars: ctx.aux ?? null, saved: savedWar(ctx.options), measure: isMeasure(ctx.options.measure) ? ctx.options.measure : DEFAULT_MEASURE }),
};
