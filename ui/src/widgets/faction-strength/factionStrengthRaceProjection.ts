import { measureOf, type TerritoryMeasure } from '../../dashboard/territoryMeasure.ts';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import {
  branchFactionIds,
  factionSelectionTop,
  factionTotalsSources,
  groupTitleForSelection,
  isAggregatedSelection,
  projectFactionTotalsRace,
  type FactionTotalsRaceOptions,
} from '../../families/bars/factionTotalsRace.ts';
import type { BarsModel, BarsProjection } from '../../families/bars/model.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { factionStrengthSource } from './factionStrengthData.ts';

function territoryUnit(measure: TerritoryMeasure): string {
  if (measure === 'provinces') return 'Provinces';
  if (measure === 'victoryPoints') return 'Victory points';
  if (measure === 'area') return 'Map cells';
  return 'States';
}

export const factionStrengthRaceProjection: BarsProjection<FactionTotalsRaceOptions> = registerFamilyProjection({
  kind: 'faction-strength',
  family: 'bars',
  project(readView: BoundedReadView, options: FactionTotalsRaceOptions): BarsModel {
    return projectFactionTotalsRace(readView, options);
  },
}, 'race');

function factionStrengthRaceSources(ctx: FamilyPlanContext): SourcePlan {
  const source = factionStrengthSource(ctx.options);
  const plan = factionTotalsSources('faction-strength:race', ctx, source.kind, source.measure);
  return plan.entries.length ? plan : { ...plan, refused: ctx.countries.reason ?? 'No faction strength is available for this selection.' };
}

export const factionStrengthRaceDeclaration: BarsWidgetDeclaration<FactionTotalsRaceOptions> = {
  family: 'bars',
  kind: 'faction-strength',
  label: 'Faction strength',
  subject: 'factions',
  emptyCampaign: 'Load a campaign to see faction strength.',
  icon: 'shield',
  countries: {
    mode: 'multi',
    content: 'hour',
    allowSlots: true,
    factions: true,
  },
  sources: factionStrengthRaceSources,
  projection: factionStrengthRaceProjection,
  projectionOptions: (ctx, extra) => {
    const source = factionStrengthSource(ctx.options);
    const aggregated = isAggregatedSelection(ctx.widget.countries);
    const scaleUnit = source.kind === 'territory'
      ? territoryUnit(measureOf(ctx.options.territoryMeasure))
      : 'Factories';
    return {
      kind: source.kind,
      measure: source.measure,
      measureLabel: source.label,
      scaleUnit,
      ...(source.note ? { note: source.note } : {}),
      aggregated,
      groupLabel: groupTitleForSelection(ctx.widget.countries),
      top: factionSelectionTop(ctx.widget.countries),
      factionIds: branchFactionIds(ctx.countries.factions),
      surface: extra.surface,
    };
  },
  unavailable: ctx => {
    if (ctx.widget.countries.kind === 'factions') return null;
    return ctx.countries.reason;
  },
  warnings: (_ctx, followRefused) => (followRefused ? [followRefused] : []),
  jumpToDate: true,
  keyByBranch: true,
};
