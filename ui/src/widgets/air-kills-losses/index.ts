import type { WidgetRenderer } from '../../dashboard/definitions';
import AirKillsLossesWidget from './AirKillsLossesWidget';
import AirLossCountryChip from './AirLossCountryChip';
export default { render: AirKillsLossesWidget, countryChip: AirLossCountryChip } satisfies WidgetRenderer;
