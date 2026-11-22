import { countryFromKey } from '../../dashboard/countryKey.ts';
import { nameReadMetadata } from '../../dashboard/nameReads.ts';
import { useMemo } from 'react';
import type { OptionChoices, WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { resolveWindow } from '../../dashboard/model';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import { useCountriesSelectionAt } from '../../dashboard/useCountriesSelectionAt';
import { useProduct } from '../../dashboard/products';
import { divisionChoices, validateDivisionCareers } from './careersData';
import ServiceRecordWidget from './ServiceRecordWidget';
import { usePlaybackHour } from '../../dashboard/follow/useFollow';

function useDivisionChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const playbackHour = usePlaybackHour(runtime.clock, widget.time.mode === 'follow');
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const { countries, selectionReason } = useCountriesSelectionAt(widget, runtime, hour, true);
  const tags = countries === undefined ? undefined : countries === null ? null : [...countries].sort();
  const request = track && tags !== undefined && !selectionReason ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'division-careers', params: { branchId: track.branch.id, hour, countries: tags?.map(countryFromKey) ?? null } } : null;
  const { data } = useProduct(runtime.client, request, true, validateDivisionCareers, track?.source);
  const wanted = useMemo(() => data ? [...new Set(data.divisions.flatMap(row => row.tag ? [row.tag] : []))].map(tag => ({ hour, tag })) : [], [data, hour]);
  const names = useCountryNamesAt(runtime.client, track, wanted, true);
  return { division: data ? divisionChoices(data, track!.source.metadata.campaignKey, tag => nameReadMetadata(names.get(nameKey(hour, tag)))) : [] };
}
export default { render: ServiceRecordWidget, useOptionChoices: useDivisionChoices } satisfies WidgetRenderer;
