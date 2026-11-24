import { useMemo } from 'react';
import { chipName, useKeyLabelsAt } from './useKeyTagsAt.ts';
import { CountryFlagUrl } from '../components/CountryFlag';
import { Icon } from '../components/Icons';
import { useCampaignSettings } from './campaignSettings';
import { resolveWindow } from './model';
import { resolveWidgetCampaign } from './campaigns';
import { selectedCountryKeys } from './countrySelection';
import type { ResolvedViewOptions, WidgetCountryChipProps } from './definitions';
import type { WidgetBase } from './model';
import {
  factionKey,
  useIdentityAtHead,
  useIdentityScope,
  type FactionIdentityResult,
  type IdentityKey,
} from '../families/identity/react.ts';

const NO_KEYS: readonly string[] = [];
const NO_IDENTITY_KEYS: readonly IdentityKey[] = [];

type AnyChipProps = Omit<WidgetCountryChipProps, 'widget'> & { readonly widget: WidgetBase; readonly options?: ResolvedViewOptions };

export function PartsCountryChip(props: AnyChipProps) {
  return props.options?.show === 'breakdown' ? <span className="dash-widget-country-slot" ref={props.onSlot} /> : <CountryChip {...props} />;
}

export default function CountryChip({ widget, runtime, active, compact = false }: Omit<AnyChipProps, 'onSlot'>) {
  const settings = useCampaignSettings();
  const effective = resolveWidgetCampaign(widget, { tracks: runtime.campaignTracks }, runtime.defaultRoleId);
  const track = runtime.tracks.find(item => item.id === effective.trackIds[0]);
  const overrides = settings.campaigns[track?.source.metadata.campaignKey ?? '']?.factions;
  const tags = selectedCountryKeys(widget.countries, track);
  const shownHour = (clockHour: number) => track ? resolveWindow(effective, track, runtime.tracks, clockHour)?.lastHour ?? Number.NaN : Number.NaN;
  const war = widget.countries.kind === 'group' && widget.countries.group.type === 'war' ? widget.countries.group.tag : null;
  const shownKeys = useMemo(() => war !== null ? [war] : tags.slice(0, 3), [war, JSON.stringify(tags)]);
  const labels = useKeyLabelsAt(runtime, track, war !== null || widget.countries.kind === 'tags' || widget.countries.kind === 'selected' ? shownKeys : NO_KEYS,
    shownHour, widget.time.mode === 'follow', active);
  const factionGroupId = widget.countries.kind === 'group' && widget.countries.group.type === 'faction' ? widget.countries.group.id : null;
  const factionScope = useIdentityScope(runtime.client, track?.source, track?.branch.id, active && factionGroupId !== null);
  const factionKeys = useMemo<readonly IdentityKey[]>(
    () => (factionGroupId !== null ? [factionKey(factionGroupId)] : NO_IDENTITY_KEYS),
    [factionGroupId],
  );
  const branchBounds = useMemo(
    () => (track ? [track.branch.firstHour, track.branch.lastHour] as const : undefined),
    [track?.branch.firstHour, track?.branch.lastHour],
  );
  const factionAtHead = useIdentityAtHead(
    runtime.client,
    track?.source,
    factionGroupId !== null ? factionScope : null,
    runtime.clock,
    factionKeys,
    clockHour => {
      const h = shownHour(clockHour);
      return Number.isFinite(h) ? h : null;
    },
    { active: active && factionGroupId !== null, follow: widget.time.mode === 'follow', branchBounds },
  );
  const label = (() => {
    if (widget.countries.kind !== 'group') return '';
    switch (widget.countries.group.type) {
      case 'humans': return 'Human players';
      case 'war': return `At war with ${chipName(labels[0])}`;
      case 'top': return `Top ${widget.countries.group.count}`;
      case 'faction': {
        const override = overrides?.[widget.countries.group.id]?.name;
        if (override) return override;
        const resolved = factionAtHead.results[0] as FactionIdentityResult | undefined;
        if (!resolved || resolved.kind === 'pending') return 'Loading faction…';
        if (resolved.kind === 'known' && resolved.name.kind === 'known') return resolved.name.value;
        return 'Unnamed faction';
      }
    }
  })();
  if (track && widget.countries.kind === 'all') return <span className="dash-widget-country" data-tooltip={compact ? 'All countries' : undefined}><Icon name="globe" /><span>All countries</span></span>;
  if (track && widget.countries.kind === 'factions') {
    const text = widget.countries.count === Number.MAX_SAFE_INTEGER ? 'All factions' : `Top ${widget.countries.count} factions`;
    return <span className="dash-widget-country" data-tooltip={compact ? text : undefined}><Icon name="layers" /><span>{text}</span></span>;
  }
  if (track && widget.countries.kind === 'group') return <span className="dash-widget-country" data-tooltip={compact ? label : undefined}>
    <Icon name="globe" /><span>{label}</span></span>;
  if (!track || !tags.length) return null;
  const names = shownKeys.map((_, i) => chipName(labels[i]));
  const first = labels[0];
  const tooltip = tags.length > 1 ? `${names.join(', ')}${tags.length > 3 ? ` and ${tags.length - 3} more` : ''}` : compact ? names[0] : undefined;
  return <span className="dash-widget-country" data-tooltip={tooltip}>
    <CountryFlagUrl className="country-flag" url={first?.flag} detail={first?.flagDetail} reason={first?.flagReason ?? undefined} pending={first?.flagPending} />
    <span>{names[0]}</span>{tags.length > 1 && <small>+{tags.length - 1}</small>}
  </span>;
}
