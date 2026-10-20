import type { WidgetRenderer } from '../../dashboard/definitions';
import EquipmentDesignsWidget, { useDesignChoices } from './EquipmentDesignsWidget';

export default { render: EquipmentDesignsWidget, useOptionChoices: useDesignChoices } satisfies WidgetRenderer;
