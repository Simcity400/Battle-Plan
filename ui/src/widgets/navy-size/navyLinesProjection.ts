import { OPTIONS } from './definition.ts';
import { createCountryLinesView } from '../../families/time-chart/countryLines.ts';
import { navyTotals } from './navyData.ts';

export const { projection: navyLinesProjection, declaration: navyLinesDeclaration } = createCountryLinesView({
  kind: 'navy-size',
  view: 'lines',
  title: 'Navy size',
  subject: 'navy size',
  optionFields: OPTIONS,
  format: 'number',
  measure: options => navyTotals(options),
});
