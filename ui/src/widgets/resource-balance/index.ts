import type { WidgetRenderer } from '../../dashboard/definitions';
import ResourceBalanceWidget from './ResourceBalanceWidget';
import { useResourceChoices } from './useResourceBalance';

export default { render: ResourceBalanceWidget, useOptionChoices: useResourceChoices } satisfies WidgetRenderer;
