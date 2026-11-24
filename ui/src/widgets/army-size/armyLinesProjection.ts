import type { ArmySizeReply } from './armySizeData.ts';
import { createCountryLinesView } from '../../families/time-chart/countryLines.ts';
import { armyMeasure, armyMetadataParams, validateArmySizeReply } from './armySizeData.ts';

const SERIES_MEASURES = { manpower: 'Manpower', divisions: 'Divisions', strength: 'Strength', battalions: 'Battalions' } as const;

export const { projection: armyLinesProjection, declaration: armyLinesDeclaration } = createCountryLinesView<ArmySizeReply>({
  kind: 'army-size',
  view: 'lines',
  title: 'Army size',
  subject: 'army size',
  format: 'compact',
  aux: {
    product: 'army-size',
    params: track => ({ ...armyMetadataParams(track.branch.id) }),
    validate: validateArmySizeReply,
  },
  measure: (options, aux) => {
    if (!aux) return null;
    const measure = armyMeasure(options.measure, aux.measures);
    const owner = options.countFor === 'host' ? 'host' : 'owner';
    return { kind: 'army-totals', measure: `${owner}${SERIES_MEASURES[measure]}`, decimals: measure === 'strength' ? 5 : 0 };
  },
});
