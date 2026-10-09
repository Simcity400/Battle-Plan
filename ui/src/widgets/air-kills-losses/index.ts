import type { WidgetRenderer } from '../../dashboard/definitions';
import AirKillsLossesWidget from './AirKillsLossesWidget';
import { useAirLossChoices } from '../../dashboard/useMilitaryAirLedger';
import AirLossCountryChip from './AirLossCountryChip';
export default { render: AirKillsLossesWidget, useOptionChoices: useAirLossChoices, countryChip: AirLossCountryChip } satisfies WidgetRenderer;
