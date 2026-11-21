import { defaultCountryKey } from '../../dashboard/countrySelection.ts';
import { worldFilter, worldRequestTags } from '../../dashboard/groups.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import type { CountryGroupReply } from '../../generated/dashboard';

export function countriesAt(widget: Pick<WidgetSpec, 'countries'>, track: Pick<BoundTrack, 'branch' | 'source'> | undefined, groups: CountryGroupReply | undefined,
  hour: number) {
  const tag = defaultCountryKey(track as BoundTrack | undefined);
  const fallback = tag ? [tag] : [];
  const lastHour = track?.branch.lastHour ?? hour;
  const filter = worldFilter(widget.countries, fallback, groups, hour, lastHour);
  const request = worldRequestTags(widget.countries, fallback, groups, lastHour);
  const ready = widget.countries.kind === 'all' || !!groups;
  return { countries: ready && !filter.reason ? filter.tags : undefined, selectionReason: filter.reason ?? request.reason, known: filter.known };
}
