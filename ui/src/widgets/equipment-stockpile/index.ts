import type { WidgetRenderer } from '../../dashboard/definitions';
import EquipmentStockpileWidget from './EquipmentStockpileWidget';
import StockpileCountryChip from './StockpileCountryChip';

export default { render: EquipmentStockpileWidget, countryChip: StockpileCountryChip } satisfies WidgetRenderer;
