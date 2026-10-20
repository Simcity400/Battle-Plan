import type { WidgetRenderer } from '../../dashboard/definitions';
import BalanceOfPowerWidget from './BalanceOfPowerWidget';
import { useBalanceChoices } from './useBalanceChoices';

export default { render: BalanceOfPowerWidget, useOptionChoices: useBalanceChoices } satisfies WidgetRenderer;
