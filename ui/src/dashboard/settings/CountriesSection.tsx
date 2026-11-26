import type { ReactElement } from 'react';
import { isSlotKey } from '../countryKey.ts';
import { failureText } from '../queryError.ts';
import CountryPicker from '../../components/CountryPicker';
import { Select } from '../../components/Select';
import { SettingLabel } from './SettingLabel';
import { CountryFlagUrl } from '../../components/CountryFlag';
import { Icon } from '../../components/Icons';
import { Segmented } from '../../components/Segmented';
import type { CountryGroupReply } from '../../generated/dashboard';
import { defaultCountryKey, selectedCountryKeys, singleCountryChoice } from '../countrySelection';
import { countrySelectionFor, countrySelectionMode, countrySelectionModes, DEFAULT_TOP } from '../groups';
import { useCampaignSettings } from '../campaignSettings';
import { factionChoiceLabel } from '../factionNames';
import { resolveWindow } from '../model';
import { useAtHead } from '../atHead.ts';
import { chipName, useKeyLabelsAt, useKeyTagsAt } from '../useKeyTagsAt.ts';
import type { CountryLabel } from '../countryFlagsAt';
import { useCountryGroups } from '../useCountryGroups';
import type { BoundTrack, CountryGroup, CountrySelection, GroupMembership, WidgetSpec } from '../model';
import type { WidgetMeta, WidgetRuntime } from '../definitions';

type Limits = NonNullable<WidgetMeta['countries']>;

export function CountriesSection({ widget, reference, runtime, limits, inline = false, onChange }: {
  widget: WidgetSpec; reference: BoundTrack | undefined; runtime: WidgetRuntime; limits: Limits; onChange: (countries: CountrySelection) => void;
  inline?: boolean;
}) {
  const source = reference?.source;
  const fallback = defaultCountryKey(reference);
  const common = { field: !inline, inline, source, client: runtime.client, countries: source?.metadata.countries ?? [], playerCountryId: source?.metadata.playerCountryId ?? null };
  const picker = (fallbackTag: string | null) => <CountryPicker key={source?.id ?? 'unbound'} {...common}
    selectedTag={widget.countries.kind === 'tags' ? widget.countries.tags[0] ?? null : fallbackTag}
    onChange={tag => onChange(singleCountryChoice(tag, fallbackTag, limits.fixed))} />;
  if (limits.max <= 1) return reference?.selectedTag != null || fallback === null || !isSlotKey(fallback) ? picker(reference?.selectedTag ?? fallback)
    : <TagsAt widget={widget} reference={reference} runtime={runtime} keys={[fallback]}>{([tag]) => picker(tag)}</TagsAt>;
  const mode = widget.countries.kind === 'all' && !limits.all ? 'countries' : countrySelectionMode(widget.countries);
  return <>
    <Segmented label="Choose countries by" value={mode} options={countrySelectionModes(limits)}
      onChange={value => { if (value !== mode) onChange(countrySelectionFor(value, limits.max)); }} />
    {mode === 'all' ? null
      : widget.countries.kind === 'group'
        ? <GroupFields widget={widget} selection={widget.countries} reference={reference} runtime={runtime} limits={limits} onChange={onChange} />
        : widget.countries.kind === 'factions'
          ? <FactionFields selection={widget.countries} max={limits.max} onChange={onChange} />
          : <CountryChips widget={widget} reference={reference} runtime={runtime} max={limits.max} inline={inline} onChange={onChange} />}
  </>;
}

function TagsAt({ widget, reference, runtime, keys, children }: {
  widget: WidgetSpec; reference: BoundTrack | undefined; runtime: WidgetRuntime; keys: readonly string[]; children: (tags: readonly (string | null)[]) => ReactElement;
}) {
  const shownHour = (clockHour: number) => reference ? resolveWindow(widget, reference, runtime.tracks, clockHour)?.lastHour ?? Number.NaN : Number.NaN;
  return children(useKeyTagsAt(runtime, reference, keys, shownHour, widget.time.mode === 'follow', true));
}

function LabelsAt({ widget, reference, runtime, keys, children }: {
  widget: WidgetSpec; reference: BoundTrack; runtime: WidgetRuntime; keys: readonly string[]; children: (labels: readonly ReturnType<CountryLabel>[]) => ReactElement;
}) {
  const shownHour = (clockHour: number) => resolveWindow(widget, reference, runtime.tracks, clockHour)?.lastHour ?? Number.NaN;
  return children(useKeyLabelsAt(runtime, reference, keys, shownHour, widget.time.mode === 'follow', true));
}

function CountryChips({ widget, reference, runtime, max, inline, onChange }: {
  widget: WidgetSpec; reference: BoundTrack | undefined; runtime: WidgetRuntime; max: number; inline: boolean; onChange: (countries: CountrySelection) => void;
}) {
  const source = reference?.source;
  const tags = selectedCountryKeys(widget.countries, reference);
  const follows = widget.countries.kind === 'selected';
  const setTags = (next: string[]) => onChange(next.length ? { kind: 'tags', tags: next } : { kind: 'selected' });
  const chips = (labels: readonly (ReturnType<CountryLabel> | undefined)[]) => <ul className="settings-countries" aria-label="Chosen countries">{tags.map((tag, i) => <li key={tag} className="settings-country">
    {labels[i] && <CountryFlagUrl className="country-flag" url={labels[i]!.flag} detail={labels[i]!.flagDetail} reason={labels[i]!.flagReason ?? undefined} pending={labels[i]!.flagPending} />}
    <span>{chipName(labels[i])}{follows && <small>Dashboard country</small>}</span>
    {!follows && <button type="button" className="icon-button small" aria-label={`Remove ${chipName(labels[i])}`}
      onClick={() => setTags(tags.filter(item => item !== tag))}><Icon name="close" /></button>}
  </li>)}</ul>;
  return <>
    {reference ? <LabelsAt widget={widget} reference={reference} runtime={runtime} keys={tags}>{chips}</LabelsAt> : chips([])}
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
  { value: 'ever', label: 'Anyone ever', tooltip: 'Every country that had joined by the displayed hour' },
];

