import type { OptionChoices, WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, setToken } from '../../dashboard/options';
import { tableSortChoices } from '../../charts/table/options';
import { OPTIONS } from './definition';
import { selectedColumns } from './scoreboardData';
import { useScoreboardColumns } from './scoreboardColumns';
import { scoreboardDeclaration } from './scoreboardProjection';
import './scoreboard.css';

function useColumns(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const data = useScoreboardColumns(runtime, track, true).data;
  const registry = data?.columns ?? [];
  const options = resolveOptions(OPTIONS, widget.options);
  const shown = selectedColumns(registry, options.columns, data?.defaultColumns ?? []);
  const unknown = contentSet(options.columns).filter(token => !registry.some(column => column.key === contentKey(token)));
  return {
    columns: [
      ...registry.map(column => ({ value: setToken(column.key), label: column.label })),
      ...unknown.map(token => ({ value: token, label: `${contentKey(token)} · not in this recording` })),
    ],
    tableSort: tableSortChoices([{ key: 'country', label: 'Country' }, ...shown]),
    topMeasure: registry.filter(column => column.available).map(column => ({ value: setToken(column.key), label: column.label })),
  };
}

export default { declaration: scoreboardDeclaration, useOptionChoices: useColumns } satisfies WidgetRenderer;
