import CountryPicker from '../../components/CountryPicker';
import { SourceFlag } from '../../components/CountryFlag';
import { Icon } from '../../components/Icons';
import { Segmented } from '../../components/Segmented';
import type { CountryGroupReply } from '../../generated/dashboard';
import { defaultCountryTag, selectedTags } from '../countrySelection';
import { DEFAULT_TOP } from '../groups';
import { useCountryGroups } from '../useCountryGroups';
import type { BoundTrack, CountryGroup, CountrySelection, GroupMembership, WidgetSpec } from '../model';
import type { WidgetMeta, WidgetRuntime } from '../definitions';

type Limits = NonNullable<WidgetMeta['countries']>;

/** One country from a picker; or, for widgets that show several, chosen countries
 * as removable chips or a smart group (human players, a faction, everyone at war
 * with a country, or the top N where the widget ranks). */
export function CountriesSection({ widget, reference, runtime, limits, onChange }: {
  widget: WidgetSpec; reference: BoundTrack | undefined; runtime: WidgetRuntime; limits: Limits; onChange: (countries: CountrySelection) => void;
}) {
  const source = reference?.source;
  const fallback = defaultCountryTag(reference);
  const common = { field: true, source, client: runtime.client, countries: source?.metadata.countries ?? [], playerCountryId: source?.metadata.playerCountryId ?? null };
  if (limits.max <= 1) return <CountryPicker key={source?.id ?? 'unbound'} {...common}
    selectedTag={widget.countries.kind === 'tags' ? widget.countries.tags[0] ?? null : fallback}
    onChange={tag => onChange(tag && tag !== fallback ? { kind: 'tags', tags: [tag] } : { kind: 'selected' })} />;
  const mode = widget.countries.kind === 'group' ? 'group' : widget.countries.kind === 'all' && limits.all ? 'all' : 'countries';
  // A widget about the whole world offers all countries first, and narrows to chosen ones or a group.
  const modes = [...(limits.all ? [{ value: 'all' as const, label: 'All' }] : []), { value: 'countries' as const, label: 'Countries' }, { value: 'group' as const, label: 'Group' }];
  return <>
    <Segmented label="Choose countries by" value={mode} options={modes}
      onChange={value => { if (value !== mode) onChange(value === 'group' ? { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false }
        : value === 'all' ? { kind: 'all' } : { kind: 'selected' }); }} />
    {mode === 'all' ? <p className="field-hint">Every country the recording names.</p>
      : widget.countries.kind === 'group'
        ? <GroupFields selection={widget.countries} reference={reference} runtime={runtime} limits={limits} onChange={onChange} />
        : <CountryChips widget={widget} reference={reference} runtime={runtime} max={limits.max} all={limits.all} onChange={onChange} />}
  </>;
}

function CountryChips({ widget, reference, runtime, max, all, onChange }: {
  widget: WidgetSpec; reference: BoundTrack | undefined; runtime: WidgetRuntime; max: number; all?: boolean; onChange: (countries: CountrySelection) => void;
}) {
  const source = reference?.source;
  const tags = selectedTags(widget.countries, reference);
  const follows = widget.countries.kind === 'selected';
  const setTags = (next: string[]) => onChange(next.length ? { kind: 'tags', tags: next } : { kind: all ? 'all' : 'selected' });
  const name = (tag: string) => source?.metadata.countries.find(country => country.tag === tag)?.name || tag;
  return <>
    <ul className="settings-countries" aria-label="Chosen countries">{tags.map(tag => <li key={tag} className="settings-country">
      {source && <SourceFlag client={runtime.client} source={source} tag={tag} />}
      <span>{name(tag)}{follows && <small>Dashboard country</small>}</span>
      {!follows && <button type="button" className="icon-button small" aria-label={`Remove ${name(tag)}`}
        onClick={() => setTags(tags.filter(item => item !== tag))}><Icon name="close" /></button>}
    </li>)}</ul>
    <CountryPicker key={source?.id ?? 'unbound'} field source={source} client={runtime.client} countries={source?.metadata.countries ?? []}
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

function GroupFields({ selection, reference, runtime, limits, onChange }: {
  selection: Extract<CountrySelection, { kind: 'group' }>; reference: BoundTrack | undefined; runtime: WidgetRuntime; limits: Limits;
  onChange: (countries: CountrySelection) => void;
}) {
  const groups = useCountryGroups(runtime, reference);
  const source = reference?.source;
  const set = (patch: Partial<Extract<CountrySelection, { kind: 'group' }>>) => onChange({ ...selection, ...patch });
  const group = selection.group;
  const kinds = groupKinds(groups.data, limits);
  // A saved faction this branch did not record (or that is still loading) stays
  // visible as the selection instead of the list silently showing another group.
  if (group.type === 'faction' && !kinds.some(kind => kind.value === `faction:${group.id}`)) {
    kinds.splice(1, 0, { value: `faction:${group.id}`, label: groups.data ? 'A faction not in this branch' : 'Loading factions…' });
  }
  const value = group.type === 'faction' ? `faction:${group.id}` : group.type;
  return <>
    <label className="setting-row"><span>Group</span>
      <select value={value} onChange={event => {
        const choice = event.currentTarget.value;
        const next: CountryGroup = choice.startsWith('faction:') ? { type: 'faction', id: choice.slice(8) }
          : choice === 'war' ? { type: 'war', tag: defaultCountryTag(reference) ?? '' }
            : choice === 'top' ? { type: 'top', count: Math.min(DEFAULT_TOP, limits.max) } : { type: 'humans' };
        if (next.type !== 'war' || next.tag) set({ group: next });
      }}>{kinds.map(kind => <option key={kind.value} value={kind.value}>{kind.label}</option>)}</select></label>
    {groups.error && <p className="field-hint" role="alert">Groups unavailable: {groups.error}</p>}
    {group.type === 'war' && <CountryPicker key={source?.id ?? 'unbound'} field source={source} client={runtime.client}
      countries={source?.metadata.countries ?? []} playerCountryId={source?.metadata.playerCountryId ?? null} selectedTag={group.tag}
      onChange={tag => { if (tag) set({ group: { type: 'war', tag } }); }} />}
    {group.type === 'top' && <label className="setting-row"><span>How many<small>Ranked by this widget's value each hour</small></span>
      <span className="option-number"><input type="number" min={1} max={limits.max} value={group.count}
        onChange={event => { const count = Math.round(Number(event.currentTarget.value)); if (count >= 1 && count <= limits.max) set({ group: { type: 'top', count } }); }} /></span></label>}
    {group.type !== 'top' && <div className="setting-row setting-row-stacked"><span>Members</span>
      <Segmented label="Group membership" value={selection.membership} options={MEMBERSHIP} onChange={membership => set({ membership })} /></div>}
    {limits.aggregate && <label className="setting-row"><span>Show as one total<small>Sum the members into one series</small></span>
      <input type="checkbox" role="switch" checked={selection.aggregate} onChange={event => set({ aggregate: event.currentTarget.checked })} /></label>}
  </>;
}

/** The groups a widget can offer: human players, each recorded faction, at war
 * with a country, and top N when the widget ranks countries. */
export function groupKinds(reply: CountryGroupReply | undefined, limits: Limits): { value: string; label: string }[] {
  return [
    { value: 'humans', label: 'Human players' },
    ...(reply?.factions ?? []).map(faction => ({ value: `faction:${faction.id}`, label: faction.name ?? 'Unnamed faction' })),
    { value: 'war', label: 'At war with a country' },
    ...(limits.ranked ? [{ value: 'top', label: 'Top countries' }] : []),
  ];
}
