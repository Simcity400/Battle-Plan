import { useMemo } from 'react';
import type { OptionChoices, WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import { useCountriesSelectionAt } from '../../dashboard/useCountriesSelectionAt';
import { useReachedProduct } from '../../dashboard/reachedProduct';
import { divisionChoices, validateDivisionCareers } from './careersData';
import ServiceRecordWidget from './ServiceRecordWidget';

/** Divisions of the chosen countries in service at the campaign's last hour. */
function useDivisionChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const hour = track?.branch.lastHour ?? 0;
  const { countries, selectionReason } = useCountriesSelectionAt(widget, runtime, hour, true);
  const tags = countries === undefined ? undefined : countries === null ? null : [...countries].sort();
  const request = track && tags !== undefined && !selectionReason ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'division-careers', params: { branchId: track.branch.id, hour, tags } } : null;
  const { data } = useReachedProduct(runtime.client, track?.source, request, JSON.stringify([track?.branch.id, tags]), true, validateDivisionCareers);
  const wanted = useMemo(() => data ? [...new Set(data.divisions.flatMap(row => row.tag ? [row.tag] : []))].map(tag => ({ hour: data.hour, tag })) : [], [data]);
  const names = useCountryNamesAt(runtime.client, track, wanted, true);
  return { division: data ? divisionChoices(data, tag => names.get(nameKey(data.hour, tag)) ?? 'Country name unavailable') : [] };
}
export default { render: ServiceRecordWidget, useOptionChoices: useDivisionChoices } satisfies WidgetRenderer;
