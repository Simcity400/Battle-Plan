import { measureOf, ruleOf, territoryMeasureLabel, territoryNote, territorySeriesMeasure, type TerritoryMeasure, type TerritoryRule } from '../../dashboard/territoryMeasure.ts';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import {
  factionSelectionTop,
  factionTotalsSources,
  groupTitleForSelection,
  isAggregatedSelection,
  isFactionTotalsSelection,
  projectFactionTotalsRace,
  type FactionTotalsRaceOptions,
} from '../../families/bars/factionTotalsRace.ts';
import type { BarsModel, BarsProjection } from '../../families/bars/model.ts';
import {
  nextSeriesRaceChange,
  projectSeriesRace,
  seriesRaceSources,
  seriesRaceUnavailable,
  seriesRaceWarnings,
  type SeriesRaceBaseOptions,
} from '../../families/bars/seriesRace.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import { registerFamilyProjection } from '../../families/registry.ts';

export interface TerritoryRaceOptions {
  readonly mode: 'countries' | 'factions';
  readonly measure: TerritoryMeasure;
  readonly rule: TerritoryRule;
  readonly measureLabel: string;
  readonly scaleUnit: string;
  readonly note?: string;
  readonly series: SeriesRaceBaseOptions;
  readonly factions: FactionTotalsRaceOptions;
}

function unitFor(measure: TerritoryMeasure): string {
  if (measure === 'provinces') return 'Provinces';
  if (measure === 'victoryPoints') return 'Victory points';
  if (measure === 'area') return 'Map cells';
  return 'States';
}

export const territoryRaceProjection: BarsProjection<TerritoryRaceOptions> = registerFamilyProjection({
  kind: 'territory',
  family: 'bars',
  nextChange(readView, options) {
    if (options.mode === 'factions') {
      return nextSeriesRaceChange(readView.hour, {
        top: options.factions.top,
        candidates: [],
        membership: options.factions.membership,
        unobserved: options.factions.unobserved,
      });
    }
    return nextSeriesRaceChange(readView.hour, options.series);
  },
  project(readView: BoundedReadView, options: TerritoryRaceOptions): BarsModel {
    if (options.mode === 'factions') {
      return projectFactionTotalsRace(readView, options.factions);
    }
    return projectSeriesRace(readView, options.series, {
      decimals: 0,
      scaleUnit: options.scaleUnit,
      scaleFormat: { decimals: 0 },
      valueFormat: { decimals: 0 },
      valueLabel: options.measureLabel,
      emptyMessage: `No ${options.measureLabel.toLowerCase()} recorded at this date.`,
      ...(options.note ? { note: options.note } : {}),
    });
  },
}, 'race');

function territoryRaceSources(ctx: FamilyPlanContext): SourcePlan {
  const measure = measureOf(ctx.options.measure);
  const rule = ruleOf(ctx.options.rule);
  const seriesMeasure = territorySeriesMeasure(measure, rule);
  if (isFactionTotalsSelection(ctx.widget.countries)) {
    return factionTotalsSources('territory:race', ctx, 'territory', seriesMeasure);
  }
  return seriesRaceSources(ctx, { kind: 'territory', family: { kind: 'territory', measure: seriesMeasure } });
}

export const territoryRaceDeclaration: BarsWidgetDeclaration<TerritoryRaceOptions> = {
  family: 'bars',
  kind: 'territory',
  label: 'Territory',
  subject: 'territory',
  emptyCampaign: 'Load a campaign to see its territory.',
  icon: 'map',
  countries: {
    mode: 'multi',
    labels: true,
    colours: true,
    allowSlots: true,
    everyOnTop: true,
    factions: true,
    requireGroupsWhen: widget => widget.countries.kind === 'factions' || (widget.countries.kind === 'group' && widget.countries.group.type !== 'top'),
  },
  sources: territoryRaceSources,
  projection: territoryRaceProjection,
  projectionOptions: ctx => {
    const measure = measureOf(ctx.options.measure);
    const rule = ruleOf(ctx.options.rule);
    const seriesMeasure = territorySeriesMeasure(measure, rule);
    const measureLabel = territoryMeasureLabel(measure, rule);
    const scaleUnit = unitFor(measure);
    const note = territoryNote(measure);
    const useFactions = isFactionTotalsSelection(ctx.widget.countries);
    const aggregated = isAggregatedSelection(ctx.widget.countries);
    const top = ctx.countries.top;
    return {
      mode: useFactions ? 'factions' : 'countries',
      measure,
      rule,
      measureLabel,
      scaleUnit,
      ...(note ? { note } : {}),
      series: {
        top,
        all: ctx.countries.all,
        candidates: top == null && !ctx.countries.all ? ctx.countries.tags : [],
        holders: top == null && !ctx.countries.all ? ctx.countries.holders : null,
        membership: top == null && !ctx.countries.all ? ctx.countries.membership : null,
        unobserved: top == null && !ctx.countries.all ? ctx.countries.unobserved : [],
      },
      factions: {
        kind: 'territory',
        measure: seriesMeasure,
        measureLabel,
        scaleUnit,
        ...(note ? { note } : {}),
        aggregated,
        groupLabel: groupTitleForSelection(ctx.widget.countries),
        top: factionSelectionTop(ctx.widget.countries),
        membership: aggregated ? ctx.countries.membership : null,
        unobserved: aggregated ? ctx.countries.unobserved : [],
      },
    };
  },
  unavailable: ctx => {
    if (ctx.widget.countries.kind === 'factions') return null;
    return seriesRaceUnavailable(ctx);
  },
  warnings: (_ctx, followRefused) => seriesRaceWarnings(followRefused),
  jumpToDate: true,
  keyByBranch: true,
};
