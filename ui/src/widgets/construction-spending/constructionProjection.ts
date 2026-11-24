import { formatAmount } from '../../charts/valueAxis.ts';
import { icDecimals } from '../../dashboard/icDecimals.ts';
import { breakdownAtHourModel, breakdownAtHourSource, validateBreakdownAtHour } from '../../families/breakdown/atHour.ts';
import type { BreakdownWidgetSpec } from '../../families/breakdown/declaration.ts';
import type { BreakdownModel, BreakdownProjection } from '../../families/breakdown/model.ts';
import type { BoundedReadView } from '../../families/plan.ts';
import type { ConstructionBreakdownReply } from '../../generated/dashboard';
import { OPTIONS } from './definition.ts';

export interface ConstructionSpendingOptions {
  readonly digits: number;
}

const validate = validateBreakdownAtHour<ConstructionBreakdownReply>('construction spending breakdown');

const plural = (count: number, one: string) => `${count.toLocaleString('en-US')} ${one}${count === 1 ? '' : 's'}`;

export const constructionProjection: BreakdownProjection<ConstructionSpendingOptions> = {
  family: 'breakdown',
  kind: 'construction-spending',
  project(readView: BoundedReadView, options: ConstructionSpendingOptions): BreakdownModel {
    const reply = readView.panel<ConstructionBreakdownReply>('main');
    const model = breakdownAtHourModel(Math.floor(readView.hour), reply, {
      unit: 'IC',
      verb: 'Used',
      since: 'Used since',
      nothing: 'construction',
      notRecorded: 'Construction was not recorded.',
      format: (raw, decimals) => formatAmount(raw, decimals, Math.min(decimals, options.digits)),
    });
    const notes = [
      ...(reply?.beforeFirstRead ? [`Output of ${plural(reply.beforeFirstRead, 'project')} before the recording began is not counted.`] : []),
      ...(reply?.unattributed ? [`Output of ${plural(reply.unattributed, 'project')} not traced to a country is not counted.`] : []),
    ];
    return notes.length ? { ...model, warnings: [...(model.warnings ?? []), ...notes] } : model;
  },
};

export const constructionBreakdown: BreakdownWidgetSpec<ConstructionSpendingOptions> = {
  kind: 'construction-spending',
  label: 'Construction spending',
  subject: 'construction spending',
  emptyCampaign: 'Load a campaign to see construction spending.',
  icon: 'sunburst',
  optionFields: OPTIONS,
  countries: { mode: 'single', choose: 'Choose a country to see its construction spending.' },
  projection: constructionProjection,
  projectionOptions: ctx => ({ digits: icDecimals(ctx.options, 5) }),
  sources: ctx => breakdownAtHourSource(ctx, 'construction-breakdown', {}, validate),
};
