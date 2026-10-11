import CountryPicker from '../../components/CountryPicker';
import { Select } from '../../components/Select';
import { SettingLabel } from './SettingLabel';
import { SourceFlag } from '../../components/CountryFlag';
import { Icon } from '../../components/Icons';
import { Segmented } from '../../components/Segmented';
import type { CountryGroupReply } from '../../generated/dashboard';
import { defaultCountryTag, selectedTags, singleCountryChoice } from '../countrySelection';
import { countrySelectionFor, countrySelectionMode, countrySelectionModes, DEFAULT_TOP } from '../groups';
import { useCampaignSettings } from '../campaignSettings';
import { factionNameAt, UNNAMED_FACTION } from '../factionNames';
import { resolveWindow } from '../model';
import { usePlaybackHour } from '../playback';
import { useCountryGroups } from '../useCountryGroups';
import type { BoundTrack, CountryGroup, CountrySelection, GroupMembership, WidgetSpec } from '../model';
import type { WidgetMeta, WidgetRuntime } from '../definitions';

type Limits = NonNullable<WidgetMeta['countries']>;

/** One country from a picker; or, for widgets that show several, chosen countries
 * as removable chips or a smart group (human players, a faction, everyone at war
 * with a country, or the top N where the widget ranks). */
export function CountriesSection({ widget, reference, runtime, limits, inline = false, onChange }: {
  widget: WidgetSpec; reference: BoundTrack | undefined; runtime: WidgetRuntime; limits: Limits; onChange: (countries: CountrySelection) => void;
  /** Show the country search and list in place (a header chip's chooser) instead of behind a field. */
  inline?: boolean;
}) {
  const source = reference?.source;
  const fallback = defaultCountryTag(reference);
  const common = { field: !inline, inline, source, client: runtime.client, countries: source?.metadata.countries ?? [], playerCountryId: source?.metadata.playerCountryId ?? null };
  if (limits.max <= 1) return <CountryPicker key={source?.id ?? 'unbound'} {...common}
    selectedTag={widget.countries.kind === 'tags' ? widget.countries.tags[0] ?? null : fallback}
    onChange={tag => onChange(singleCountryChoice(tag, fallback, limits.fixed))} />;
  const mode = widget.countries.kind === 'all' && !limits.all ? 'countries' : countrySelectionMode(widget.countries);
  return <>
    <Segmented label="Choose countries by" value={mode} options={countrySelectionModes(limits)}
      onChange={value => { if (value !== mode) onChange(countrySelectionFor(value, limits.max)); }} />
    {mode === 'all' ? <p className="field-hint">Every country the recording names.</p>
      : widget.countries.kind === 'group'
        ? <GroupFields widget={widget} selection={widget.countries} reference={reference} runtime={runtime} limits={limits} onChange={onChange} />
        : widget.countries.kind === 'factions'
          ? <FactionFields selection={widget.countries} max={limits.max} onChange={onChange} />
          : <CountryChips widget={widget} reference={reference} runtime={runtime} max={limits.max} inline={inline} onChange={onChange} />}
  </>;
}

function CountryChips({ widget, reference, runtime, max, inline, onChange }: {
  widget: WidgetSpec; reference: BoundTrack | undefined; runtime: WidgetRuntime; max: number; inline: boolean; onChange: (countries: CountrySelection) => void;
}) {
  const source = reference?.source;
  const tags = selectedTags(widget.countries, reference);
  const follows = widget.countries.kind === 'selected';
  const setTags = (next: string[]) => onChange(next.length ? { kind: 'tags', tags: next } : { kind: 'selected' });
  const name = (tag: string) => source?.metadata.countries.find(country => country.tag === tag)?.name ?? 'Country name unavailable';
  return <>
    <ul className="settings-countries" aria-label="Chosen countries">{tags.map(tag => <li key={tag} className="settings-country">
      {source && <SourceFlag client={runtime.client} source={source} tag={tag} />}
      <span>{name(tag)}{follows && <small>Dashboard country</small>}</span>
      {!follows && <button type="button" className="icon-button small" aria-label={`Remove ${name(tag)}`}
        onClick={() => setTags(tags.filter(item => item !== tag))}><Icon name="close" /></button>}
    </li>)}</ul>
    <CountryPicker key={source?.id ?? 'unbound'} field={!inline} inline={inline} source={source} client={runtime.client} countries={source?.metadata.countries ?? []}
      playerCountryId={source?.metadata.playerCountryId ?? null} selectedTag={null} chosen={tags}
      addLabel={tags.length >= max ? `Up to ${max} countries; pick one to remove it` : 'Add or remove a country'}
      onChange={tag => {
        if (!tag) return;
        if (tags.includes(tag)) setTags(tags.filter(item => item !== tag));
        else if (tags.length < max) setTags([...tags, tag]);
      }} />
  </>;
}

const MEMBERSHIP: readonly { value: GroupMembership; label: string; tooltip: string }[] = [
  { value: 'hour', label: 'At each hour', tooltip: 'Members as they were at each moment; joins and leaves are marked' },
  { value: 'end', label: 'At the end', tooltip: 'Members at the end of the campaign' },
  { value: 'ever', label: 'Anyone ever', tooltip: 'Every country that was ever a member' },
];

