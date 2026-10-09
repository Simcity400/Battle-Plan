import type { WidgetRenderer } from '../../dashboard/definitions';
import FuelWidget from './FuelWidget';
import FuelCountryChip from './FuelCountryChip';

export default { render: FuelWidget, countryChip: FuelCountryChip } satisfies WidgetRenderer;
