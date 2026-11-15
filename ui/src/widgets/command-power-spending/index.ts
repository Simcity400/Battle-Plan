import type { WidgetRenderer } from '../../dashboard/definitions';
import CommandPowerSpendingWidget from './CommandPowerSpendingWidget';
import { useSpendingChoices } from '../../dashboard/usePowerSpending.ts';

export default { render: CommandPowerSpendingWidget,
  useOptionChoices: (widget, runtime) => useSpendingChoices(widget, runtime, 'Command'),
} satisfies WidgetRenderer;
