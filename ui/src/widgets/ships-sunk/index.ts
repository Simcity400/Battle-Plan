import type { WidgetRenderer } from '../../dashboard/definitions';
import ShipsSunkWidget from './ShipsSunkWidget';
import { useShipClassChoices } from './useShipsSunk';

export default { render: ShipsSunkWidget, useOptionChoices: useShipClassChoices } satisfies WidgetRenderer;
