import type { WidgetRenderer } from '../../dashboard/definitions';
import TradeFlowsWidget from './TradeFlowsWidget';
import TradeCountryChip from './TradeCountryChip';
import { useTradeResourceChoices } from './useTradeFlows';

export default { render: TradeFlowsWidget, countryChip: TradeCountryChip, useOptionChoices: useTradeResourceChoices } satisfies WidgetRenderer;
