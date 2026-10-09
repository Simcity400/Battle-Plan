import type { WidgetRenderer } from '../../dashboard/definitions';
import territory from '../territory';
import TerritoryBreakdownWidget from './TerritoryBreakdownWidget';
export default { render: TerritoryBreakdownWidget, useOptionChoices: territory.useOptionChoices } satisfies WidgetRenderer;
