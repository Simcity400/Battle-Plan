import type { WidgetRenderer } from '../../dashboard/definitions';
import EquipmentProducedWidget from './EquipmentProducedWidget';
import ProducedCountryChip from './ProducedCountryChip';
import { useProducedTypeChoices } from './useProduced';

export default { render: EquipmentProducedWidget, countryChip: ProducedCountryChip, useOptionChoices: useProducedTypeChoices } satisfies WidgetRenderer;
