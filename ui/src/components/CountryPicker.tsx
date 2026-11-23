import { ABSENT_NAME, nameReadFailure, nameReadMetadata, type NameRead } from '../dashboard/nameReads.ts';
import { type Failure, asFailure, failureText, failureDetails } from '../dashboard/queryError.ts';
import { Popover, usePopover } from './Popover';
import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { CountryMetadata } from '../shell/api';
import { Icon } from './Icons';
import { CountryFlag } from './CountryFlag';
import { FailureLine } from './FailureLine';
import { loadProduct } from '../dashboard/products';
import { isFailedFact } from '../dashboard/countryFacts.ts';
import { flagReadKey, retryFlags, useFlagReads } from '../dashboard/countryFlags';
import { useContentRevision } from '../dashboard/contentRevision';
import { nameKey, useCountryNamesAt } from '../dashboard/countryNamesAt.ts';
import type { DashboardClient, DashboardSource } from '../dashboard/client';
import type { CountryFlagImage, CountryFlagSize, LandOwners } from '../generated/dashboard';
import { COUNTRY_ROW_HEIGHT, allowedCountryChoices, countryChoices, filterChoices, moveIndex, nameAskHour, revealRow, visibleRange } from './countryList';
import type { CountryOption } from './countryList';
import { productRequest } from '../dashboard/request.ts';

const LIST_HEIGHT = 340;
const NO_NAMES: ReadonlyMap<string, NameRead> = new Map();
const NO_NAME_READS: readonly { tag: string; hour: number }[] = [];
const listView = (scrollTop: number, height: number) => (before: { scrollTop: number; height: number }) =>
  before.scrollTop === scrollTop && before.height === height ? before : { scrollTop, height };

export interface CountryPickerRead { tag: string; branchId: string; hour: number }

