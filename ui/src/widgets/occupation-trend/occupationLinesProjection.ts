import { createCountryLinesView } from '../../families/time-chart/countryLines.ts';

export const { projection: occupationLinesProjection, declaration: occupationLinesDeclaration } = createCountryLinesView({
  kind: 'occupation-trend',
  view: 'lines',
  title: 'Occupation trend',
  subject: 'occupation trend',
  format: 'percent',
  step: true,
  measure: options => {
    const shown = options.measure === 'both' ? options.rankingMeasure : options.measure;
    return { kind: 'occupier-totals', measure: shown === 'compliance' ? 'compliance' : 'resistance', decimals: 5 };
  },
});
