import { Popover, usePopover } from '../ui/Popover';
import { useEffect, useId, useMemo, useState } from 'react';
import type { CountryMetadata } from '../api';
import { Icon } from '../Icons';
import { CountryFlag } from './CountryFlag';
import { countryData } from './countryData';
import type { CountryFlagImage, DashboardClient, DashboardSource } from './client';

export default function CountryPicker({ countries, selectedTag, playerCountryId, onChange, onCountries, onReconnect, source, client, field = false }: {
  countries: CountryMetadata[]; selectedTag: string | null; playerCountryId: string | null;
  onChange: (tag: string | null) => void; source: DashboardSource | null | undefined; client: DashboardClient;
  onCountries?: (sourceId: string, revision: string, countries: CountryMetadata[]) => void;
  onReconnect?: () => void; field?: boolean;
}) {
  const id = useId();
  const data = countryData(client);
  const menu = usePopover();
  const { trigger, popup, open } = menu;
  const [query, setQuery] = useState('');
  const [retry, setRetry] = useState(0);
  const sourceKey = source ? `${source.id}:${source.revision}` : '';
  const [roster, setRoster] = useState<{ key: string; countries: CountryMetadata[]; error: string | null; warning: string | null }>({ key: '', countries: [], error: null, warning: null });
  const loading = !!source && roster.key !== sourceKey;
  const choices = useMemo(() => {
    const byTag = new Map<string, CountryMetadata>();
    if (roster.key === sourceKey) for (const country of roster.countries) if (!byTag.has(country.tag)) byTag.set(country.tag, country);
    return [...byTag.values()].sort((a, b) => (a.name || a.tag).localeCompare(b.name || b.tag));
  }, [roster, sourceKey]);
  const selected = selectedTag == null ? countries.find((country) => country.id === playerCountryId)
    : choices.find((country) => country.tag === selectedTag) ?? countries.find((country) => country.tag === selectedTag);
  const filtered = choices.filter((country) => `${country.name} ${country.tag}`.toLowerCase().includes(query.trim().toLowerCase()));
  const [flagState, setFlagState] = useState<{ key: string; images: Record<string, CountryFlagImage>; error: string | null }>({ key: '', images: {}, error: null });
  const [headerFlag, setHeaderFlag] = useState<{ key: string; image?: CountryFlagImage; error: string | null }>({ key: '', error: null });
  const headerTag = selected?.tag ?? selectedTag;
  const headerKey = `${sourceKey}:${headerTag}:${selected?.observedBranchId}:${selected?.observedHour}`;
  const flagImages = flagState.key === sourceKey ? flagState.images : {};
  const flagError = [flagState.key === sourceKey ? flagState.error : null, headerFlag.key === headerKey ? headerFlag.error : null].filter(Boolean).join('\n') || null;
  const flagContext = useMemo(() => {
    if (!source?.metadata.branches.length) return null;
    const witness = selected ?? source.metadata.countries.find((country) => country.id === source.metadata.playerCountryId) ?? source.metadata.countries[0];
    const branch = source.metadata.branches.find((branch) => branch.id === witness?.observedBranchId) ?? source.metadata.branches[0];
    return { sourceId: source.id, revision: source.revision, branchId: branch.id,
      hour: witness?.observedBranchId === branch.id ? witness.observedHour : branch.firstHour };
  }, [source, selected]);
  useEffect(() => {
    if (!flagContext || !headerTag || !client.countryFlags) return;
    const controller = new AbortController();
    void data.flags({ ...flagContext, size: 'full', tags: [headerTag] }, controller.signal).then(result => {
      if (!controller.signal.aborted) setHeaderFlag({ key: headerKey, image: result.flags.find(image => image.tag === headerTag), error: result.warnings.join('\n') || null });
    }).catch(error => {
      if (!controller.signal.aborted) setHeaderFlag(previous => ({ key: headerKey, image: previous.key === headerKey ? previous.image : undefined, error: String(error) }));
    });
    return () => controller.abort();
  }, [flagContext, client, headerTag, headerKey, retry]);
  useEffect(() => {
    if (!source || !client.landOwners) return;
    const controller = new AbortController();
    void data.owners(source.id, source.revision, controller.signal).then(result => {
      if (!controller.signal.aborted) {
        setRoster({ key: sourceKey, countries: result.countries, error: null, warning: result.warnings.join('\n') || null });
        onCountries?.(result.sourceId, result.revision, result.countries);
      }
    }).catch(error => {
      if (!controller.signal.aborted) setRoster(previous => ({ key: sourceKey, countries: previous.key === sourceKey ? previous.countries : [], error: String(error), warning: null }));
    });
    return () => controller.abort();
  }, [sourceKey, client, onCountries, retry]);
  useEffect(() => {
    if (!source || !client.countryFlags || !choices.length) return;
    const controller = new AbortController();
    void (async () => {
      const images: Record<string, CountryFlagImage> = {};
      let warning: string | null = null;
      try {
        const groups = new Map<string, CountryMetadata[]>();
        for (const country of choices) {
          const key = `${country.observedBranchId}:${country.observedHour}`;
          const group = groups.get(key) ?? []; group.push(country); groups.set(key, group);
        }
        for (const group of groups.values()) for (let start = 0; start < group.length; start += 32) {
          const result = await data.flags({ sourceId: source.id, revision: source.revision, branchId: group[0].observedBranchId, hour: group[0].observedHour, size: 'small', tags: group.slice(start, start + 32).map((country) => country.tag) }, controller.signal);
          if (controller.signal.aborted) return;
          for (const image of result.flags) images[image.tag] = image;
          warning = result.warnings.length ? result.warnings.join('\n') : warning;
          setFlagState(previous => ({ key: sourceKey, images: { ...(previous.key === sourceKey ? previous.images : {}), ...images }, error: warning }));
        }
      } catch (error) {
        if (!controller.signal.aborted) setFlagState(previous => ({ key: sourceKey, images: { ...(previous.key === sourceKey ? previous.images : {}), ...images }, error: String(error) }));
      }
    })();
    return () => controller.abort();
  }, [source, sourceKey, client, choices, retry]);
  const retryCountries = () => {
    if (onReconnect && [roster.error, flagError].some(error => error?.includes('source is unavailable or changed'))) onReconnect();
    else { if (source && flagError) data.retryFlags(source.id); setRetry(value => value + 1); }
  };
  const choose = (tag: string | null) => { onChange(tag); menu.close(); };
  return <div className={field ? 'field widget-country-field' : 'header-control header-country'}>
    <span className={field ? undefined : 'sr-only'}>Country</span>
    <button ref={trigger} type="button" className={field ? 'widget-country-trigger' : 'header-country-trigger'} disabled={!source} popoverTarget={id}
      aria-label={field ? 'Widget country' : 'Choose country'} aria-haspopup="dialog" aria-expanded={open}>
      <CountryFlag image={headerFlag.key === headerKey ? headerFlag.image : undefined} /><span>{selected?.name || selected?.tag || selectedTag || 'Choose country'}</span>
      {field && <Icon name="chevron-down" />}
    </button>
    <Popover menu={menu} portal={!field} id={id} aria-label="Choose country" className="country-picker"
      onToggle={(event) => { if (event.newState === 'closed') setQuery(''); }}>
      {open && <>
        <div className="country-picker-search"><input data-popover-autofocus aria-label="Search countries" placeholder="Search countries or tags" value={query} onChange={(event) => setQuery(event.currentTarget.value)}
          onKeyDown={(event) => { if (event.key === 'ArrowDown') { event.preventDefault(); popup.current?.querySelector<HTMLButtonElement>('.country-picker-list button')?.focus(); } }} /><span>{choices.length}</span></div>
        <div className="country-picker-list" onKeyDown={(event) => {
          const buttons = [...event.currentTarget.querySelectorAll<HTMLButtonElement>('button')];
          const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
          if (index < 0 || !['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
          event.preventDefault();
          const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : (index + (event.key === 'ArrowDown' ? 1 : -1) + buttons.length) % buttons.length;
          buttons[next]?.focus();
        }}>
          {filtered.map((country) => <button type="button" key={country.tag} aria-pressed={selected?.tag === country.tag} onClick={() => choose(country.tag)}>
            <CountryFlag image={flagImages[country.tag]} /><span>{country.name || country.tag}</span><small>{country.tag}</small>{selected?.tag === country.tag && <span className="country-selected-mark">{'\u2713'}</span>}
          </button>)}
          {flagError && !Object.keys(flagImages).length && <div className="country-picker-note"><span>Flags unavailable</span><button type="button" onClick={retryCountries}>Retry</button></div>}
          {loading && <p className="country-picker-empty" role="status">Finding countries that owned land…</p>}
          {!loading && roster.error && <div className="country-picker-note" data-tooltip={roster.error}><span>Could not load countries</span><button type="button" onClick={retryCountries}>Retry</button></div>}
          {!loading && roster.warning && <div className="country-picker-note" data-tooltip={roster.warning}><Icon name="info" /><span>Some ownership history is unavailable</span></div>}
          {!loading && !roster.error && !filtered.length && <p className="country-picker-empty">{query ? 'No matching countries' : 'No recorded land-owning countries'}</p>}
        </div>
      </>}
    </Popover>
  </div>;
}