export default function CountryPicker({ countries, selectedTag, playerCountryId, onChange, onCountries, onReconnect, source, client, field = false, inline = false, flagAt = null, addLabel, chosen, allowedTags, choiceReads, disabled = false }: {
  countries: CountryMetadata[]; selectedTag: string | null; playerCountryId: string | null;
  onChange: (tag: string | null, slot: number | null) => void; source: DashboardSource | null | undefined; client: DashboardClient;
  onCountries?: (sourceId: string, revision: string, countries: CountryMetadata[]) => void;
  onReconnect?: () => void; field?: boolean;
  disabled?: boolean;
  inline?: boolean; flagAt?: { branchId: string; hour: number } | null;
  addLabel?: string; chosen?: readonly string[];
  allowedTags?: readonly string[];
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
  const [capturedNames, setCapturedNames] = useState<{ context: typeof nameContext | null; names: ReadonlyMap<string, NameRead> }>({ context: null, names: NO_NAMES });
  const heldNames = capturedNames.context === nameContext ? capturedNames.names : NO_NAMES;
  const [roster, setRoster] = useState<{ key: string; countries: CountryMetadata[]; error: Failure | null }>({ key: '', countries: [], error: null });
  const loading = !!source && !allowedTags && roster.key !== sourceKey;
  const choiceTrack = useMemo(() => {
    const branch = source?.metadata.branches.find(branch => branch.id === flagAt?.branchId);
    return source && branch ? { source, branch } : undefined;
  }, [source, flagAt?.branchId]);
  const choiceRead = (tag: string): CountryPickerRead | null => {
    const read = choiceReads === undefined ? flagAt ? { tag, ...flagAt } : null : choiceReads.get(tag);
    return read && choiceTrack && read.branchId === choiceTrack.branch.id && Number.isSafeInteger(read.hour)
      && read.hour >= choiceTrack.branch.firstHour && read.hour <= choiceTrack.branch.lastHour ? read : null;
  };
  const choices = useMemo(() => {
    const choices = allowedTags ? allowedCountryChoices(countries, allowedTags, flagAt, heldNames)
      : roster.key === sourceKey ? countryChoices(roster.countries) : [];
    return choiceReads === undefined ? choices : choices.map(({ country }) => {
      const name = nameReadMetadata(heldNames.get(country.tag)).name;
      return { country: { ...country, name }, search: `${name} ${country.tag}`.toLowerCase() };
    });
  }, [countries, roster, sourceKey, allowedTags, flagAt?.branchId, flagAt?.hour, heldNames, choiceReads, choiceTrack]);
  const byTag = useMemo(() => new Map(choices.map(({ country }) => [country.tag, country])), [choices]);
  const selected = useMemo(() => selectedTag == null ? countries.find((country) => country.id === playerCountryId)
    : byTag.get(selectedTag) ?? countries.find((country) => country.tag === selectedTag), [countries, playerCountryId, selectedTag, byTag]);
  const searching = !!query.trim();
  const unfiltered = visibleRange(choices.length, view.scrollTop, view.height);
  const named = !open ? [] : searching ? choices : choices.slice(unfiltered.start, unfiltered.end);
  const datedRoster = !allowedTags && choiceReads === undefined && !!flagAt;
  const wantedChoices = allowedTags || choiceReads !== undefined || datedRoster ? [...new Set([...named.map(choice => choice.country.tag), ...(selected ? [selected.tag] : [])])]
    .flatMap(displayTag => { const read = choiceRead(displayTag); return read ? [{ displayTag, ...read }] : []; }) : [];
  const wantedNames = wantedChoices.map(({ tag, hour }) => ({ tag, hour }));
  const names = useCountryNamesAt(client, choiceTrack, wantedNames, (!!allowedTags || choiceReads !== undefined || datedRoster) && (!!flagAt || choiceReads !== undefined),
    new Set(wantedNames.map(read => read.hour)).size);
  useEffect(() => {
    if ((!allowedTags || !flagAt) && choiceReads === undefined && !datedRoster) return;
    setCapturedNames(previous => {
      const held = previous.context === nameContext ? previous.names : NO_NAMES;
      const added = wantedChoices.flatMap(({ displayTag, tag, hour }) => {
        const name = names.get(nameKey(hour, tag));
        return name !== undefined && name.kind !== 'loading' && held.get(displayTag) !== name ? [[displayTag, name] as const] : [];
      });
      return previous.context === nameContext && !added.length ? previous : { context: nameContext, names: new Map([...held, ...added]) };
    });
  }, [names, nameContext, allowedTags]);
  const datedName = (country: CountryOption): NameRead | undefined => {
    const read = choiceRead(country.tag);
    const dated = read ? names.get(nameKey(read.hour, read.tag)) ?? heldNames.get(country.tag) : undefined;
    return dated?.kind === 'value' ? dated : undefined;
  };
  const countryNameRead = (country: CountryOption): NameRead => {
    if (datedRoster) return datedName(country) ?? { kind: 'value', value: country.name };
    if (choiceReads === undefined && !(allowedTags && flagAt)) return { kind: 'value', value: country.name };
    const read = choiceRead(country.tag);
    if (!read) return ABSENT_NAME;
    const current = names.get(nameKey(read.hour, read.tag)), held = heldNames.get(country.tag);
    return current?.kind === 'loading' ? held ?? current : current ?? held ?? ABSENT_NAME;
  };
  const countryName = (country: CountryOption) => nameReadMetadata(countryNameRead(country)).name;
  const filtered = filterChoices(choices, query, choice => countryName(choice.country));
  const range = visibleRange(filtered.length, view.scrollTop, view.height);
  const visible = open ? filtered.slice(range.start, range.end) : [];
  const sole = searching && filtered.length === 1 ? 0 : -1;
  const shownActive = active >= 0 ? active : sole;
  const displayHeaderTag = addLabel ? null : selected?.tag ?? selectedTag;
  const headerRead = displayHeaderTag ? choiceRead(displayHeaderTag) : null;
  const headerTag = choiceReads === undefined ? displayHeaderTag : headerRead?.tag ?? null;
  const headerSize: CountryFlagSize = field ? 'panel' : 'shield';
  const countryAt = (country: CountryOption) => choiceReads !== undefined ? choiceRead(country.tag)
    : allowedTags && flagAt ? choiceRead(country.tag)
    : datedRoster && datedName(country) ? choiceRead(country.tag)
    : country.observedBranchId != null && country.observedHour != null ? { tag: country.tag, branchId: country.observedBranchId, hour: country.observedHour } : null;
  const listReads = source ? visible.flatMap(({ country }) => { const at = countryAt(country); return at ? [at] : []; }) : [];
  const listFlags = useFlagReads(client, source ?? undefined, listReads, true, 'panel');
  const flagOf = (country: CountryOption) => {
    const at = countryAt(country);
    const flag = at ? listFlags.get(flagReadKey(at)) : undefined;
    return flag && !isFailedFact(flag) ? flag.image : undefined;
  };
  const flagContext = useMemo(() => {
    if (!source?.metadata.branches.length) return null;
    const witness = selected ?? source.metadata.countries.find((country) => country.id === source.metadata.playerCountryId) ?? source.metadata.countries[0];
    const branch = source.metadata.branches.find((branch) => branch.id === witness?.observedBranchId) ?? source.metadata.branches[0];
    return { sourceId: source.id, revision: source.revision, branchId: branch.id,
      hour: witness?.observedBranchId === branch.id ? witness.observedHour ?? branch.firstHour : branch.firstHour };
  }, [source?.id, source?.revision, source?.metadata.branches, selected]);
  const live = (choiceReads !== undefined || (!field || !!allowedTags) && !!flagAt) && !!source;
  const headerContext = useMemo(() => choiceReads !== undefined ? source && headerRead
    ? { sourceId: source.id, revision: source.revision, branchId: headerRead.branchId, hour: headerRead.hour } : null
    : live ? source && headerRead ? { sourceId: source.id, revision: source.revision, branchId: headerRead.branchId, hour: headerRead.hour } : null : flagContext,
    [choiceReads !== undefined, source?.id, source?.revision, headerRead?.branchId, headerRead?.hour, live, flagAt?.branchId, flagAt?.hour, flagContext]);
  const headerIdentity = choiceReads !== undefined ? JSON.stringify([sourceKey, displayHeaderTag, headerTag, headerSize, headerContext?.branchId, headerContext?.hour])
    : JSON.stringify([sourceKey, headerTag, headerSize, choiceTrack?.branch.id]);
  const flagOwner = `${headerIdentity}:${headerContext?.branchId}`;
  const wantedNameHour = live && choiceReads === undefined && !allowedTags && headerTag ? flagAt!.hour : null;
  const [askedName, setAskedName] = useState<{ identity: string; hour: number } | null>(null);
  const askedHour = wantedNameHour === null ? null : askedName?.identity === headerIdentity ? askedName.hour : wantedNameHour;
  const headerNameAt = useMemo(() => askedHour !== null && headerTag ? [{ tag: headerTag, hour: askedHour }] : NO_NAME_READS, [askedHour, headerTag]);
  const headerNameReads = useCountryNamesAt(client, choiceTrack, headerNameAt, headerNameAt.length > 0);
  const headerNameRead = headerNameAt.length ? headerNameReads.get(nameKey(headerNameAt[0].hour, headerNameAt[0].tag)) : undefined;
  const [headerName, setHeaderName] = useState<{ identity: string; name: NameRead } | null>(null);
  useEffect(() => {
    if (headerNameRead !== undefined && headerNameRead.kind !== 'loading') setHeaderName({ identity: headerIdentity, name: headerNameRead });
    const next = nameAskHour(askedName?.identity === headerIdentity ? askedName.hour : null, wantedNameHour, headerNameRead !== undefined && headerNameRead.kind !== 'loading');
    if (next !== null && (askedName?.identity !== headerIdentity || askedName.hour !== next)) setAskedName({ identity: headerIdentity, hour: next });
  }, [headerNameRead, headerIdentity, wantedNameHour, askedName]);
  const datedHeaderName = !headerNameAt.length ? undefined
    : nameReadMetadata(headerName?.identity === headerIdentity ? headerName.name : headerNameRead).name;
  const headerFlagAt = headerContext && headerTag ? { tag: headerTag, branchId: headerContext.branchId, hour: headerContext.hour } : null;
  const headerFlags = useFlagReads(client, source ?? undefined, headerFlagAt ? [headerFlagAt] : [], !!headerFlagAt, headerSize);
  const headerFlag = headerFlagAt ? headerFlags.get(flagReadKey(headerFlagAt)) : undefined;
  const shownHeader = useRef<{ owner: string; client: DashboardClient | null; image?: CountryFlagImage }>({ owner: '', client: null });
  if (headerFlag !== undefined || shownHeader.current.owner !== flagOwner || shownHeader.current.client !== client)
    shownHeader.current = { owner: flagOwner, client, image: headerFlag && !isFailedFact(headerFlag) ? headerFlag.image : undefined };
  const headerImage = shownHeader.current.image;
  const flagFailure = [headerFlag, ...listFlags.values()].find((flag): flag is Error => flag instanceof Error);
  const flagFailed = !!flagFailure;
  useEffect(() => {
    if (!source || allowedTags) return;
    const controller = new AbortController();
    void loadProduct<LandOwners>(client, productRequest({ sourceId: source.id, revision: source.revision, product: 'land-owners', params: {} }), controller.signal, undefined, source).then(result => {
      if (!controller.signal.aborted) {
        setRoster({ key: sourceKey, countries: result.countries, error: null });
        onCountries?.(source.id, source.revision, result.countries);
      }
    }).catch(error => {
      if (!controller.signal.aborted) setRoster(previous => ({ key: sourceKey, countries: previous.key === sourceKey ? previous.countries : [], error: asFailure(error) }));
    });
    return () => controller.abort();
  }, [sourceKey, client, onCountries, retry, allowedTags]);
  const revealSelected = useRef(false);
  useLayoutEffect(() => {
    revealSelected.current = open && !chosen;
    if (open && list.current) setView(listView(list.current.scrollTop, list.current.clientHeight || LIST_HEIGHT));
  }, [open]);
  const scrollTo = (scrollTop: number) => {
    if (list.current) list.current.scrollTop = scrollTop;
    setView(previous => listView(scrollTop, previous.height)(previous));
  };
  const reset = () => { setActive(-1); scrollTo(0); };
  useLayoutEffect(() => {
    if (!revealSelected.current || query) return;
    const at = selected ? filtered.findIndex(({ country }) => country.tag === selected.tag) : -1;
    if (at < 0) return;
    revealSelected.current = false;
    const height = list.current?.clientHeight || LIST_HEIGHT;
    scrollTo(Math.max(0, at * COUNTRY_ROW_HEIGHT - (height - COUNTRY_ROW_HEIGHT) / 2));
    setActive(at);
  }, [open, filtered, selected?.tag, query]);
  const moveTo = (index: number) => {
    setActive(index);
    scrollTo(revealRow(index, list.current?.scrollTop ?? view.scrollTop, list.current?.clientHeight || view.height));
  };
  const retryCountries = () => {
    if (onReconnect && roster.error?.message.includes('source is unavailable or changed')) onReconnect();
    else { if (source && flagFailed) retryFlags(client, source.id); setRetry(value => value + 1); }
  };
  const choose = (country: CountryOption | null) => {
    const slot = country?.id === undefined ? NaN : Number(country.id);
    onChange(country?.tag ?? null, Number.isSafeInteger(slot) ? slot : null);
    if (!inline) menu.close();
  };
  const body = <>
        <div className="country-picker-search"><Icon name="search" /><input type="search" data-popover-autofocus aria-label="Search countries" placeholder="Search countries or tags" aria-controls={listId} value={query}
          aria-activedescendant={sole >= 0 ? optionId(sole) : undefined}
          onChange={(event) => { setQuery(event.currentTarget.value); reset(); }}
          onKeyDown={(event) => {
            if (event.key === 'ArrowDown' && filtered.length) { event.preventDefault(); listbox.current?.focus({ preventScroll: true }); moveTo(0); }
            else if (event.key === 'Enter' && sole >= 0) { event.preventDefault(); choose(filtered[sole].country); }
          }} /></div>
        <div ref={list} className="country-picker-list" onScroll={(event) => {
          const { scrollTop, clientHeight } = event.currentTarget;
          setView(listView(scrollTop, clientHeight || LIST_HEIGHT));
        }}>
          <div ref={listbox} id={listId} role="listbox" aria-label="Countries" data-tooltip-side="right" aria-multiselectable={chosen !== undefined} tabIndex={filtered.length ? 0 : -1} className="country-picker-options"
            aria-activedescendant={active >= range.start && active < range.end ? optionId(active) : undefined}
            style={{ height: filtered.length * COUNTRY_ROW_HEIGHT }}
            onFocus={(event) => { if (event.target === event.currentTarget && active < 0 && filtered.length) setActive(Math.min(filtered.length - 1, Math.ceil(view.scrollTop / COUNTRY_ROW_HEIGHT))); }}
            onKeyDown={(event) => {
              if ((event.key === 'Enter' || event.key === ' ') && active >= 0 && active < filtered.length) {
                event.preventDefault(); choose(filtered[active].country); return;
              }
              const next = moveIndex(event.key, active, filtered.length);
              if (next == null) return;
              event.preventDefault();
              moveTo(next);
            }}>
            {visible.map(({ country }, offset) => {
              const index = range.start + offset;
              const ticked = chosen ? chosen.includes(country.tag) : selected?.tag === country.tag;
              const name = countryName(country).trim();
              return <div key={country.tag} id={optionId(index)} role="option" aria-selected={ticked} aria-setsize={filtered.length} aria-posinset={index + 1}
                data-detail={nameReadFailure(countryNameRead(country))?.message} data-tooltip-overflow={name || undefined} className="country-picker-option" data-active={index === shownActive || undefined} data-sole={index === sole && active < 0 || undefined} style={{ top: index * COUNTRY_ROW_HEIGHT }}
                onClick={() => choose(country)}>
                <CountryFlag image={flagOf(country)} tag={country.tag} decorative />
                {name ? <span data-overflow-label>{name}</span> : <span className="country-picker-unnamed">{country.tag}<small>name not recorded</small></span>}
                {name && <small>{country.tag}</small>}
                {ticked && <Icon name="check" className="country-selected-mark" />}
              </div>;
            })}
          </div>
          {loading && <p className="country-picker-empty" role="status"><span className="spinner" />Loading countries…</p>}
          {!allowedTags && !loading && roster.error ? <FailureLine className="country-picker-note" detail={roster.error.message} message={failureText(roster.error, 'Couldn’t load countries.')} onRetry={retryCountries} />
            : flagFailed && <FailureLine className="country-picker-note" detail={flagFailure?.message} message={failureText(flagFailure, 'Couldn’t load flags.')} onRetry={retryCountries} />}
          {!loading && (allowedTags || !roster.error) && !filtered.length && <p className="country-picker-empty">{query ? 'No matching countries.' : 'No countries available.'}</p>}
        </div>
  </>;
  if (inline) return <div className="country-picker country-picker-inline">{body}</div>;
  const triggerDisabled = !source || disabled;
  const shownTag = selected?.tag ?? selectedTag;
  const shownName = (field ? selected ? countryName(selected) : '' : datedHeaderName ?? (selected ? countryName(selected) : '')).trim();
  const flagPending = !!headerFlagAt && headerFlag === undefined && !headerImage;
  return <div data-detail={failureDetails([...heldNames.values(), headerNameRead, headerName?.name].map(nameReadFailure))} className={field ? 'field widget-country-field' : 'header-control header-country'}>
    <span className={field ? undefined : 'sr-only'}>Country</span>
    <button ref={trigger} type="button" className={field ? 'widget-country-trigger' : 'header-country-trigger'} disabled={triggerDisabled} popoverTarget={id}
      aria-label={addLabel ?? `${field ? 'Widget country' : 'Choose country'}${shownName || shownTag ? `, currently ${shownName || shownTag}` : ''}`} aria-haspopup="dialog" aria-expanded={open}
      aria-description={!addLabel && (field || source) && !headerImage ? `Flag ${flagPending ? 'loading' : 'unavailable'}` : undefined}
      data-tooltip-overflow={!addLabel && shownName ? shownName : undefined}>
      {addLabel ? <Icon name="plus" /> : (field || source) && <CountryFlag image={headerImage} tag={shownTag} name={shownName || null}
        pending={flagPending} decorative />}
      {addLabel ? <span>{addLabel}</span> : field ? <span data-overflow-label>{shownName || shownTag || 'Choose country'}</span>
        : <span className="header-country-text"><strong data-overflow-label>{shownName || shownTag || (source ? 'Choose country' : 'No campaign')}</strong>
          {!shownName && shownTag && <small>name not recorded</small>}</span>}
      {!triggerDisabled && <Icon name="chevron-down" />}
    </button>
    <Popover menu={menu} portal={!field} id={id} aria-label="Choose country" className="country-picker"
      onToggle={(event) => { if (event.newState === 'closed') { setQuery(''); reset(); } }}>
      {open && body}
    </Popover>
  </div>;
}
