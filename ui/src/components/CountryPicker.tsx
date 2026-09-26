import { Popover, usePopover } from './Popover';
import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { CountryMetadata } from '../shell/api';
import { Icon } from './Icons';
import { CountryFlag } from './CountryFlag';
import { forgetSettledProducts, loadProduct, loadTransientProduct } from '../dashboard/products';
import { FlagStore, flagStore, loadListFlags } from '../dashboard/flagCache';
import type { DashboardClient, DashboardSource, ProductRequest } from '../dashboard/client';
import type { CountryFlagImage, CountryFlagParams, CountryFlagReply, CountryFlagSize, LandOwners } from '../generated/dashboard';
import { COUNTRY_ROW_HEIGHT, countryChoices, filterChoices, moveIndex, revealRow, visibleRange } from './countryList';

/** `.country-picker-list` max-height, used until the list is measured. */
const LIST_HEIGHT = 340;

function flagsRequest(context: { sourceId: string; revision: string; branchId: string; hour: number }, size: CountryFlagSize, tags: string[]): ProductRequest<CountryFlagParams> {
  return { sourceId: context.sourceId, revision: context.revision, product: 'country-flags', params: { branchId: context.branchId, hour: context.hour, tags, size } };
}

/** Flags are always the game's own files from the recording's content stack. The
 * header shows the country as the game's top bar does (`shield`) at `flagAt`, the
 * playback day; lists use the game's medium flags at each country's observed hour.
 * The list renders only the rows in view and loads only their flags, so it costs
 * the same at any country count. */
