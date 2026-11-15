import type { WidgetRenderer } from '../../dashboard/definitions';
import PoliticalPowerSpendingWidget from './PoliticalPowerSpendingWidget';
import { useSpendingChoices } from '../../dashboard/usePowerSpending.ts';

export default { render: PoliticalPowerSpendingWidget,
  useOptionChoices: (widget, runtime) => useSpendingChoices(widget, runtime, 'Political'),
} satisfies WidgetRenderer;
