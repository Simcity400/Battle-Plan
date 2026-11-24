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

const CONTRIBUTIONS_FAMILY = { kind: 'threat-contributions', measure: 'contribution' } as const;

export type WorldTensionRaceOptions = SeriesRaceBaseOptions;

export const worldTensionRaceProjection: BarsProjection<WorldTensionRaceOptions> = registerBarsProjection({
  kind: 'world-tension',
  family: 'bars',
  nextChange(readView, options) {
    return nextSeriesRaceChange(readView.hour, options);
  },
  project(readView: BoundedReadView, options: WorldTensionRaceOptions): BarsModel {
    return projectSeriesRace(readView, options, {
      decimals: 5,
      valueFormat: { decimals: 0, maxDecimals: 2, percent: true },
      scaleFormat: { decimals: 0, maxDecimals: 1, percent: true },
      scaleUnit: 'World tension',
      valueLabel: 'Contribution to world tension',
      emptyMessage: 'No contribution to world tension recorded at this date.',
      positiveOnly: true,
    });
  },
}, 'race');

function worldTensionRaceSources(ctx: FamilyPlanContext): SourcePlan {
  return seriesRaceSources(ctx, {
    kind: 'world-tension',
    family: CONTRIBUTIONS_FAMILY,
  });
}

export const worldTensionRaceDeclaration: BarsWidgetDeclaration<WorldTensionRaceOptions> = {
  family: 'bars',
  kind: 'world-tension',
  label: 'Contribution to world tension',
  subject: 'world tension',
  emptyCampaign: 'Load a campaign to see world tension.',
  icon: 'alert',
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
  sources: worldTensionRaceSources,
  projection: worldTensionRaceProjection,
  projectionOptions: ctx => {
    const ranked = ctx.countries.top != null || ctx.countries.all;
    return {
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
