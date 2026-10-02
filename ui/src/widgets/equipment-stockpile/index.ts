import type { WidgetRenderer } from '../../dashboard/definitions';
import EquipmentStockpileWidget from './EquipmentStockpileWidget';
import { useStockpileChoices } from './useEquipmentStockpile';

export default { render: EquipmentStockpileWidget, useOptionChoices: useStockpileChoices } satisfies WidgetRenderer;
