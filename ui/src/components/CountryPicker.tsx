import { Popover, usePopover } from './Popover';
import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { CountryMetadata } from '../shell/api';
import { Icon } from './Icons';
import { CountryFlag } from './CountryFlag';
import { forgetSettledProducts, loadProduct, loadTransientProduct } from '../dashboard/products';
import { FlagStore, flagStore, loadListFlags } from '../dashboard/flagCache';
import { useContentRevision } from '../dashboard/contentRevision';
import { nameKey, useCountryNamesAt } from '../dashboard/countryNamesAt';
import type { DashboardClient, DashboardSource, ProductRequest } from '../dashboard/client';
import type { CountryFlagImage, CountryFlagParams, CountryFlagReply, CountryFlagSize, LandOwners } from '../generated/dashboard';
import { COUNTRY_ROW_HEIGHT, allowedCountryChoices, countryChoices, filterChoices, moveIndex, revealRow, visibleRange } from './countryList';
import type { CountryOption } from './countryList';

/** `.country-picker-list` max-height, used until the list is measured. */
const LIST_HEIGHT = 340;
const NO_NAMES: ReadonlyMap<string, string> = new Map();
/** An unchanged list viewport keeps its state, so measuring and scrolling never re-render for nothing. */
const listView = (scrollTop: number, height: number) => (before: { scrollTop: number; height: number }) =>
  before.scrollTop === scrollTop && before.height === height ? before : { scrollTop, height };

export interface CountryPickerRead { tag: string; branchId: string; hour: number }

function flagsRequest(context: { sourceId: string; revision: string; branchId: string; hour: number }, size: CountryFlagSize, tags: string[]): ProductRequest<CountryFlagParams> {
  return { sourceId: context.sourceId, revision: context.revision, product: 'country-flags', params: { branchId: context.branchId, hour: context.hour, tags, size } };
}

/** Flags are always the game's own files from the recording's content stack. The
 * header shows the country as the game's top bar does (`shield`) at `flagAt`, the
 * playback day; lists use the same styled flags at each country's observed hour.
 * The list renders only the rows in view and loads only their flags, so it costs
 * the same at any country count. */
