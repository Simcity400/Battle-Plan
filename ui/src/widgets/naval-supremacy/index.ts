import type { WidgetRenderer } from '../../dashboard/definitions';
import CountryChip from '../../dashboard/CountryChip';
import NavalSupremacyWidget from './NavalSupremacyWidget';

export default { render: NavalSupremacyWidget, countryChip: CountryChip } satisfies WidgetRenderer;
