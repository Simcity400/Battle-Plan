import type { WidgetRenderer } from '../../dashboard/definitions';
import CountryChip from '../../dashboard/CountryChip';
import { convoyHealthDeclaration } from './convoyHealthProjection.ts';

export default { declaration: convoyHealthDeclaration, countryChip: CountryChip } satisfies WidgetRenderer;
