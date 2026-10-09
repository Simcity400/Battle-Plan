import StabilityCountryChip from './StabilityCountryChip';
import StabilityWidget from './StabilityWidget';
import type { WidgetRenderer } from '../../dashboard/definitions';

export default { countryChip: StabilityCountryChip, render: StabilityWidget } satisfies WidgetRenderer;
