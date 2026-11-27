import { projectCountryFlow } from '../../families/flow/countryFlow.ts';
import { countryFlowPicture, flowProjectionOptions } from '../../families/flow/countryFlowPlan.ts';
import type { FlowWidgetDeclaration } from '../../families/flow/declaration.ts';
import type { FlowProjection } from '../../families/flow/model.ts';

export const licencesProjection: FlowProjection = {
  family: 'flow',
  kind: 'production-licences',
  project: (readView, options) => projectCountryFlow(readView, {
    leftHeader: 'Licensed from',
    rightHeader: 'Licensed to',
    unit: ['licence', 'licences'],
    groupLeftHeader: 'Licensors',
    groupRightHeader: 'Licensees',
    notRecorded: 'Licences were not recorded at this date.',
    none: 'No production licences at this date.',
    others: count => `${count} other ${count === 1 ? 'country' : 'countries'}`,
    ribbonTitle: (from, to) => `${from} licenses to ${to}`,
  }, options),
};

export const licencesDeclaration: FlowWidgetDeclaration = {
  family: 'flow',
  kind: 'production-licences',
  label: 'Production licences',
  subject: 'production licences',
  emptyCampaign: 'Load a campaign to see its production licences.',
  icon: 'factory',
  countries: { mode: 'multi', content: 'hour' },
  sources: (ctx, view) => countryFlowPicture(ctx, view, 'production-licences', false),
  projection: licencesProjection,
  projectionOptions: ctx => flowProjectionOptions(ctx, 'production-licences'),
  jumpToDate: true,
};
