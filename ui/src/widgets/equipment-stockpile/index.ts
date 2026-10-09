import type { WidgetRenderer } from '../../dashboard/definitions';
import EquipmentStockpileWidget from './EquipmentStockpileWidget';
import StockpileCountryChip from './StockpileCountryChip';
import { useStockpileChoices } from './useEquipmentStockpile';

export default { render: EquipmentStockpileWidget, countryChip: StockpileCountryChip, useOptionChoices: useStockpileChoices } satisfies WidgetRenderer;
