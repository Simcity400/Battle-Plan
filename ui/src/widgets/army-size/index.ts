import type { WidgetRenderer } from '../../dashboard/definitions';
import ArmySizeWidget from './ArmySizeWidget';
import ArmyCountryChip from './ArmyCountryChip';

export default { render: ArmySizeWidget, countryChip: ArmyCountryChip } satisfies WidgetRenderer;
