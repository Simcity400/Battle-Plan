import { SourceFlag } from '../components/CountryFlag';
import { Icon } from '../components/Icons';
import { useCampaignSettings } from './campaignSettings';
import { factionNameAt } from './factionNames';
import { resolveWindow } from './model';
import { usePlaybackHour } from './playback';
import { groupLabel } from './groups';
import { useCountryGroups } from './useCountryGroups';
import { resolveWidgetCampaign } from './campaigns';
import { selectedTags } from './countrySelection';
import type { WidgetCountryChipProps, WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';

/** Parts breakdowns publish the holder and date shown by their body here. */
export function PartsCountryChip(props: WidgetCountryChipProps) {
  return props.widget.options?.show === 'breakdown' ? <span className="dash-widget-country-slot" ref={props.onSlot} /> : <CountryChip {...props} />;
}

/** The widget header's country: flag and name, with "+N" when it shows several.
 * Names are the recording's own; the tooltip lists every country. */
export default function CountryChip({ widget, runtime }: { widget: WidgetSpec; runtime: WidgetRuntime }) {
  usePlaybackHour(runtime.clock, widget.time.mode === 'follow' && widget.countries.kind === 'group' && widget.countries.group.type === 'faction');
  const settings = useCampaignSettings();
  const effective = resolveWidgetCampaign(widget, { tracks: runtime.campaignTracks }, runtime.defaultRoleId);
  const track = runtime.tracks.find(item => item.id === effective.trackIds[0]);
  const groups = useCountryGroups(runtime, track, widget.countries.kind === 'group');
  const hour = track ? resolveWindow(effective, track, runtime.tracks, runtime.clock.getHour())?.lastHour ?? Number.NaN : undefined;
  const overrides = settings.campaigns[track?.source.metadata.campaignKey ?? '']?.factions;
  const group = widget.countries.kind === 'group' ? widget.countries.group : null;
  const faction = group?.type === 'faction' ? groups.data?.factions.find(faction => faction.id === group.id) : undefined;
  const nameReason = faction ? factionNameAt(faction, hour, overrides?.[faction.id]?.name).reason : null;
  const tags = selectedTags(widget.countries, track);
  const name = (tag: string) => track?.source.metadata.countries.find(country => country.tag === tag)?.name || tag;
  if (track && widget.countries.kind === 'all') return <span className="dash-widget-country"><Icon name="globe" /><span>All countries</span></span>;
  if (track && widget.countries.kind === 'factions') return <span className="dash-widget-country"><Icon name="layers" /><span>Top {widget.countries.count} factions</span></span>;
  if (track && widget.countries.kind === 'group') return <span className="dash-widget-country" data-tooltip={nameReason ?? undefined}>
    <Icon name="layers" /><span>{groupLabel(widget.countries.group, groups.data, name, hour, overrides)}</span></span>;
  if (!track || !tags.length) return null;
  const names = tags.map(name);
  return <span className="dash-widget-country" data-tooltip={names.length > 1 ? names.join(', ') : undefined}>
    <SourceFlag client={runtime.client} source={track.source} tag={tags[0]} />
    <span>{names[0]}</span>{tags.length > 1 && <small>+{tags.length - 1}</small>}
  </span>;
}
