import type { WidgetRenderer } from '../../dashboard/definitions';
import NavySizeWidget from './NavySizeWidget';
import { PartsCountryChip } from '../../dashboard/CountryChip';

export default { render: NavySizeWidget, countryChip: PartsCountryChip } satisfies WidgetRenderer;
