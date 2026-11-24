import type { FamilyViewRenderer } from '../../dashboard/definitions';
import CountryChip from '../../dashboard/CountryChip';
import { convoyHealthDeclaration } from './convoyHealthProjection.ts';

export default { declaration: convoyHealthDeclaration, countryChip: CountryChip } satisfies FamilyViewRenderer;
