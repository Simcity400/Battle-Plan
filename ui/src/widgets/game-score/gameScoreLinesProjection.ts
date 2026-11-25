import { createCountryLinesView } from '../../families/time-chart/countryLines.ts';
import { scorePart } from './gameScoreData.ts';

export const { projection: gameScoreLinesProjection, declaration: gameScoreLinesDeclaration } = createCountryLinesView({
  kind: 'game-score',
  view: 'lines',
  title: 'Game score',
  subject: 'game scores',
  format: 'number',
  digits: () => 0,
  measure: options => ({ kind: 'country-game-scores', measure: scorePart(options.topMeasure), decimals: 5 }),
});
