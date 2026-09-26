import { SourceFlag } from '../components/CountryFlag';
import { Icon } from '../components/Icons';
import { groupLabel } from './groups';
import { useCountryGroups } from './useCountryGroups';
import { resolveWidgetCampaign } from './campaigns';
import { selectedTags } from './countrySelection';
import type { WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';

/** The widget header's country: flag and name, with "+N" when it shows several.
 * Names are the recording's own; the tooltip lists every country. */
export default function CountryChip({ widget, runtime }: { widget: WidgetSpec; runtime: WidgetRuntime }) {
  const effective = resolveWidgetCampaign(widget, { tracks: runtime.campaignTracks }, runtime.defaultRoleId);
  const track = runtime.tracks.find(item => item.id === effective.trackIds[0]);
  const groups = useCountryGroups(runtime, track, widget.countries.kind === 'group');
  const tags = selectedTags(widget.countries, track);
  const name = (tag: string) => track?.source.metadata.countries.find(country => country.tag === tag)?.name || tag;
  if (track && widget.countries.kind === 'all') return <span className="dash-widget-country"><Icon name="globe" /><span>All countries</span></span>;
  if (track && widget.countries.kind === 'group') return <span className="dash-widget-country">
    <Icon name="layers" /><span>{groupLabel(widget.countries.group, groups.data, name)}</span></span>;
  if (!track || !tags.length) return null;
  const names = tags.map(name);
  return <span className="dash-widget-country" data-tooltip={names.length > 1 ? names.join(', ') : undefined}>
    <SourceFlag client={runtime.client} source={track.source} tag={tags[0]} />
    <span>{names[0]}</span>{tags.length > 1 && <small>+{tags.length - 1}</small>}
  </span>;
}
