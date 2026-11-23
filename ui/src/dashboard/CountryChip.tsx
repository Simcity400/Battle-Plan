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

export function PartsCountryChip(props: WidgetCountryChipProps) {
  return props.widget.options?.show === 'breakdown' ? <span className="dash-widget-country-slot" ref={props.onSlot} /> : <CountryChip {...props} />;
}

export default function CountryChip({ widget, runtime, active, compact = false }: Omit<WidgetCountryChipProps, 'onSlot'>) {
  const settings = useCampaignSettings();
  const effective = resolveWidgetCampaign(widget, { tracks: runtime.campaignTracks }, runtime.defaultRoleId);
  const track = runtime.tracks.find(item => item.id === effective.trackIds[0]);
  const groups = useCountryGroups(runtime, track, widget.countries.kind === 'group');
  const overrides = settings.campaigns[track?.source.metadata.campaignKey ?? '']?.factions;
  const tags = selectedCountryKeys(widget.countries, track);
  const shownHour = (clockHour: number) => track ? resolveWindow(effective, track, runtime.tracks, clockHour)?.lastHour ?? Number.NaN : Number.NaN;
  const war = widget.countries.kind === 'group' && widget.countries.group.type === 'war' ? widget.countries.group.tag : null;
  const shownKeys = useMemo(() => war !== null ? [war] : tags.slice(0, 3), [war, JSON.stringify(tags)]);
  const labels = useKeyLabelsAt(runtime, track, war !== null || widget.countries.kind === 'tags' || widget.countries.kind === 'selected' ? shownKeys : NO_KEYS,
    shownHour, widget.time.mode === 'follow', active);
  const name = () => chipName(labels[0]);
  const faction = widget.countries.kind === 'group' && widget.countries.group.type === 'faction';
  const label = useAtHead(runtime.clock, active && widget.time.mode === 'follow' && faction, clockHour => widget.countries.kind === 'group'
    ? groupLabel(widget.countries.group, groups.data, name, track ? shownHour(clockHour) : undefined, overrides) : '');
  if (track && widget.countries.kind === 'all') return <span className="dash-widget-country" data-tooltip={compact ? 'All countries' : undefined}><Icon name="globe" /><span>All countries</span></span>;
  if (track && widget.countries.kind === 'factions') {
    const text = widget.countries.count === Number.MAX_SAFE_INTEGER ? 'All factions' : `Top ${widget.countries.count} factions`;
    return <span className="dash-widget-country" data-tooltip={compact ? text : undefined}><Icon name="layers" /><span>{text}</span></span>;
  }
  if (track && widget.countries.kind === 'group') return <span className="dash-widget-country" data-tooltip={compact ? label : undefined}>
    <Icon name="layers" /><span>{label}</span></span>;
  if (!track || !tags.length) return null;
  const names = shownKeys.map((_, i) => chipName(labels[i]));
  const first = labels[0];
  const tooltip = tags.length > 1 ? `${names.join(', ')}${tags.length > 3 ? ` and ${tags.length - 3} more` : ''}` : compact ? names[0] : undefined;
  return <span className="dash-widget-country" data-tooltip={tooltip}>
    <CountryFlagUrl className="country-flag" url={first?.flag} detail={first?.flagDetail} reason={first?.flagReason ?? undefined} pending={first?.flagPending} />
    <span>{names[0]}</span>{tags.length > 1 && <small>+{tags.length - 1}</small>}
  </span>;
}
