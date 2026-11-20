import { useMemo } from 'react';
import { chipName, useKeyLabelsAt } from './useKeyTagsAt.ts';
import { CountryFlagUrl } from '../components/CountryFlag';
import { Icon } from '../components/Icons';
import { useCampaignSettings } from './campaignSettings';
import { resolveWindow } from './model';
import { useAtHead } from './atHead.ts';
import { groupLabel } from './groups';
import { useCountryGroups } from './useCountryGroups';
import { resolveWidgetCampaign } from './campaigns';
import { selectedCountryKeys } from './countrySelection';
import type { WidgetCountryChipProps } from './definitions';

const NO_KEYS: readonly string[] = [];

/** A parts race's header: the country its pinned breakdown shows, named by the body at its displayed hour; otherwise
 * the widget's countries. */
export function PartsCountryChip(props: WidgetCountryChipProps) {
  return props.widget.options?.show === 'breakdown' ? <span className="dash-widget-country-slot" ref={props.onSlot} /> : <CountryChip {...props} />;
}

/** The widget header's country: flag and name, with "+N" when it shows several.
 * Names are the recording's own; the tooltip previews three countries. It follows
 * playback only while its widget is in view. */
export default function CountryChip({ widget, runtime, active }: Omit<WidgetCountryChipProps, 'onSlot'>) {
  const settings = useCampaignSettings();
  const effective = resolveWidgetCampaign(widget, { tracks: runtime.campaignTracks }, runtime.defaultRoleId);
  const track = runtime.tracks.find(item => item.id === effective.trackIds[0]);
  const groups = useCountryGroups(runtime, track, widget.countries.kind === 'group');
  const overrides = settings.campaigns[track?.source.metadata.campaignKey ?? '']?.factions;
  const tags = selectedCountryKeys(widget.countries, track);
  const shownHour = (clockHour: number) => track ? resolveWindow(effective, track, runtime.tracks, clockHour)?.lastHour ?? Number.NaN : Number.NaN;
  // Named and flagged as every label names a country at the hour shown, never by its catalogue entry's hour.
  const war = widget.countries.kind === 'group' && widget.countries.group.type === 'war' ? widget.countries.group.tag : null;
  const shownKeys = useMemo(() => war !== null ? [war] : tags.slice(0, 3), [war, JSON.stringify(tags)]); // eslint-disable-line react-hooks/exhaustive-deps
  const labels = useKeyLabelsAt(runtime, track, war !== null || widget.countries.kind === 'tags' || widget.countries.kind === 'selected' ? shownKeys : NO_KEYS,
    shownHour, widget.time.mode === 'follow', active);
  const name = () => chipName(labels[0]);
  // A faction's name at the head is read each frame and renders only when it changes.
  const faction = widget.countries.kind === 'group' && widget.countries.group.type === 'faction';
  const label = useAtHead(runtime.clock, active && widget.time.mode === 'follow' && faction, clockHour => widget.countries.kind === 'group'
    ? groupLabel(widget.countries.group, groups.data, name, track ? shownHour(clockHour) : undefined, overrides) : '');
  if (track && widget.countries.kind === 'all') return <span className="dash-widget-country"><Icon name="globe" /><span>All countries</span></span>;
  if (track && widget.countries.kind === 'factions') return <span className="dash-widget-country"><Icon name="layers" /><span>{widget.countries.count === Number.MAX_SAFE_INTEGER ? 'All factions' : `Top ${widget.countries.count} factions`}</span></span>;
  if (track && widget.countries.kind === 'group') return <span className="dash-widget-country">
    <Icon name="layers" /><span>{label}</span></span>;
  if (!track || !tags.length) return null;
  const names = shownKeys.map((_, i) => chipName(labels[i]));
  const first = labels[0];
  return <span className="dash-widget-country" data-tooltip={tags.length > 1 ? `${names.join(', ')}${tags.length > 3 ? ` and ${tags.length - 3} more` : ''}` : undefined}>
    <CountryFlagUrl className="country-flag" url={first?.flag} detail={first?.flagDetail} reason={first?.flagReason ?? undefined} pending={first?.flagPending} />
    <span>{names[0]}</span>{tags.length > 1 && <small>+{tags.length - 1}</small>}
  </span>;
}
