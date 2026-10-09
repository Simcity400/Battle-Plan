import type { WidgetRenderer } from '../../dashboard/definitions';
import NavalLossesWidget from './NavalLossesWidget';
import NavalLossesCountryChip from './NavalLossesCountryChip';

export default { render: NavalLossesWidget, countryChip: NavalLossesCountryChip } satisfies WidgetRenderer;
