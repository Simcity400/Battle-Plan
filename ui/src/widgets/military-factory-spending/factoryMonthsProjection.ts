import { createSpendingMonthsView } from '../dockyard-spending/spendingMonthsProjection.ts';

const factoryMonths = createSpendingMonthsView({
  kind: 'military-factory-spending',
  name: 'Military factory spending',
  measure: 'military factory IC',
  subject: 'military factory spending',
  repairs: false,
  domain: options => (options.equipment === 'land' ? 'land' : 'air'),
  measureTitle: options => (options.equipment === 'land' ? 'Land factory IC per month' : 'Air factory IC per month'),
});

export const factoryMonthsProjection = factoryMonths.projection;
export const factoryMonthsDeclaration = factoryMonths.declaration;
