import type { WidgetRenderer } from '../../dashboard/definitions';
import TradeFlowsWidget from './TradeFlowsWidget';
import { useTradeResourceChoices } from './useTradeFlows';

export default { render: TradeFlowsWidget, useOptionChoices: useTradeResourceChoices } satisfies WidgetRenderer;
