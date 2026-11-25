import { breakdownAtHourModel, breakdownAtHourSource, validateBreakdownAtHour } from '../../families/breakdown/atHour.ts';
import type { BreakdownWidgetSpec } from '../../families/breakdown/declaration.ts';
import type { BreakdownModel, BreakdownProjection } from '../../families/breakdown/model.ts';
import type { BoundedReadView } from '../../families/plan.ts';
import { formatRational } from '../../families/valueFormat.ts';
import type { BombingDamageBreakdownReply, BombingDirection } from '../../generated/dashboard';

export interface BombingBreakdownOptions {
  readonly direction?: BombingDirection;
}

const validate = validateBreakdownAtHour<BombingDamageBreakdownReply>('bombing damage breakdown reply');

export const bombingDamageBreakdownProjection: BreakdownProjection<BombingBreakdownOptions> = {
  family: 'breakdown',
  kind: 'bombing-damage-breakdown',
  project(readView: BoundedReadView, options: BombingBreakdownOptions): BreakdownModel {
    const reply = readView.panel<BombingDamageBreakdownReply>('main');
    const dealt = (reply?.direction ?? options.direction) === 'dealt';
    return breakdownAtHourModel(Math.floor(readView.hour), reply, {
      unit: 'building levels',
      verb: dealt ? 'Inflicted' : 'Suffered',
      since: dealt ? 'Inflicted since' : 'Suffered since',
      nothing: 'bombing damage',
      notRecorded: 'Bombing damage was not recorded.',
      format: (numerator, denominator) => formatRational(numerator, denominator, { maxDecimals: 2 }),
    });
  },
};

export const bombingDamageBreakdown: BreakdownWidgetSpec<BombingBreakdownOptions> = {
  kind: 'bombing-damage-breakdown',
  label: 'Bombing damage breakdown',
  subject: 'bombing damage',
  emptyCampaign: 'Load a campaign to see bombing damage.',
  icon: 'sunburst',
  countries: { mode: 'single', choose: 'Choose a country to see its bombing damage.' },
  projection: bombingDamageBreakdownProjection,
  projectionOptions: ctx => ({
    direction: ctx.options.direction === 'dealt' ? 'dealt' : 'taken',
  }),
  sources: ctx => breakdownAtHourSource(
    ctx,
    'bombing-damage-breakdown',
    { direction: ctx.options.direction === 'dealt' ? 'dealt' : 'taken' },
    validate,
  ),
};
