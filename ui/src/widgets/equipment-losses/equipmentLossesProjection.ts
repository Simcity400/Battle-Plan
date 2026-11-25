import { breakdownAtHourModel, breakdownAtHourSource, validateBreakdownAtHour } from '../../families/breakdown/atHour.ts';
import type { BreakdownWidgetSpec } from '../../families/breakdown/declaration.ts';
import type { BreakdownModel, BreakdownProjection } from '../../families/breakdown/model.ts';
import type { BoundedReadView } from '../../families/plan.ts';
import { formatRational } from '../../families/valueFormat.ts';
import type { EquipmentLossesReply, LandSource } from '../../generated/dashboard';

export interface EquipmentLossesOptions {
  readonly landSource?: LandSource;
}

const validate = validateBreakdownAtHour<EquipmentLossesReply>('equipment losses breakdown reply');

export const equipmentLossesProjection: BreakdownProjection<EquipmentLossesOptions> = {
  family: 'breakdown',
  kind: 'equipment-losses',
  project(readView: BoundedReadView, _options: EquipmentLossesOptions): BreakdownModel {
    const reply = readView.panel<EquipmentLossesReply>('main');
    return breakdownAtHourModel(Math.floor(readView.hour), reply, {
      unit: 'units',
      verb: 'Lost',
      since: 'Lost since',
      nothing: 'equipment losses',
      notRecorded: 'Equipment losses were not recorded.',
      format: (numerator, denominator) => formatRational(numerator, denominator, { maxDecimals: 5 }),
    });
  },
};

export const equipmentLossesBreakdown: BreakdownWidgetSpec<EquipmentLossesOptions> = {
  kind: 'equipment-losses',
  label: 'Equipment losses',
  subject: 'equipment losses',
  emptyCampaign: 'Load a campaign to see equipment losses.',
  icon: 'sunburst',
  countries: { mode: 'single', choose: 'Choose a country to see its equipment losses.' },
  projection: equipmentLossesProjection,
  projectionOptions: ctx => ({
    landSource: ctx.options.landSource === 'combatLog' ? 'combatLog' : 'destruction',
  }),
  sources: ctx => breakdownAtHourSource(
    ctx,
    'equipment-losses',
    { landSource: ctx.options.landSource === 'combatLog' ? 'combatLog' : 'destruction' },
    validate,
  ),
};
