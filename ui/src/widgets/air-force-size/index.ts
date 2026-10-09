import type { WidgetRenderer } from '../../dashboard/definitions';
import AirForceSizeWidget from './AirForceSizeWidget';
import { PartsCountryChip } from '../../dashboard/CountryChip';

export default { render: AirForceSizeWidget, countryChip: PartsCountryChip } satisfies WidgetRenderer;
