import { createSpendingMonthsView } from './spendingMonthsProjection.ts';

const dockyardMonths = createSpendingMonthsView({
  kind: 'dockyard-spending',
  name: 'Dockyard spending',
  measure: 'dockyard IC',
  subject: 'dockyard spending',
  repairs: true,
  domain: () => 'naval',
  measureTitle: () => 'Dockyard IC per month',
});

export const dockyardMonthsProjection = dockyardMonths.projection;
export const dockyardMonthsDeclaration = dockyardMonths.declaration;
