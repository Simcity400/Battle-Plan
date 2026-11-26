import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import type { BarsProjection } from '../../families/bars/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { FamilyPlanContext } from '../../families/contract.ts';
import { contentKey, contentSet, type OptionValue } from '../../dashboard/options.ts';
import { groupSelection } from '../../families/groupSelection.ts';
import { lossCausesModel, militaryAirModel, militaryAirParams, militaryAirSources, type MilitaryAirAtOptions, type MilitaryAirFilters } from '../bombing-damage/militaryAirAt.ts';

export const airKillsLossesProjection: BarsProjection<MilitaryAirAtOptions> = registerFamilyProjection({
  kind: 'air-kills-losses',
  family: 'bars',
  nextChange: () => Infinity,
  project: (readView, options) => militaryAirModel(readView, options),
}, 'butterfly');

export const airKillsLossesPairedProjection: BarsProjection<MilitaryAirAtOptions> = registerFamilyProjection({
  kind: 'air-kills-losses',
  family: 'bars',
  nextChange: () => Infinity,
  project: (readView, options) => militaryAirModel(readView, options),
}, 'paired');

export const airLossCausesProjection: BarsProjection<MilitaryAirAtOptions> = registerFamilyProjection({
  kind: 'air-kills-losses',
  family: 'bars',
  nextChange: () => Infinity,
  project: (readView, options) => lossCausesModel(readView, options),
}, 'breakdown');

const numbers = (value: OptionValue | undefined) => contentSet(value ?? '').map(contentKey).map(Number).filter(Number.isInteger);
const filtersOf = (ctx: FamilyPlanContext): MilitaryAirFilters => ({ regions: numbers(ctx.options.regions), causes: numbers(ctx.options.causes) });

function sources(ctx: FamilyPlanContext) {
  const metric = ctx.options.measure === 'dealt' ? 'dealt' : 'taken';
  return militaryAirSources('air-kills-losses', ctx, militaryAirParams(ctx, 'airLosses', metric, filtersOf(ctx)));
}

export const airKillsLossesDeclaration: BarsWidgetDeclaration<MilitaryAirAtOptions> = {
  family: 'bars',
  kind: 'air-kills-losses',
  label: 'Own aircraft lost and enemy aircraft destroyed',
  subject: 'aircraft losses',
  emptyCampaign: 'Load a campaign to see aircraft losses.',
  icon: 'target',
  countries: {
    mode: 'multi',
    allowSlots: true,
  },
  sources,
  projection: airKillsLossesProjection,
  projectionOptions: (ctx, extra) => ({
    surface: extra.surface,
    family: 'airLosses',
    scale: 100_000,
    digits: 5,
    shownDecimals: 0,
    leftHeader: 'Own losses',
    rightHeader: 'Enemy aircraft destroyed',
    unit: 'Aircraft',
    ratio: true,
    none: ctx.options.measure === 'dealt' ? 'No enemy aircraft destroyed yet.' : 'No aircraft lost yet.',
    group: ctx.countries.top == null && !ctx.countries.all && groupSelection(ctx.widget.countries) !== null,
    filters: filtersOf(ctx),
  }),
  jumpToDate: true,
  keyByBranch: true,
};

export const airKillsLossesPairedDeclaration: BarsWidgetDeclaration<MilitaryAirAtOptions> = {
  ...airKillsLossesDeclaration,
  projection: airKillsLossesPairedProjection,
  projectionOptions: (ctx, extra) => ({
    ...airKillsLossesDeclaration.projectionOptions(ctx, extra),
    leftHeader: 'Own losses',
    rightHeader: 'Destroyed',
    paired: { solid: 'Destroyed (solid)', outline: 'Own losses (outline)' },
  }),
};

export const airLossCausesDeclaration: BarsWidgetDeclaration<MilitaryAirAtOptions> = {
  ...airKillsLossesDeclaration,
  label: 'Own aircraft lost by cause',
  sources: ctx => militaryAirSources('air-loss-causes', ctx, militaryAirParams(ctx, 'airLosses', 'taken', filtersOf(ctx))),
  projection: airLossCausesProjection,
  projectionOptions: (ctx, extra) => ({ ...airKillsLossesDeclaration.projectionOptions(ctx, extra), none: 'No aircraft lost yet.' }),
};
