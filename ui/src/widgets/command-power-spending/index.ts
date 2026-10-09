import type { WidgetRenderer } from '../../dashboard/definitions';
import CommandPowerSpendingWidget from './CommandPowerSpendingWidget';
import { useSpendingChoices } from '../../dashboard/usePoliticsSpending';

export default { render: CommandPowerSpendingWidget,
  useOptionChoices: (widget, runtime) => useSpendingChoices(widget, runtime, 'Command'),
} satisfies WidgetRenderer;
