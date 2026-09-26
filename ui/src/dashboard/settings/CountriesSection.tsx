import CountryPicker from '../../components/CountryPicker';
import { SourceFlag } from '../../components/CountryFlag';
import { Icon } from '../../components/Icons';
import { defaultCountryTag, selectedTags } from '../countrySelection';
import type { BoundTrack, CountrySelection, WidgetSpec } from '../model';
import type { WidgetRuntime } from '../definitions';

/** One country from a picker, or several as removable chips up to the widget's limit. */
export function CountriesSection({ widget, reference, runtime, max, onChange }: {
  widget: WidgetSpec; reference: BoundTrack | undefined; runtime: WidgetRuntime; max: number; onChange: (countries: CountrySelection) => void;
}) {
  const source = reference?.source;
  const fallback = defaultCountryTag(reference);
  const common = { field: true, source, client: runtime.client, countries: source?.metadata.countries ?? [], playerCountryId: source?.metadata.playerCountryId ?? null };
  if (max <= 1) return <CountryPicker key={source?.id ?? 'unbound'} {...common}
    selectedTag={widget.countries.kind === 'tags' ? widget.countries.tags[0] ?? null : fallback}
    onChange={tag => onChange(tag && tag !== fallback ? { kind: 'tags', tags: [tag] } : { kind: 'selected' })} />;
  const tags = selectedTags(widget.countries, reference);
  const follows = widget.countries.kind === 'selected';
  const setTags = (next: string[]) => onChange(next.length ? { kind: 'tags', tags: next } : { kind: 'selected' });
  const name = (tag: string) => source?.metadata.countries.find(country => country.tag === tag)?.name || tag;
  return <>
    <ul className="settings-countries" aria-label="Chosen countries">{tags.map(tag => <li key={tag} className="settings-country">
      {source && <SourceFlag client={runtime.client} source={source} tag={tag} />}
      <span>{name(tag)}{follows && <small>Dashboard country</small>}</span>
      {!follows && <button type="button" className="icon-button small" aria-label={`Remove ${name(tag)}`}
        onClick={() => setTags(tags.filter(item => item !== tag))}><Icon name="close" /></button>}
    </li>)}</ul>
    <CountryPicker key={source?.id ?? 'unbound'} {...common} selectedTag={null} chosen={tags}
      addLabel={tags.length >= max ? `Up to ${max} countries; pick one to remove it` : 'Add or remove a country'}
      onChange={tag => {
        if (!tag) return;
        if (tags.includes(tag)) setTags(tags.filter(item => item !== tag));
        else if (tags.length < max) setTags([...tags, tag]);
      }} />
  </>;
}
