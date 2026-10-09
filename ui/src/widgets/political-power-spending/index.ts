import type { WidgetRenderer } from '../../dashboard/definitions';
import PoliticalPowerSpendingWidget from './PoliticalPowerSpendingWidget';
import { useSpendingChoices } from '../../dashboard/usePoliticsSpending';

export default { render: PoliticalPowerSpendingWidget,
  useOptionChoices: (widget, runtime) => useSpendingChoices(widget, runtime, 'Political'),
} satisfies WidgetRenderer;