function FactionFields({ selection, max, onChange }: {
  selection: Extract<CountrySelection, { kind: 'factions' }>; max: number; onChange: (countries: CountrySelection) => void;
}) {
  return <>
    <label data-setting="faction-count" className="setting-row"><SettingLabel label="How many" hint="Factions ranked by their total each hour" />
      <span className="option-number"><input type="number" min={1} max={max} placeholder="All"
        value={selection.count === Number.MAX_SAFE_INTEGER ? '' : selection.count}
        onChange={event => {
          const count = event.currentTarget.value === '' ? Number.MAX_SAFE_INTEGER : Math.round(Number(event.currentTarget.value));
          if (count >= 1 && count <= max) onChange({ ...selection, count });
        }} /></span></label>
    <div className="setting-row setting-row-stacked"><span>Members</span>
      <Segmented label="Faction membership" value={selection.membership} options={MEMBERSHIP} onChange={membership => onChange({ ...selection, membership })} /></div>
  </>;
}

function GroupFields({ widget, selection, reference, runtime, limits, onChange }: {
  widget: WidgetSpec; selection: Extract<CountrySelection, { kind: 'group' }>; reference: BoundTrack | undefined; runtime: WidgetRuntime; limits: Limits;
  onChange: (countries: CountrySelection) => void;
}) {
  const settings = useCampaignSettings();
  const overrides = settings.campaigns[reference?.source.metadata.campaignKey ?? '']?.factions;
  const groups = useCountryGroups(runtime, reference);
  const source = reference?.source;
  const set = (patch: Partial<Extract<CountrySelection, { kind: 'group' }>>) => onChange({ ...selection, ...patch });
  const group = selection.group;
  const kinds = [...useAtHead(runtime.clock, widget.time.mode === 'follow', clockHour => groupKinds(groups.data, limits,
    reference ? resolveWindow(widget, reference, runtime.tracks, clockHour)?.lastHour ?? Number.NaN : undefined, overrides), sameKinds)];
  if (group.type === 'faction' && !kinds.some(kind => kind.value === `faction:${group.id}`)) {
    kinds.splice(1, 0, { value: `faction:${group.id}`, label: groups.data ? 'Faction unavailable' : 'Loading factions…' });
  }
  const value = group.type === 'faction' ? `faction:${group.id}` : group.type;
  return <>
    <div className="setting-row"><span>Group</span>
      <Select label="Group" value={value} options={kinds} onChange={choice => {
        const next: CountryGroup = choice.startsWith('faction:') ? { type: 'faction', id: choice.slice(8) }
          : choice === 'war' ? { type: 'war', tag: defaultCountryKey(reference) ?? '' }
            : choice === 'top' ? { type: 'top', count: Math.min(DEFAULT_TOP, limits.max) } : { type: 'humans' };
        if (next.type !== 'war' || next.tag) set({ group: next });
      }} /></div>
    {groups.error && <p className="field-hint" role="alert" data-detail={groups.error.message}>{failureText(groups.error, 'Couldn’t load groups.')}</p>}
    {group.type === 'war' && <CountryPicker key={source?.id ?? 'unbound'} field source={source} client={runtime.client}
      countries={source?.metadata.countries ?? []} playerCountryId={source?.metadata.playerCountryId ?? null} selectedTag={group.tag}
      onChange={tag => { if (tag) set({ group: { type: 'war', tag } }); }} />}
    {group.type === 'top' && <label data-setting="top-count" className="setting-row"><SettingLabel label="How many" hint={`Ranked by ${limits.rankedBy ?? "this widget's value"} each hour`} />
      <span className="option-number"><input type="number" min={1} max={limits.max} value={group.count}
        onChange={event => { const count = Math.round(Number(event.currentTarget.value)); if (count >= 1 && count <= limits.max) set({ group: { type: 'top', count } }); }} /></span></label>}
    {group.type !== 'top' && <div className="setting-row setting-row-stacked"><span>Members</span>
      <Segmented label="Group membership" value={selection.membership} options={MEMBERSHIP} onChange={membership => set({ membership })} /></div>}
    {limits.aggregate && <label className="setting-row"><SettingLabel label="Show as one total" />
      <input type="checkbox" role="switch" checked={selection.aggregate} onChange={event => set({ aggregate: event.currentTarget.checked })} /></label>}
  </>;
}

const sameKinds = (a: readonly { value: string; label: string }[], b: readonly { value: string; label: string }[]) =>
  a.length === b.length && a.every((kind, index) => kind.value === b[index].value && kind.label === b[index].label);

export function groupKinds(reply: CountryGroupReply | undefined, limits: Limits, hour?: number,
  overrides?: Readonly<Record<string, { name?: string }>>): { value: string; label: string }[] {
  return [
    { value: 'humans', label: 'Human players' },
    ...(reply?.factions ?? []).map(faction => ({ value: `faction:${faction.id}`, label: factionChoiceLabel(faction, hour, overrides?.[faction.id]?.name) })),
    { value: 'war', label: 'At war with a country' },
    ...(limits.ranked ? [{ value: 'top', label: 'Top countries' }] : []),
  ];
}
