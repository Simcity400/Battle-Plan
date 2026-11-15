import type { WidgetRenderer } from '../../dashboard/definitions';
import { PartsCountryChip } from '../../dashboard/CountryChip';
import NuclearArsenalWidget from './NuclearArsenalWidget';

export default { render: NuclearArsenalWidget, countryChip: PartsCountryChip } satisfies WidgetRenderer;
