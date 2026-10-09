import type { WidgetRenderer } from '../../dashboard/definitions';
import NuclearArsenalWidget from './NuclearArsenalWidget';
import NuclearCountryChip from './NuclearCountryChip';

export default { render: NuclearArsenalWidget, countryChip: NuclearCountryChip } satisfies WidgetRenderer;
