import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import type { BarsProjection } from '../../families/bars/model.ts';
import { registerBarsProjection } from '../../families/bars/registry.ts';
import type { FamilyPlanContext } from '../../families/contract.ts';
import { contentKey, contentSet, type OptionValue } from '../../dashboard/options.ts';
import { militaryAirModel, militaryAirParams, militaryAirSources, type MilitaryAirAtOptions } from '../bombing-damage/militaryAirAt.ts';
import { OPTIONS } from './definition.ts';

export const airKillsLossesProjection: BarsProjection<MilitaryAirAtOptions> = registerBarsProjection({
  kind: 'air-kills-losses',
  family: 'bars',
  nextChange: () => Infinity,
  project: (readView, options) => militaryAirModel(readView, options),
}, 'butterfly');

const numbers = (value: OptionValue | undefined) => contentSet(value ?? '').map(contentKey).map(Number).filter(Number.isInteger);

function sources(ctx: FamilyPlanContext) {
  const metric = ctx.options.measure === 'dealt' ? 'dealt' : 'taken';
  return militaryAirSources('air-kills-losses', ctx, militaryAirParams(ctx, 'airLosses', metric, { regions: numbers(ctx.options.regions), causes: numbers(ctx.options.causes) }));
}

export const airKillsLossesDeclaration: BarsWidgetDeclaration<MilitaryAirAtOptions> = {
  family: 'bars',
  kind: 'air-kills-losses',
  label: 'Own aircraft lost and enemy aircraft destroyed',
  subject: 'aircraft losses',
  emptyCampaign: 'Load a campaign to see aircraft losses.',
  icon: 'target',
  optionFields: OPTIONS,
  countries: {
    mode: 'multi',
    labels: true,
    colours: true,
    allowSlots: true,
    requireGroupsWhen: widget => widget.countries.kind === 'group' && widget.countries.group.type !== 'top',
  },
  sources,
  projection: airKillsLossesProjection,
  projectionOptions: ctx => ({
    family: 'airLosses',
    scale: 100_000,
    digits: 5,
    leftHeader: 'Own losses',
    rightHeader: 'Enemy aircraft destroyed',
    unit: 'Aircraft',
    ratio: true,
    chosen: !ctx.countries.all && ctx.countries.top == null ? ctx.countries.tags : null,
  }),
  jumpToDate: true,
  keyByBranch: true,
};