export default function CountryPicker({ countries, selectedTag, playerCountryId, onChange, onCountries, onReconnect, source, client, field = false, flagAt = null, addLabel, chosen }: {
  countries: CountryMetadata[]; selectedTag: string | null; playerCountryId: string | null;
  onChange: (tag: string | null) => void; source: DashboardSource | null | undefined; client: DashboardClient;
  onCountries?: (sourceId: string, revision: string, countries: CountryMetadata[]) => void;
  onReconnect?: () => void; field?: boolean; flagAt?: { branchId: string; hour: number } | null;
  /** A multi-country field: the trigger reads this label and every `chosen` tag is ticked. */
  addLabel?: string; chosen?: readonly string[];
}) {
  const id = useId();
  const listId = `${id}-countries`;
  const optionId = (index: number) => `${id}-country-${index}`;
  const menu = usePopover(field ? 'end' : 'start');
  const { trigger, open } = menu;
  const list = useRef<HTMLDivElement>(null);
  const listbox = useRef<HTMLDivElement>(null);
  const [query, setQuery] = useState('');
  const [active, setActive] = useState(-1);
  const [view, setView] = useState({ scrollTop: 0, height: LIST_HEIGHT });
  const [retry, setRetry] = useState(0);
  const sourceKey = source ? `${source.id}:${source.revision}` : '';
  const [roster, setRoster] = useState<{ key: string; countries: CountryMetadata[]; error: string | null; warning: string | null }>({ key: '', countries: [], error: null, warning: null });
  const loading = !!source && roster.key !== sourceKey;
  const choices = useMemo(() => roster.key === sourceKey ? countryChoices(roster.countries) : [], [roster, sourceKey]);
  const byTag = useMemo(() => new Map(choices.map(({ country }) => [country.tag, country])), [choices]);
  const selected = useMemo(() => selectedTag == null ? countries.find((country) => country.id === playerCountryId)
    : byTag.get(selectedTag) ?? countries.find((country) => country.tag === selectedTag), [countries, playerCountryId, selectedTag, byTag]);
  const filtered = useMemo(() => filterChoices(choices, query), [choices, query]);
  const range = visibleRange(filtered.length, view.scrollTop, view.height);
  const visible = open ? filtered.slice(range.start, range.end) : [];
  const [flagState, setFlagState] = useState<{ key: string; loads: number; error: string | null }>({ key: '', loads: 0, error: null });
  const [headerFlag, setHeaderFlag] = useState<{ identity: string; key: string; image?: CountryFlagImage; error: string | null }>({ identity: '', key: '', error: null });
  const headerTag = addLabel ? null : selected?.tag ?? selectedTag;
  const headerSize: CountryFlagSize = field ? 'medium' : 'shield';
  const flags = flagStore(client);
  const flagOf = (country: CountryMetadata) => source
    ? flags.get(FlagStore.key(source.id, source.revision, country.observedBranchId, country.observedHour, country.tag)) ?? undefined : undefined;
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
      if (!controller.signal.aborted) setHeaderFlag({ identity: headerIdentity, key: headerKey, image: result.flags.find(image => image.tag === headerTag), error: null });
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
  // Only the rows in view ask for flags; stored flags are never requested again.
  // JSON escapes line breaks, so one line per row keeps the key exact.
  const visibleTargets = visible.map(({ country }) => JSON.stringify([country.observedBranchId, country.observedHour, country.tag])).join('\n');
  useEffect(() => {
    if (!source || !visibleTargets) return;
    const targets = visibleTargets.split('\n').map(line => {
      const [branchId, hour, tag] = JSON.parse(line) as [string, number, string];
      return { branchId, hour, tag };
    });
    const controller = new AbortController();
    // Reply warnings describe the recording (such as its content coverage), not
    // a failed flag; only a failed request counts as a flag error.
    void loadListFlags(client, source, targets, controller.signal).then(() => {
      if (!controller.signal.aborted) setFlagState(previous => ({ key: sourceKey, loads: (previous.key === sourceKey ? previous.loads : 0) + 1, error: null }));
    }).catch(error => {
      if (!controller.signal.aborted) setFlagState(previous => ({ key: sourceKey, loads: previous.key === sourceKey ? previous.loads : 0, error: String(error) }));
    });
    return () => controller.abort();
  }, [source, sourceKey, client, visibleTargets, retry]);
  useLayoutEffect(() => {
    if (open && list.current) setView({ scrollTop: list.current.scrollTop, height: list.current.clientHeight || LIST_HEIGHT });
  }, [open]);
  const scrollTo = (scrollTop: number) => {
    if (list.current) list.current.scrollTop = scrollTop;
    setView(previous => ({ ...previous, scrollTop }));
  };
  const reset = () => { setActive(-1); scrollTo(0); };
  const moveTo = (index: number) => {
    setActive(index);
    scrollTo(revealRow(index, list.current?.scrollTop ?? view.scrollTop, list.current?.clientHeight || view.height));
  };
  const retryCountries = () => {
    if (onReconnect && [roster.error, flagError].some(error => error?.includes('source is unavailable or changed'))) onReconnect();
    else { if (source && flagError) forgetSettledProducts(client, source.id); setRetry(value => value + 1); }
  };
  const choose = (tag: string | null) => { onChange(tag); menu.close(); };
  const flagsMissing = flagState.key === sourceKey && !!flagState.error && !visible.some(({ country }) => flagOf(country));
  return <div className={field ? 'field widget-country-field' : 'header-control header-country'}>
    <span className={field ? undefined : 'sr-only'}>Country</span>
    <button ref={trigger} type="button" className={field ? 'widget-country-trigger' : 'header-country-trigger'} disabled={!source} popoverTarget={id}
      aria-label={addLabel ?? (field ? 'Widget country' : 'Choose country')} aria-haspopup="dialog" aria-expanded={open}>
      {addLabel ? <Icon name="plus" /> : <CountryFlag image={headerImage} />}
      {addLabel ? <span>{addLabel}</span> : field ? <span>{selected?.name || selected?.tag || selectedTag || 'Choose country'}</span>
        : <span className="header-country-text"><strong>{selected?.name || selected?.tag || selectedTag || (source ? 'Choose country' : 'No campaign')}</strong></span>}
      <Icon name="chevron-down" />
    </button>
    <Popover menu={menu} portal={!field} id={id} aria-label="Choose country" className="country-picker"
      onToggle={(event) => { if (event.newState === 'closed') { setQuery(''); reset(); } }}>
      {open && <>
        <div className="country-picker-search"><Icon name="search" /><input data-popover-autofocus aria-label="Search countries" placeholder="Search countries or tags" aria-controls={listId} value={query}
          onChange={(event) => { setQuery(event.currentTarget.value); reset(); }}
          onKeyDown={(event) => {
            if (event.key === 'ArrowDown' && filtered.length) { event.preventDefault(); listbox.current?.focus({ preventScroll: true }); moveTo(0); }
          }} /><span className="pill">{choices.length}</span></div>
        <div ref={list} className="country-picker-list" onScroll={(event) => {
          const { scrollTop, clientHeight } = event.currentTarget;
          setView({ scrollTop, height: clientHeight || LIST_HEIGHT });
        }}>
          <div ref={listbox} id={listId} role="listbox" aria-label="Countries" tabIndex={filtered.length ? 0 : -1} className="country-picker-options"
            aria-activedescendant={active >= 0 && active < filtered.length ? optionId(active) : undefined}
            style={{ height: filtered.length * COUNTRY_ROW_HEIGHT }}
            // Mark the first row in view without scrolling: a click focuses the
            // list before its own row receives the click.
            onFocus={(event) => { if (event.target === event.currentTarget && active < 0 && filtered.length) setActive(Math.min(filtered.length - 1, Math.ceil(view.scrollTop / COUNTRY_ROW_HEIGHT))); }}
            onKeyDown={(event) => {
              if ((event.key === 'Enter' || event.key === ' ') && active >= 0 && active < filtered.length) {
                event.preventDefault(); choose(filtered[active].country.tag); return;
              }
              const next = moveIndex(event.key, active, filtered.length);
              if (next == null) return;
              event.preventDefault();
              moveTo(next);
            }}>
            {visible.map(({ country }, offset) => {
              const index = range.start + offset;
              const ticked = chosen ? chosen.includes(country.tag) : selected?.tag === country.tag;
              return <div key={country.tag} id={optionId(index)} role="option" aria-selected={ticked} aria-setsize={filtered.length} aria-posinset={index + 1}
                className="country-picker-option" data-active={index === active || undefined} style={{ top: index * COUNTRY_ROW_HEIGHT }}
                onClick={() => choose(country.tag)}>
                <CountryFlag image={flagOf(country)} /><span>{country.name || country.tag}</span><small>{country.tag}</small>{ticked && <Icon name="check" className="country-selected-mark" />}
              </div>;
            })}
          </div>
          {flagsMissing && <div className="country-picker-note"><Icon name="alert" /><span>Flags unavailable</span><button type="button" className="button small quiet" onClick={retryCountries}>Retry</button></div>}
          {loading && <p className="country-picker-empty" role="status"><span className="spinner" />Finding countries that owned land…</p>}
          {!loading && roster.error && <div className="country-picker-note" data-tooltip={roster.error}><Icon name="alert" /><span>Could not load countries</span><button type="button" className="button small quiet" onClick={retryCountries}>Retry</button></div>}
          {!loading && roster.warning && <div className="country-picker-note" data-tooltip={roster.warning}><Icon name="info" /><span>Some ownership history is unavailable</span></div>}
          {!loading && !roster.error && !filtered.length && <p className="country-picker-empty">{query ? 'No matching countries' : 'No recorded land-owning countries'}</p>}
        </div>
      </>}
    </Popover>
  </div>;
}
