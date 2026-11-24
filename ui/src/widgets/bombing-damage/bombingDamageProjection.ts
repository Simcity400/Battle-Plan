import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import type { BarsProjection } from '../../families/bars/model.ts';
import { registerBarsProjection } from '../../families/bars/registry.ts';
import type { FamilyPlanContext } from '../../families/contract.ts';
import { militaryAirModel, militaryAirParams, militaryAirSources, type MilitaryAirAtOptions } from './militaryAirAt.ts';

export const bombingDamageProjection: BarsProjection<MilitaryAirAtOptions> = registerBarsProjection({
  kind: 'bombing-damage',
  family: 'bars',
  nextChange: () => Infinity,
  project: (readView, options) => militaryAirModel(readView, options),
});

const sources = (ctx: FamilyPlanContext) => militaryAirSources('bombing-damage', ctx, militaryAirParams(ctx, 'bombing', 'total'));

export const bombingDamageDeclaration: BarsWidgetDeclaration<MilitaryAirAtOptions> = {
  family: 'bars',
  kind: 'bombing-damage',
  label: 'Bombing damage suffered and inflicted',
  subject: 'bombing damage',
  emptyCampaign: 'Load a campaign to see bombing damage.',
  icon: 'target',
  countries: {
    mode: 'multi',
    labels: true,
    colours: true,
    allowSlots: true,
    requireGroupsWhen: widget => widget.countries.kind === 'group' && widget.countries.group.type !== 'top',
  },
  sources,
  projection: bombingDamageProjection,
  projectionOptions: ctx => ({
    family: 'bombing',
    scale: 10_000_000,
    digits: 7,
    leftHeader: 'Damage suffered',
    rightHeader: 'Damage inflicted',
    unit: 'Building levels',
    ratio: false,
    chosen: !ctx.countries.all && ctx.countries.top == null ? ctx.countries.tags : null,
  }),
  jumpToDate: true,
  keyByBranch: true,
};
