import type { OptionChoices, WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import CountryChip from '../../dashboard/CountryChip';
import type { WidgetSpec } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useRegionChoices } from '../../families/host/useRegionChoices.ts';
import { OPTIONS } from './definition';
import NavalSupremacyWidget from './NavalSupremacyWidget';

function useChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const options = resolveOptions(OPTIONS, widget.options);
  return { regions: useRegionChoices(widget, runtime, String(options.regions ?? ''), { sea: true }) };
}

export default { render: NavalSupremacyWidget, countryChip: CountryChip, useOptionChoices: useChoices } satisfies WidgetRenderer;
