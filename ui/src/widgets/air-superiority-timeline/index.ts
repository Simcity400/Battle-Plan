import type { FamilyViewRenderer } from '../../dashboard/definitions';
import CountryChip from '../../dashboard/CountryChip';
import { useAirChoices } from '../../dashboard/useAirSuperiority';
import { airTimelineDeclaration } from './airTimelineProjection.ts';
export default { declaration: airTimelineDeclaration, countryChip: CountryChip, useOptionChoices: useAirChoices } satisfies FamilyViewRenderer;
