import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import type { BarsModel, BarsProjection } from '../../families/bars/model.ts';
import { registerBarsProjection } from '../../families/bars/registry.ts';
import {
  nextSeriesRaceChange,
  projectSeriesRace,
  seriesRaceSources,
  seriesRaceUnavailable,
  seriesRaceWarnings,
  type SeriesRaceBaseOptions,
} from '../../families/bars/seriesRace.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import { OPTIONS } from './definition.ts';
import { measureName, rankingMeasure, SERIES, type OccupationMeasure, type ShownMeasure } from './occupationTrendData.ts';

export interface OccupationTrendRaceOptions extends SeriesRaceBaseOptions {
  readonly rank: OccupationMeasure;
  readonly digits: number;
}

function clampDigits(raw: unknown): number {
  const num = Number(raw);
  return Number.isFinite(num) ? Math.max(0, Math.min(5, Math.trunc(num))) : 2;
}

function resolveRank(options: Readonly<Record<string, unknown>>): OccupationMeasure {
  const shown: ShownMeasure = options.measure === 'compliance' || options.measure === 'both' ? options.measure : 'resistance';
  const ranking: OccupationMeasure = options.rankingMeasure === 'compliance' ? 'compliance' : 'resistance';
  return rankingMeasure(shown, ranking);
}

export const occupationTrendRaceProjection: BarsProjection<OccupationTrendRaceOptions> = registerBarsProjection({
  kind: 'occupation-trend',
  family: 'bars',
  nextChange(readView, options) {
    return nextSeriesRaceChange(readView.hour, options);
  },
  project(readView: BoundedReadView, options: OccupationTrendRaceOptions): BarsModel {
    const label = `Average ${measureName(options.rank).toLowerCase()}`;
    return projectSeriesRace(readView, options, {
      decimals: 5,
      valueFormat: { decimals: 0, maxDecimals: options.digits, percent: true },
      scaleFormat: { decimals: 0, maxDecimals: 1, percent: true },
      scaleUnit: label,
      valueLabel: label,
      emptyMessage: `No ${label.toLowerCase()} recorded at this date.`,
    });
  },
}, 'race');

function occupationTrendRaceSources(ctx: FamilyPlanContext): SourcePlan {
  const rank = resolveRank(ctx.options);
  return seriesRaceSources(ctx, {
    kind: 'occupation-trend',
    family: { kind: SERIES, measure: rank },
  });
}

export const occupationTrendRaceDeclaration: BarsWidgetDeclaration<OccupationTrendRaceOptions> = {
  family: 'bars',
  kind: 'occupation-trend',
  label: 'Occupation trend',
  subject: 'occupation',
  emptyCampaign: 'Load a campaign to see occupation trends.',
  icon: 'globe',
  optionFields: OPTIONS,
  countries: {
    mode: 'multi',
    labels: true,
    colours: true,
    allowSlots: true,
    everyOnTop: true,
    allTagsForLabels: true,
    requireGroupsWhen: widget => widget.countries.kind === 'group' && widget.countries.group.type !== 'top',
  },
  sources: occupationTrendRaceSources,
  projection: occupationTrendRaceProjection,
  projectionOptions: ctx => {
    const ranked = ctx.countries.top != null || ctx.countries.all;
    return {
      rank: resolveRank(ctx.options),
      digits: clampDigits(ctx.options.digits),
      candidates: ranked ? ctx.countries.every : ctx.countries.tags,
      holders: ranked ? null : ctx.countries.holders,
      membership: ranked ? null : ctx.countries.membership,
      unobserved: ranked ? [] : ctx.countries.unobserved,
      top: ctx.countries.top,
      all: ctx.countries.all,
    };
  },
  unavailable: seriesRaceUnavailable,
  warnings: (_ctx, followRefused) => seriesRaceWarnings(followRefused),
  jumpToDate: true,
  keyByBranch: true,
};
