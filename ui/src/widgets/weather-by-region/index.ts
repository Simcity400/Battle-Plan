import type { OptionChoices, WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { useWeatherModifiers } from './useWeatherModifiers.ts';
import { weatherDeclaration } from './weatherProjection.ts';

function useChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  return { modifier: useWeatherModifiers(widget, runtime) };
}

export default { declaration: weatherDeclaration, useOptionChoices: useChoices } satisfies WidgetRenderer;
