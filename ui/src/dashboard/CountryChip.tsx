import { SourceFlag } from '../components/CountryFlag';
import { Icon } from '../components/Icons';
import { useCampaignSettings } from './campaignSettings';
import { resolveWindow } from './model';
import { usePlaybackHour } from './playback';
import { groupLabel } from './groups';
import { useCountryGroups } from './useCountryGroups';
import { resolveWidgetCampaign } from './campaigns';
import { selectedTags } from './countrySelection';
import type { WidgetCountryChipProps } from './definitions';
import { ChooseCountryChip } from './DatedCountryChip';

/** Parts breakdowns publish the holder and date shown by their body here. */
export function PartsCountryChip(props: WidgetCountryChipProps) {
  return props.widget.options?.show === 'breakdown' ? <span className="dash-widget-country-slot" ref={props.onSlot} /> : <CountryChip {...props} />;
}

/** A one-country breakdown's header. Pin as widget saves the country it pinned in the
 * option `pin`, and the body names that holder here at its displayed date. An unpinned
 * breakdown, such as a catalogue card's, whose body offers no country to choose, asks
 * for one here. */
export function breakdownCountryChip({ widget, onSlot }: WidgetCountryChipProps, pin: string) {
  return Object.hasOwn(widget.options ?? {}, pin) ? <span className="dash-widget-country-slot" ref={onSlot} /> : <ChooseCountryChip />;
}

/** The widget header's country: flag and name, with "+N" when it shows several.
 * Names are the recording's own; the tooltip previews three countries. It follows
 * playback only while its widget is in view. */
export default function CountryChip({ widget, runtime, active }: Omit<WidgetCountryChipProps, 'onSlot'>) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow' && widget.countries.kind === 'group' && widget.countries.group.type === 'faction');
  const settings = useCampaignSettings();
  const effective = resolveWidgetCampaign(widget, { tracks: runtime.campaignTracks }, runtime.defaultRoleId);
  const track = runtime.tracks.find(item => item.id === effective.trackIds[0]);
  const groups = useCountryGroups(runtime, track, widget.countries.kind === 'group');
  const hour = track ? resolveWindow(effective, track, runtime.tracks, runtime.clock.getHour())?.lastHour ?? Number.NaN : undefined;
  const overrides = settings.campaigns[track?.source.metadata.campaignKey ?? '']?.factions;
  const tags = selectedTags(widget.countries, track);
  const name = (tag: string) => track?.source.metadata.countries.find(country => country.tag === tag)?.name ?? 'Country name unavailable';
  if (track && widget.countries.kind === 'all') return <span className="dash-widget-country"><Icon name="globe" /><span>All countries</span></span>;
  if (track && widget.countries.kind === 'factions') return <span className="dash-widget-country"><Icon name="layers" /><span>{widget.countries.count === Number.MAX_SAFE_INTEGER ? 'All factions' : `Top ${widget.countries.count} factions`}</span></span>;
  if (track && widget.countries.kind === 'group') return <span className="dash-widget-country">
    <Icon name="layers" /><span>{groupLabel(widget.countries.group, groups.data, name, hour, overrides)}</span></span>;
  if (!track || !tags.length) return null;
  const names = tags.slice(0, 3).map(name);
  return <span className="dash-widget-country" data-tooltip={tags.length > 1 ? `${names.join(', ')}${tags.length > 3 ? ` and ${tags.length - 3} more` : ''}` : undefined}>
    <SourceFlag client={runtime.client} source={track.source} tag={tags[0]} />
    <span>{names[0]}</span>{tags.length > 1 && <small>+{tags.length - 1}</small>}
  </span>;
}
