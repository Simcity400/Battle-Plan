import { Popover, usePopover } from './Popover';
import { useEffect, useId, useMemo, useState } from 'react';
import type { CountryMetadata } from '../shell/api';
import { Icon } from './Icons';
import { CountryFlag } from './CountryFlag';
import { forgetSettledProducts, loadProduct, loadTransientProduct } from '../dashboard/products';
import type { DashboardClient, DashboardSource, ProductRequest } from '../dashboard/client';
import type { CountryFlagImage, CountryFlagParams, CountryFlagReply, CountryFlagSize, LandOwners } from '../generated/dashboard';

function flagsRequest(context: { sourceId: string; revision: string; branchId: string; hour: number }, size: CountryFlagSize, tags: string[]): ProductRequest<CountryFlagParams> {
  return { sourceId: context.sourceId, revision: context.revision, product: 'country-flags', params: { branchId: context.branchId, hour: context.hour, tags, size } };
}

/** Flags are always the game's own files from the recording's content stack. The
 * header shows the country as the game's top bar does (`shield`) at `flagAt`, the
 * playback day; lists use the game's medium flags at each country's observed hour. */
export default function CountryPicker({ countries, selectedTag, playerCountryId, onChange, onCountries, onReconnect, source, client, field = false, flagAt = null }: {
  countries: CountryMetadata[]; selectedTag: string | null; playerCountryId: string | null;
  onChange: (tag: string | null) => void; source: DashboardSource | null | undefined; client: DashboardClient;
  onCountries?: (sourceId: string, revision: string, countries: CountryMetadata[]) => void;
  onReconnect?: () => void; field?: boolean; flagAt?: { branchId: string; hour: number } | null;
}) {
  const id = useId();
  const menu = usePopover(field ? 'end' : 'start');
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
  const [headerFlag, setHeaderFlag] = useState<{ identity: string; key: string; image?: CountryFlagImage; error: string | null }>({ identity: '', key: '', error: null });
  const headerTag = selected?.tag ?? selectedTag;
  const headerSize: CountryFlagSize = field ? 'medium' : 'shield';
  const flagImages = flagState.key === sourceKey ? flagState.images : {};
  const flagContext = useMemo(() => {
    if (!source?.metadata.branches.length) return null;
    const witness = selected ?? source.metadata.countries.find((country) => country.id === source.metadata.playerCountryId) ?? source.metadata.countries[0];
    const branch = source.metadata.branches.find((branch) => branch.id === witness?.observedBranchId) ?? source.metadata.branches[0];
    return { sourceId: source.id, revision: source.revision, branchId: branch.id,
      hour: witness?.observedBranchId === branch.id ? witness.observedHour : branch.firstHour };
  }, [source, selected]);
  const live = !field && !!flagAt && !!source;
  const headerContext = useMemo(() => live ? { sourceId: source!.id, revision: source!.revision, branchId: flagAt!.branchId, hour: flagAt!.hour } : flagContext,
    [live, source, flagAt?.branchId, flagAt?.hour, flagContext]);
  const headerIdentity = `${sourceKey}:${headerTag}:${headerSize}`;
  const headerKey = `${headerIdentity}:${headerContext?.branchId}:${headerContext?.hour}`;
  // A new playback day keeps the current flag on screen until its own flag arrives.
  const headerImage = headerFlag.identity === headerIdentity ? headerFlag.image : undefined;
  const flagError = [flagState.key === sourceKey ? flagState.error : null, headerFlag.key === headerKey ? headerFlag.error : null].filter(Boolean).join('\n') || null;
  useEffect(() => {
    if (!headerContext || !headerTag) return;
    const controller = new AbortController();
    const request = flagsRequest(headerContext, headerSize, [headerTag]);
    // Per-day requests bypass the shared cache so they cannot evict full-branch products.
    void (live ? loadTransientProduct<CountryFlagReply>(client, request, controller.signal) : loadProduct<CountryFlagReply>(client, request, controller.signal)).then(result => {
      if (!controller.signal.aborted) setHeaderFlag({ identity: headerIdentity, key: headerKey, image: result.flags.find(image => image.tag === headerTag), error: result.warnings.join('\n') || null });
    }).catch(error => {
      // An unreadable hour shows no flag rather than carrying one across it.
      if (!controller.signal.aborted) setHeaderFlag({ identity: headerIdentity, key: headerKey, image: undefined, error: String(error) });
    });
    return () => controller.abort();
  }, [headerContext, client, headerTag, headerKey, headerIdentity, headerSize, live, retry]);
  useEffect(() => {
    if (!source) return;
    const controller = new AbortController();
    void loadProduct<LandOwners>(client, { sourceId: source.id, revision: source.revision, product: 'land-owners', params: {} }, controller.signal).then(result => {
      if (!controller.signal.aborted) {
        setRoster({ key: sourceKey, countries: result.countries, error: null, warning: result.warnings.join('\n') || null });
        onCountries?.(source.id, source.revision, result.countries);
      }
    }).catch(error => {
      if (!controller.signal.aborted) setRoster(previous => ({ key: sourceKey, countries: previous.key === sourceKey ? previous.countries : [], error: String(error), warning: null }));
    });
    return () => controller.abort();
  }, [sourceKey, client, onCountries, retry]);
  useEffect(() => {
    if (!source || !choices.length) return;
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
          const result = await loadProduct<CountryFlagReply>(client, flagsRequest({ sourceId: source.id, revision: source.revision, branchId: group[0].observedBranchId, hour: group[0].observedHour }, 'medium', group.slice(start, start + 32).map((country) => country.tag)), controller.signal);
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
    else { if (source && flagError) forgetSettledProducts(client, source.id); setRetry(value => value + 1); }
  };
  const choose = (tag: string | null) => { onChange(tag); menu.close(); };
  return <div className={field ? 'field widget-country-field' : 'header-control header-country'}>
    <span className={field ? undefined : 'sr-only'}>Country</span>
    <button ref={trigger} type="button" className={field ? 'widget-country-trigger' : 'header-country-trigger'} disabled={!source} popoverTarget={id}
      aria-label={field ? 'Widget country' : 'Choose country'} aria-haspopup="dialog" aria-expanded={open}>
      <CountryFlag image={headerImage} />
      {field ? <span>{selected?.name || selected?.tag || selectedTag || 'Choose country'}</span>
        : <span className="header-country-text"><strong>{selected?.name || selected?.tag || selectedTag || (source ? 'Choose country' : 'No campaign')}</strong></span>}
      <Icon name="chevron-down" />
    </button>
    <Popover menu={menu} portal={!field} id={id} aria-label="Choose country" className="country-picker"
      onToggle={(event) => { if (event.newState === 'closed') setQuery(''); }}>
      {open && <>
        <div className="country-picker-search"><Icon name="search" /><input data-popover-autofocus aria-label="Search countries" placeholder="Search countries or tags" value={query} onChange={(event) => setQuery(event.currentTarget.value)}
          onKeyDown={(event) => { if (event.key === 'ArrowDown') { event.preventDefault(); popup.current?.querySelector<HTMLButtonElement>('.country-picker-list button')?.focus(); } }} /><span className="pill">{choices.length}</span></div>
        <div className="country-picker-list" onKeyDown={(event) => {
          const buttons = [...event.currentTarget.querySelectorAll<HTMLButtonElement>('button')];
          const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
          if (index < 0 || !['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
          event.preventDefault();
          const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : (index + (event.key === 'ArrowDown' ? 1 : -1) + buttons.length) % buttons.length;
          buttons[next]?.focus();
        }}>
          {filtered.map((country) => <button type="button" key={country.tag} aria-pressed={selected?.tag === country.tag} onClick={() => choose(country.tag)}>
            <CountryFlag image={flagImages[country.tag]} /><span>{country.name || country.tag}</span><small>{country.tag}</small>{selected?.tag === country.tag && <Icon name="check" className="country-selected-mark" />}
          </button>)}
          {flagError && !Object.keys(flagImages).length && <div className="country-picker-note"><Icon name="alert" /><span>Flags unavailable</span><button type="button" className="button small quiet" onClick={retryCountries}>Retry</button></div>}
          {loading && <p className="country-picker-empty" role="status"><span className="spinner" />Finding countries that owned land…</p>}
          {!loading && roster.error && <div className="country-picker-note" data-tooltip={roster.error}><Icon name="alert" /><span>Could not load countries</span><button type="button" className="button small quiet" onClick={retryCountries}>Retry</button></div>}
          {!loading && roster.warning && <div className="country-picker-note" data-tooltip={roster.warning}><Icon name="info" /><span>Some ownership history is unavailable</span></div>}
          {!loading && !roster.error && !filtered.length && <p className="country-picker-empty">{query ? 'No matching countries' : 'No recorded land-owning countries'}</p>}
        </div>
      </>}
    </Popover>
  </div>;
}
