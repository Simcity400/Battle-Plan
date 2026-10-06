import type { OptionChoices, WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, setToken } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { tableSortChoices } from '../../charts/table/options';
import type { ScoreboardReply } from '../../generated/dashboard';
import ScoreboardWidget from './ScoreboardWidget';
import { OPTIONS } from './definition';
import { selectedColumns, validateScoreboardReply } from './scoreboardData';

function useColumns(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'scoreboard',
    params: { branchId: track.branch.id, columns: [], tags: [], top: null } } : null;
  const { data } = useProduct<ScoreboardReply>(runtime.client, request, true, validateScoreboardReply);
  const registry = data?.columns ?? [];
  const options = resolveOptions(OPTIONS, widget.options);
  const shown = selectedColumns(registry, options.columns, data?.defaultColumns ?? []);
  // An old campaign's unrecognised picks stay stored; it is clear why they cannot act.
  const unknown = contentSet(options.columns).filter(token => !registry.some(column => column.key === contentKey(token)));
  return {
    columns: [...registry.map(column => ({ value: setToken(column.key), label: column.label })),
      ...unknown.map(token => ({ value: token, label: `${contentKey(token)} · not in this recording` }))],
    tableSort: tableSortChoices([{ key: 'country', label: 'Country' }, ...shown]),
    topMeasure: registry.filter(column => column.available).map(column => ({ value: setToken(column.key), label: column.label })),
  };
}

export default { render: ScoreboardWidget, useOptionChoices: useColumns } satisfies WidgetRenderer;
