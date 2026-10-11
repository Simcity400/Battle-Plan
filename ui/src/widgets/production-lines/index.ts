import type { WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import ProductionLinesWidget from './ProductionLinesWidget';
import ProductionCountryChip from './ProductionCountryChip';
import { readDatedCountry } from '../../dashboard/datedCountry';
import { useProductionLines, useLineBreakdown } from './useProductionLines';
import { productionLineKey, productionLineToken, productionLineLabel } from './productionData';

function useLineChoices(widget: WidgetSpec, runtime: WidgetRuntime) {
  const { data, track, tag } = useProductionLines(widget, runtime, true);
  const key = track && tag ? productionLineKey(track.source.metadata.campaignKey, track.branch.id, tag, widget.options?.line) : null;
  const group = widget.options?.show === 'output' ? 'output' : 'efficiency';
  const country = track ? readDatedCountry(widget.options?.productionCountry, track.source.metadata.campaignKey, track.branch.id, widget.countries) : null;
  const breakdown = useLineBreakdown(runtime, track, tag, key, group, true, country && track ? track.branch.firstHour + country.originOffset : undefined);
  return { line: data && track ? data.lines.map((line, index) => ({ value: productionLineToken(track.source.metadata.campaignKey, data.branchId, data.tag, line.key),
      label: productionLineLabel(data, line, index, country && track ? track.branch.firstHour + country.originOffset : track.branch.lastHour) })) : [],
    factor: (breakdown.data?.groups ?? []).map(group => ({ value: group.id, label: group.label })) };
}
export default { render: ProductionLinesWidget, countryChip: ProductionCountryChip, useOptionChoices: useLineChoices } satisfies WidgetRenderer;
