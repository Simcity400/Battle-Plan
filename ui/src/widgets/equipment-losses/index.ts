import LossesCountryChip from './LossesCountryChip';
import type { WidgetRenderer } from '../../dashboard/definitions';
import EquipmentLossesWidget from './EquipmentLossesWidget';

export default { countryChip: LossesCountryChip, render: EquipmentLossesWidget } satisfies WidgetRenderer;