function FactionFields({ selection, max, onChange }: {
  selection: Extract<CountrySelection, { kind: 'factions' }>; max: number; onChange: (countries: CountrySelection) => void;
}) {
  return <>
    <label className="setting-row"><SettingLabel label="How many" hint="Factions ranked by their total each hour" />
      <span className="option-number"><input type="number" min={1} max={max} placeholder="All"
        value={selection.count === Number.MAX_SAFE_INTEGER ? '' : selection.count}
        onChange={event => {
          // Cleared means every faction, as a new widget starts.
          const count = event.currentTarget.value === '' ? Number.MAX_SAFE_INTEGER : Math.round(Number(event.currentTarget.value));
          if (count >= 1 && count <= max) onChange({ ...selection, count });
        }} /></span></label>
    <div className="setting-row setting-row-stacked"><span>Members</span>
      <Segmented label="Faction membership" value={selection.membership} options={MEMBERSHIP} onChange={membership => onChange({ ...selection, membership })} /></div>
    <p className="field-hint">Each recorded faction is one total. A missing member value or membership read leaves a gap.</p>
  </>;
}

function GroupFields({ widget, selection, reference, runtime, limits, onChange }: {
  widget: WidgetSpec; selection: Extract<CountrySelection, { kind: 'group' }>; reference: BoundTrack | undefined; runtime: WidgetRuntime; limits: Limits;
  onChange: (countries: CountrySelection) => void;
}) {
  usePlaybackHour(runtime.clock, widget.time.mode === 'follow');
  const settings = useCampaignSettings();
  const hour = reference ? resolveWindow(widget, reference, runtime.tracks, runtime.clock.getHour())?.lastHour ?? Number.NaN : undefined;
  const overrides = settings.campaigns[reference?.source.metadata.campaignKey ?? '']?.factions;
  const groups = useCountryGroups(runtime, reference);
  const source = reference?.source;
  const set = (patch: Partial<Extract<CountrySelection, { kind: 'group' }>>) => onChange({ ...selection, ...patch });
  const group = selection.group;
  const kinds = groupKinds(groups.data, limits, hour, overrides);
  // A saved faction this branch did not record (or that is still loading) stays
  // visible as the selection instead of the list silently showing another group.
  if (group.type === 'faction' && !kinds.some(kind => kind.value === `faction:${group.id}`)) {
    kinds.splice(1, 0, { value: `faction:${group.id}`, label: groups.data ? 'A faction not in this branch' : 'Loading factions…' });
  }
  const value = group.type === 'faction' ? `faction:${group.id}` : group.type;
  return <>
    <div className="setting-row"><span>Group</span>
      <Select label="Group" value={value} options={kinds} onChange={choice => {
        const next: CountryGroup = choice.startsWith('faction:') ? { type: 'faction', id: choice.slice(8) }
          : choice === 'war' ? { type: 'war', tag: defaultCountryTag(reference) ?? '' }
            : choice === 'top' ? { type: 'top', count: Math.min(DEFAULT_TOP, limits.max) } : { type: 'humans' };
        if (next.type !== 'war' || next.tag) set({ group: next });
      }} /></div>
    {groups.error && <p className="field-hint" role="alert">Groups unavailable: {groups.error}</p>}
    {group.type === 'war' && <CountryPicker key={source?.id ?? 'unbound'} field source={source} client={runtime.client}
      countries={source?.metadata.countries ?? []} playerCountryId={source?.metadata.playerCountryId ?? null} selectedTag={group.tag}
      onChange={tag => { if (tag) set({ group: { type: 'war', tag } }); }} />}
    {group.type === 'top' && <label className="setting-row"><SettingLabel label="How many" hint="Ranked by this widget's value each hour" />
      <span className="option-number"><input type="number" min={1} max={limits.max} value={group.count}
        onChange={event => { const count = Math.round(Number(event.currentTarget.value)); if (count >= 1 && count <= limits.max) set({ group: { type: 'top', count } }); }} /></span></label>}
    {group.type !== 'top' && <div className="setting-row setting-row-stacked"><span>Members</span>
      <Segmented label="Group membership" value={selection.membership} options={MEMBERSHIP} onChange={membership => set({ membership })} /></div>}
    {limits.aggregate && <label className="setting-row"><SettingLabel label="Show as one total" hint="Sum the members into one series" />
      <input type="checkbox" role="switch" checked={selection.aggregate} onChange={event => set({ aggregate: event.currentTarget.checked })} /></label>}
  </>;
}

/** The groups a widget can offer: human players, each recorded faction, at war
 * with a country, and top N when the widget ranks countries. */
export function groupKinds(reply: CountryGroupReply | undefined, limits: Limits, hour?: number,
  overrides?: Readonly<Record<string, { name?: string }>>): { value: string; label: string; reason?: string }[] {
  return [
    { value: 'humans', label: 'Human players' },
    ...(reply?.factions ?? []).map(faction => {
      const named = factionNameAt(faction, hour, overrides?.[faction.id]?.name);
      return { value: `faction:${faction.id}`, label: named.name ?? UNNAMED_FACTION, reason: named.reason ?? undefined };
    }),
    { value: 'war', label: 'At war with a country' },
    ...(limits.ranked ? [{ value: 'top', label: 'Top countries' }] : []),
  ];
}