export default function CountryPicker({ countries, selectedTag, playerCountryId, onChange, onCountries, onReconnect, source, client, field = false, inline = false, flagAt = null, addLabel, chosen, allowedTags, choiceReads }: {
  countries: CountryMetadata[]; selectedTag: string | null; playerCountryId: string | null;
  onChange: (tag: string | null) => void; source: DashboardSource | null | undefined; client: DashboardClient;
  onCountries?: (sourceId: string, revision: string, countries: CountryMetadata[]) => void;
  onReconnect?: () => void; field?: boolean;
  /** The search and list alone, shown in place (inside another popover). */
  inline?: boolean; flagAt?: { branchId: string; hour: number } | null;
  /** A multi-country field: the trigger reads this label and every `chosen` tag is ticked. */
  addLabel?: string; chosen?: readonly string[];
  /** A drill chooser can restrict its list to the countries being compared. */
  allowedTags?: readonly string[];
  /** Immutable, authoritative supplying reads, keyed by the pickable tag. A
   * missing/null read refuses looks; read branches must match flagAt.branchId.
   * Clicks still return the pickable tag, never the supplying read's alias. */
  choiceReads?: ReadonlyMap<string, CountryPickerRead | null>;
}) {
  const id = useId();
  const listId = `${id}-countries`;
  const optionId = (index: number) => `${id}-country-${index}`;
  const menu = usePopover(field ? 'end' : 'start');
  const { trigger } = menu;
  const open = inline || menu.open;
  const list = useRef<HTMLDivElement>(null);
  const listbox = useRef<HTMLDivElement>(null);
  const [query, setQuery] = useState('');
  const [active, setActive] = useState(-1);
  const [view, setView] = useState({ scrollTop: 0, height: LIST_HEIGHT });
  const [retry, setRetry] = useState(0);
  const content = useContentRevision(client);
  const sourceKey = source ? JSON.stringify([source.id, source.revision, content]) : '';
  const nameContext = useMemo(() => ({ sourceKey, client, branchId: flagAt?.branchId, hour: flagAt?.hour, choiceReads }),
    [sourceKey, client, flagAt?.branchId, flagAt?.hour, choiceReads]);
  const [capturedNames, setCapturedNames] = useState<{ context: typeof nameContext | null; names: ReadonlyMap<string, string> }>({ context: null, names: NO_NAMES });
  const heldNames = capturedNames.context === nameContext ? capturedNames.names : NO_NAMES;
  const [roster, setRoster] = useState<{ key: string; countries: CountryMetadata[]; error: string | null }>({ key: '', countries: [], error: null });
  const loading = !!source && !allowedTags && roster.key !== sourceKey;
  const choices = useMemo(() => {
    const choices = allowedTags ? allowedCountryChoices(countries, allowedTags, flagAt, heldNames)
      : roster.key === sourceKey ? countryChoices(roster.countries) : [];
    return choiceReads === undefined ? choices : choices.map(({ country }) => {
      const name = heldNames.get(country.tag) ?? 'Country name unavailable';
      return { country: { ...country, name }, search: `${name} ${country.tag}`.toLowerCase() };
    });
  }, [countries, roster, sourceKey, allowedTags, flagAt?.branchId, flagAt?.hour, heldNames, choiceReads]); // eslint-disable-line react-hooks/exhaustive-deps
  const byTag = useMemo(() => new Map(choices.map(({ country }) => [country.tag, country])), [choices]);
  const selected = useMemo(() => selectedTag == null ? countries.find((country) => country.id === playerCountryId)
    : byTag.get(selectedTag) ?? countries.find((country) => country.tag === selectedTag), [countries, playerCountryId, selectedTag, byTag]);
  const filtered = useMemo(() => filterChoices(choices, query), [choices, query]);
  const range = visibleRange(filtered.length, view.scrollTop, view.height);
  const visible = open ? filtered.slice(range.start, range.end) : [];
  const choiceTrack = useMemo(() => {
    const branch = source?.metadata.branches.find(branch => branch.id === flagAt?.branchId);
    return source && branch ? { source, branch } : undefined;
  }, [source, flagAt?.branchId]);
  const choiceRead = (tag: string): CountryPickerRead | null => {
    if (choiceReads === undefined) return flagAt ? { tag, ...flagAt } : null;
    const read = choiceReads.get(tag);
    return read && choiceTrack && read.branchId === choiceTrack.branch.id && Number.isSafeInteger(read.hour)
      && read.hour >= choiceTrack.branch.firstHour && read.hour <= choiceTrack.branch.lastHour ? read : null;
  };
  const wantedChoices = allowedTags || choiceReads !== undefined ? [...new Set([...visible.map(choice => choice.country.tag), ...(selected ? [selected.tag] : [])])]
    .flatMap(displayTag => { const read = choiceRead(displayTag); return read ? [{ displayTag, ...read }] : []; }) : [];
  const wantedNames = wantedChoices.map(({ tag, hour }) => ({ tag, hour }));
  const names = useCountryNamesAt(client, choiceTrack, wantedNames, (!!allowedTags || choiceReads !== undefined) && (!!flagAt || choiceReads !== undefined));
  // Keep names already seen at this held hour searchable after filtering or
  // scrolling releases their visible-row requests. Another context starts empty.
  useEffect(() => {
    if ((!allowedTags || !flagAt) && choiceReads === undefined) return;
    setCapturedNames(previous => {
      const held = previous.context === nameContext ? previous.names : NO_NAMES;
      const added = wantedChoices.flatMap(({ displayTag, tag, hour }) => {
        const name = names.get(nameKey(hour, tag));
        return name !== undefined && held.get(displayTag) !== name ? [[displayTag, name] as const] : [];
      });
      return previous.context === nameContext && !added.length ? previous : { context: nameContext, names: new Map([...held, ...added]) };
    });
    // `names` changes for the exact visible target set returned by the hook.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [names, nameContext, allowedTags]);
  const countryName = (country: CountryOption) => choiceReads !== undefined || allowedTags && flagAt
    ? heldNames.get(country.tag) ?? 'Country name unavailable' : country.name;
  const [flagState, setFlagState] = useState<{ context: typeof nameContext | null; loads: number; error: string | null }>({ context: null, loads: 0, error: null });
  const [headerFlag, setHeaderFlag] = useState<{ client: DashboardClient | null; identity: string; key: string; image?: CountryFlagImage; error: string | null }>({ client: null, identity: '', key: '', error: null });
  const displayHeaderTag = addLabel ? null : selected?.tag ?? selectedTag;
  const headerRead = displayHeaderTag ? choiceRead(displayHeaderTag) : null;
  const headerTag = choiceReads === undefined ? displayHeaderTag : headerRead?.tag ?? null;
  const headerSize: CountryFlagSize = 'shield';
  const flags = flagStore(client);
  const countryAt = (country: CountryOption) => choiceReads !== undefined ? choiceRead(country.tag)
    : allowedTags && flagAt ? { tag: country.tag, ...flagAt }
    : country.observedBranchId != null && country.observedHour != null ? { tag: country.tag, branchId: country.observedBranchId, hour: country.observedHour } : null;
  const flagOf = (country: CountryOption) => {
    const at = countryAt(country);
    return source && at ? flags.get(FlagStore.key(source.id, source.revision, at.branchId, at.hour, at.tag)) ?? undefined : undefined;
  };
  const flagContext = useMemo(() => {
    if (!source?.metadata.branches.length) return null;
    const witness = selected ?? source.metadata.countries.find((country) => country.id === source.metadata.playerCountryId) ?? source.metadata.countries[0];
    const branch = source.metadata.branches.find((branch) => branch.id === witness?.observedBranchId) ?? source.metadata.branches[0];
    return { sourceId: source.id, revision: source.revision, branchId: branch.id,
      hour: witness?.observedBranchId === branch.id ? witness.observedHour ?? branch.firstHour : branch.firstHour };
  }, [source, selected]);
  const live = (choiceReads !== undefined || (!field || !!allowedTags) && !!flagAt) && !!source;
  const headerContext = useMemo(() => choiceReads !== undefined ? source && headerRead
    ? { sourceId: source.id, revision: source.revision, branchId: headerRead.branchId, hour: headerRead.hour } : null
    : live ? { sourceId: source!.id, revision: source!.revision, branchId: flagAt!.branchId, hour: flagAt!.hour } : flagContext,
    [choiceReads !== undefined, source, headerRead?.branchId, headerRead?.hour, live, flagAt?.branchId, flagAt?.hour, flagContext]);
  const headerIdentity = choiceReads !== undefined ? JSON.stringify([sourceKey, displayHeaderTag, headerTag, headerSize, headerContext?.branchId, headerContext?.hour])
    : `${sourceKey}:${headerTag}:${headerSize}`;
  const headerKey = `${headerIdentity}:${headerContext?.branchId}:${headerContext?.hour}`;
  // A new playback day keeps the current flag on screen until its own flag arrives.
  const headerImage = headerFlag.client === client && headerFlag.identity === headerIdentity ? headerFlag.image : undefined;
  const flagError = [flagState.context === nameContext ? flagState.error : null, headerFlag.client === client && headerFlag.key === headerKey ? headerFlag.error : null].filter(Boolean).join('\n') || null;
  useEffect(() => {
    if (!headerContext || !headerTag) return;
    const controller = new AbortController();
    const request = flagsRequest(headerContext, headerSize, [headerTag]);
    // Per-day requests bypass the shared cache so they cannot evict full-branch products.
    void (live ? loadTransientProduct<CountryFlagReply>(client, request, controller.signal) : loadProduct<CountryFlagReply>(client, request, controller.signal)).then(result => {
      if (!controller.signal.aborted) setHeaderFlag({ client, identity: headerIdentity, key: headerKey, image: result.flags.find(image => image.tag === headerTag), error: null });
    }).catch(error => {
      // An unreadable hour shows no flag rather than carrying one across it.
      if (!controller.signal.aborted) setHeaderFlag({ client, identity: headerIdentity, key: headerKey, image: undefined, error: String(error) });
    });
    return () => controller.abort();
  }, [headerContext, client, headerTag, headerKey, headerIdentity, headerSize, live, retry]);
  useEffect(() => {
    if (!source || allowedTags) return;
    const controller = new AbortController();
    void loadProduct<LandOwners>(client, { sourceId: source.id, revision: source.revision, product: 'land-owners', params: {} }, controller.signal).then(result => {
      if (!controller.signal.aborted) {
        setRoster({ key: sourceKey, countries: result.countries, error: null });
        onCountries?.(source.id, source.revision, result.countries);
      }
    }).catch(error => {
      if (!controller.signal.aborted) setRoster(previous => ({ key: sourceKey, countries: previous.key === sourceKey ? previous.countries : [], error: String(error) }));
    });
    return () => controller.abort();
  }, [sourceKey, client, onCountries, retry, allowedTags]);
  // Only the rows in view ask for flags; stored flags are never requested again.
  // JSON escapes line breaks, so one line per row keeps the key exact.
  const visibleTargets = visible.flatMap(({ country }) => {
    const at = countryAt(country);
    return at ? [JSON.stringify([at.branchId, at.hour, at.tag])] : [];
  }).join('\n');
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
      if (!controller.signal.aborted) setFlagState(previous => ({ context: nameContext, loads: (previous.context === nameContext ? previous.loads : 0) + 1, error: null }));
    }).catch(error => {
      if (!controller.signal.aborted) setFlagState(previous => ({ context: nameContext, loads: previous.context === nameContext ? previous.loads : 0, error: String(error) }));
    });
    return () => controller.abort();
  }, [source, sourceKey, client, visibleTargets, retry, nameContext]);
  useLayoutEffect(() => {
    if (open && list.current) setView(listView(list.current.scrollTop, list.current.clientHeight || LIST_HEIGHT));
  }, [open]);
  const scrollTo = (scrollTop: number) => {
    if (list.current) list.current.scrollTop = scrollTop;
    setView(previous => listView(scrollTop, previous.height)(previous));
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
  const choose = (tag: string | null) => { onChange(tag); if (!inline) menu.close(); };
  const body = <>
        <div className="country-picker-search"><Icon name="search" /><input data-popover-autofocus aria-label="Search countries" placeholder="Search countries or tags" aria-controls={listId} value={query}
          onChange={(event) => { setQuery(event.currentTarget.value); reset(); }}
          onKeyDown={(event) => {
            if (event.key === 'ArrowDown' && filtered.length) { event.preventDefault(); listbox.current?.focus({ preventScroll: true }); moveTo(0); }
          }} /></div>
        <div ref={list} className="country-picker-list" onScroll={(event) => {
          const { scrollTop, clientHeight } = event.currentTarget;
          setView(listView(scrollTop, clientHeight || LIST_HEIGHT));
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
                <CountryFlag image={flagOf(country)} /><span>{countryName(country)}</span><small>{country.tag}</small>{ticked && <Icon name="check" className="country-selected-mark" />}
              </div>;
            })}
          </div>
          {loading && <p className="country-picker-empty" role="status"><span className="spinner" />Loading countries…</p>}
          {!allowedTags && !loading && roster.error && <div className="country-picker-note"><Icon name="alert" /><span>Couldn’t load countries.</span><button type="button" className="button small quiet" onClick={retryCountries}>Retry</button></div>}
          {!loading && (allowedTags || !roster.error) && !filtered.length && <p className="country-picker-empty">{query ? 'No matching countries' : 'No countries available'}</p>}
        </div>
  </>;
  if (inline) return <div className="country-picker country-picker-inline">{body}</div>;
  return <div className={field ? 'field widget-country-field' : 'header-control header-country'}>
    <span className={field ? undefined : 'sr-only'}>Country</span>
    <button ref={trigger} type="button" className={field ? 'widget-country-trigger' : 'header-country-trigger'} disabled={!source} popoverTarget={id}
      aria-label={addLabel ?? (field ? 'Widget country' : 'Choose country')} aria-haspopup="dialog" aria-expanded={open}>
      {addLabel ? <Icon name="plus" /> : <CountryFlag image={headerImage} />}
      {addLabel ? <span>{addLabel}</span> : field ? <span>{(selected ? countryName(selected) : selectedTag) ?? 'Choose country'}</span>
        : <span className="header-country-text"><strong>{(selected ? countryName(selected) : selectedTag) ?? (source ? 'Choose country' : 'No campaign')}</strong></span>}
      <Icon name="chevron-down" />
    </button>
    <Popover menu={menu} portal={!field} id={id} aria-label="Choose country" className="country-picker"
      onToggle={(event) => { if (event.newState === 'closed') { setQuery(''); reset(); } }}>
      {open && body}
    </Popover>
  </div>;
}
