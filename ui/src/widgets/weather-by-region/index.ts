import type { OptionChoices, WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useRegionChoices } from '../../families/useRegionChoices.ts';
import { OPTIONS } from './definition';
import { useWeatherModifiers } from './useWeatherModifiers.ts';
import { weatherDeclaration } from './weatherProjection.ts';

function useChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const options = resolveOptions(OPTIONS, widget.options);
  const regions = useRegionChoices(widget, runtime, String(options.regions ?? ''));
  const modifier = useWeatherModifiers(widget, runtime);
  return { regions, modifier };
}

export default { declaration: weatherDeclaration, useOptionChoices: useChoices } satisfies WidgetRenderer;
