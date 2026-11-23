import { countryFromKey } from '../../dashboard/countryKey.ts';
import { nameReadMetadata } from '../../dashboard/nameReads.ts';
import { useMemo } from 'react';
import type { OptionChoices, WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import { useCountriesSelectionAt } from '../../dashboard/useCountriesSelectionAt';
import { useProduct } from '../../dashboard/products';
import { divisionChoices, validateDivisionCareers } from './careersData';
import ServiceRecordWidget from './ServiceRecordWidget';
import { productRequest } from '../../dashboard/request.ts';

function useDivisionChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const { countries, selectionReason, hour } = useCountriesSelectionAt(widget, runtime, true);
  const tags = countries === undefined ? undefined : countries === null ? null : [...countries].sort();
  const request = track && hour !== null && tags !== undefined && !selectionReason ? productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'division-careers', params: { branchId: track.branch.id, hour, countries: tags?.map(countryFromKey) ?? null } }) : null;
  const { data } = useProduct(runtime.client, request, true, validateDivisionCareers, track?.source);
  const wanted = useMemo(() => data && hour !== null ? [...new Set(data.divisions.flatMap(row => row.tag ? [row.tag] : []))].map(tag => ({ hour, tag })) : [], [data, hour]);
  const names = useCountryNamesAt(runtime.client, track, wanted, true);
  return { division: data && hour !== null ? divisionChoices(data, track!.source.metadata.campaignKey, tag => nameReadMetadata(names.get(nameKey(hour, tag)))) : [] };
}
export default { render: ServiceRecordWidget, useOptionChoices: useDivisionChoices } satisfies WidgetRenderer;
