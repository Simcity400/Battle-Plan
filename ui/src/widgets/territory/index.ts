import type { OptionChoices, WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { useTerritoryMeasures } from '../../dashboard/useTerritoryMeasures';
import TerritoryWidget from './TerritoryWidget';

function useMeasures(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  return { measure: useTerritoryMeasures(widget, runtime) };
}

export default { render: TerritoryWidget, useOptionChoices: useMeasures } satisfies WidgetRenderer;
