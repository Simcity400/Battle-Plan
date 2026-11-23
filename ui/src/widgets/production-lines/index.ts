import type { WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import ProductionLinesWidget, { useProductionLines } from './ProductionLinesWidget';
import { productionLineLabel, productionLineToken } from './productionData';

function useLineChoices(widget: WidgetSpec, runtime: WidgetRuntime) {
  const { data, track } = useProductionLines(widget, runtime, true);
  return {
    line: data && track ? data.lines.map((line, index) => ({ value: productionLineToken(track.source.metadata.campaignKey, data.branchId, line.key),
      label: productionLineLabel(data, line, index, track.branch.lastHour) })) : [],
    factor: FACTORS,
  };
}

const FACTORS = [
  ['efficiency', 'Stored factory efficiency'], ['start', 'Starting efficiency'], ['cap', 'Efficiency cap'], ['gain', 'Efficiency growth coefficient'],
  ['daily', 'Native daily efficiency increase'], ['retention-0', 'Variant switch retention'], ['retention-1', 'Parent switch retention'],
  ['retention-2', 'Family switch retention'], ['retention-3', 'Archetype switch retention'], ['output', 'Recorded daily production IC'],
  ['power', 'Effective powered-factory fraction'], ['resources', 'Resource output factor'], ['conversion', 'Conversion speed parameter'], ['refit', 'Refit speed parameter'],
].map(([value, label]) => ({ value, label }));

export default { render: ProductionLinesWidget, useOptionChoices: useLineChoices } satisfies WidgetRenderer;
