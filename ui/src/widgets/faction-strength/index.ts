import type { OptionChoices, WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useTerritoryMeasures } from '../../dashboard/useTerritoryMeasures';
import { OPTIONS } from './definition';
import FactionStrengthWidget from './FactionStrengthWidget';

function useMeasures(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const options = resolveOptions(OPTIONS, widget.options);
  return { territoryMeasure: useTerritoryMeasures(widget, runtime, options.measure === 'territory') };
}

export default { render: FactionStrengthWidget, useOptionChoices: useMeasures } satisfies WidgetRenderer;
