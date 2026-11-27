import { projectCountryFlow } from '../../families/flow/countryFlow.ts';
import { countryFlowPicture, flowProjectionOptions } from '../../families/flow/countryFlowPlan.ts';
import type { FlowWidgetDeclaration } from '../../families/flow/declaration.ts';
import type { FlowProjection } from '../../families/flow/model.ts';

export const volunteersProjection: FlowProjection = {
  family: 'flow',
  kind: 'volunteers',
  project: (readView, options) => projectCountryFlow(readView, {
    leftHeader: 'Hosted from',
    rightHeader: 'Sent to',
    unitNote: 'divisions',
    unit: ['division', 'divisions'],
    groupLeftHeader: 'Senders',
    groupRightHeader: 'Hosts',
    notRecorded: 'Volunteers and expeditionary forces were not recorded at this date.',
    none: 'No volunteers or expeditionary forces at this date.',
    others: count => `${count} other ${count === 1 ? 'country' : 'countries'}`,
    ribbonTitle: (from, to) => `${from} → ${to}`,
  }, options),
};

export const volunteersDeclaration: FlowWidgetDeclaration = {
  family: 'flow',
  kind: 'volunteers',
  label: 'Volunteers & expeditionaries',
  subject: 'volunteers and expeditionary forces',
  emptyCampaign: 'Load a campaign to see its volunteers and expeditionary forces.',
  icon: 'flows',
  countries: { mode: 'multi', content: 'hour' },
  sources: (ctx, view) => countryFlowPicture(ctx, view, 'volunteer-flow', true),
  projection: volunteersProjection,
  projectionOptions: ctx => flowProjectionOptions(ctx, 'volunteer-flow'),
  jumpToDate: true,
};
