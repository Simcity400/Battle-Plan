import type { WidgetRenderer } from '../../dashboard/definitions';
import SupplyHealthWidget from './SupplyHealthWidget';
import SupplyCountryChip from './SupplyCountryChip';

export default { render: SupplyHealthWidget, countryChip: SupplyCountryChip } satisfies WidgetRenderer;
