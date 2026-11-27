import { projectCountryFlow } from '../../families/flow/countryFlow.ts';
import { countryFlowPlan, flowPerSide, flowViewSubject } from '../../families/flow/countryFlowPlan.ts';
import type { FlowWidgetDeclaration } from '../../families/flow/declaration.ts';
import type { FlowProjection } from '../../families/flow/model.ts';

const EMPTY_OPTIONS: Readonly<Record<string, never>> = {};

export const licencesProjection: FlowProjection = {
  family: 'flow',
  kind: 'production-licences',
  project: readView => projectCountryFlow(readView, {
    leftHeader: 'Licensed from',
    rightHeader: 'Licensed to',
    unit: ['licence', 'licences'],
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
  countries: { mode: 'multi', content: 'hour' },
  sources: (ctx, view) => countryFlowPlan(ctx, 'production-licences', { limit: flowPerSide(view) }, flowViewSubject(ctx)),
  projection: licencesProjection,
  projectionOptions: () => EMPTY_OPTIONS,
  jumpToDate: true,
};
