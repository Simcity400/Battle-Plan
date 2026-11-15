import { defaultCountryTag } from '../../dashboard/countrySelection.ts';
import { worldFilter, worldRequestTags } from '../../dashboard/groups.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import type { CountryGroupReply } from '../../generated/dashboard';

/** The countries a widget's selection holds at `hour`, as `useCountriesSelectionAt` gives them, from the groups already
 * read: `countries` undefined while they are not known, `selectionReason` where the selection cannot be shown. */
export function countriesAt(widget: Pick<WidgetSpec, 'countries'>, track: Pick<BoundTrack, 'branch' | 'source'> | undefined, groups: CountryGroupReply | undefined,
  hour: number) {
  const tag = defaultCountryTag(track as BoundTrack | undefined);
  const fallback = tag ? [tag] : [];
  const lastHour = track?.branch.lastHour ?? hour;
  const filter = worldFilter(widget.countries, fallback, groups, hour, lastHour);
  const request = worldRequestTags(widget.countries, fallback, groups, lastHour);
  const ready = widget.countries.kind === 'all' || !!groups;
  return { countries: ready && !filter.reason ? filter.tags : undefined, selectionReason: filter.reason ?? request.reason, known: filter.known };
}
