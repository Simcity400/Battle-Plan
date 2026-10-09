import type { WidgetRenderer } from '../../dashboard/definitions';
import ResourceBalanceWidget from './ResourceBalanceWidget';
import ResourceCountryChip from './ResourceCountryChip';
import { useResourceChoices } from './useResourceBalance';

export default { render: ResourceBalanceWidget, countryChip: ResourceCountryChip, useOptionChoices: useResourceChoices } satisfies WidgetRenderer;
