import { projectCountryFlow } from '../../families/flow/countryFlow.ts';
import { countryFlowPlan, FLOW_PER_SIDE } from '../../families/flow/countryFlowPlan.ts';
import type { FlowWidgetDeclaration } from '../../families/flow/declaration.ts';
import type { FlowProjection } from '../../families/flow/model.ts';

const EMPTY_OPTIONS: Readonly<Record<string, never>> = {};

export const licencesProjection: FlowProjection = {
  family: 'flow',
  kind: 'production-licences',
  project: readView => projectCountryFlow(readView, {
    leftHeader: 'Licensed from',
    rightHeader: 'Licensed to',
    groupLeftHeader: 'Licensors',
    groupRightHeader: 'Licensees',
    notRecorded: 'Licences were not recorded at this date.',
    none: 'No production licences at this date.',
    others: count => `${count} other ${count === 1 ? 'country' : 'countries'}`,
    ribbonTitle: (from, to) => `${from} licenses to ${to}`,
  }),
};

export const licencesDeclaration: FlowWidgetDeclaration = {
  family: 'flow',
  kind: 'production-licences',
  label: 'Production licences',
  subject: 'production licences',
  emptyCampaign: 'Load a campaign to see its production licences.',
  icon: 'factory',
  countries: { mode: 'multi', labels: false, colours: false },
  sources: ctx => countryFlowPlan(ctx, 'production-licences', { limit: FLOW_PER_SIDE }),
  projection: licencesProjection,
  projectionOptions: () => EMPTY_OPTIONS,
  jumpToDate: true,
};
