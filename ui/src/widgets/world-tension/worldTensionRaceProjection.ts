import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import type { BarsModel, BarsProjection } from '../../families/bars/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import {
  projectSeriesRace,
  seriesRaceSelection,
  seriesRaceSources,
  seriesRaceUnavailable,
  seriesRaceWarnings,
  type SeriesRaceBaseOptions,
} from '../../families/bars/seriesRace.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';

const CONTRIBUTIONS_FAMILY = { kind: 'threat-contributions', measure: 'contribution' } as const;

export type WorldTensionRaceOptions = SeriesRaceBaseOptions;

export const worldTensionRaceProjection: BarsProjection<WorldTensionRaceOptions> = registerFamilyProjection({
  kind: 'world-tension',
  family: 'bars',
  nextChange: () => Infinity,
  project(readView: BoundedReadView, options: WorldTensionRaceOptions): BarsModel {
    return projectSeriesRace(readView, options, {
      valueFormat: { decimals: 0, maxDecimals: 2, percent: true },
      scaleFormat: { decimals: 0, maxDecimals: 1, percent: true },
      scaleUnit: 'World tension',
      valueLabel: 'Contribution to world tension',
      emptyMessage: 'No contribution to world tension recorded at this date.',
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
  countries: {
    mode: 'multi',
    allowSlots: true,
    everyOnTop: true,
    requireGroupsWhen: widget => widget.countries.kind === 'group' && widget.countries.group.type !== 'top',
  },
  sources: worldTensionRaceSources,
  projection: worldTensionRaceProjection,
  projectionOptions: seriesRaceSelection,
  unavailable: seriesRaceUnavailable,
  warnings: (_ctx, followRefused) => seriesRaceWarnings(followRefused),
  jumpToDate: true,
  keyByBranch: true,
};
