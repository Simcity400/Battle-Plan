import { ABSENT_NAME, nameReadFailure, nameReadMetadata, type NameRead } from '../dashboard/nameReads.ts';
import { type Failure, asFailure, failureText, failureDetails } from '../dashboard/queryError.ts';
import { Popover, usePopover } from './Popover';
import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { CountryMetadata } from '../shell/api';
import { Icon } from './Icons';
import { CountryFlag } from './CountryFlag';
import { FailureLine } from './FailureLine';
import { loadProduct } from '../dashboard/products';
import { useContentRevision } from '../dashboard/contentRevision';
import type { DashboardClient, DashboardSource } from '../dashboard/client';
import type { CountryFlagImage, CountryFlagSize, LandOwners } from '../generated/dashboard';
import { COUNTRY_ROW_HEIGHT, allowedCountryChoices, countryChoices, filterChoices, moveIndex, revealRow, visibleRange } from './countryList';
import type { CountryOption } from './countryList';
import { productRequest } from '../dashboard/request.ts';
import { identityBrokerFor } from '../families/identity/broker.ts';
import {
  countryTagKey,
  useIdentities,
  useIdentity,
  useIdentityScope,
  type CountryIdentityResult,
  type IdentityAsk,
} from '../families/identity/react.ts';

const LIST_HEIGHT = 340;
const NO_ASKS: readonly IdentityAsk[] = [];
const listView = (scrollTop: number, height: number) => (before: { scrollTop: number; height: number }) =>
  before.scrollTop === scrollTop && before.height === height ? before : { scrollTop, height };

export interface CountryPickerRead { tag: string; branchId: string; hour: number }

function nameReadFromIdentity(result: CountryIdentityResult | null | undefined): NameRead {
  if (!result || result.kind === 'pending') return { kind: 'loading' };
  if (result.kind === 'absent') return ABSENT_NAME;
  if (result.kind === 'failed') return { kind: 'failed', failure: result.failure };
  if (result.name.kind === 'known') return { kind: 'value', value: result.name.value };
  if (result.name.kind === 'pending') return { kind: 'loading' };
  if (result.name.kind === 'missing') return ABSENT_NAME;
  return { kind: 'failed', failure: result.name.failure };
}

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
  const [roster, setRoster] = useState<{ key: string; countries: CountryMetadata[]; error: Failure | null }>({ key: '', countries: [], error: null });
  const loading = !!source && !allowedTags && roster.key !== sourceKey;
  const datedMode = choiceReads !== undefined || !!flagAt;
  const choiceTrack = useMemo(() => {
    const branch = source?.metadata.branches.find(branch => branch.id === flagAt?.branchId);
    return source && branch ? { source, branch } : undefined;
  }, [source, flagAt?.branchId]);
  const choiceRead = (tag: string): CountryPickerRead | null => {
    const read = choiceReads === undefined ? (flagAt ? { tag, ...flagAt } : null) : choiceReads.get(tag);
    return read && choiceTrack && read.branchId === choiceTrack.branch.id && Number.isSafeInteger(read.hour)
      && read.hour >= choiceTrack.branch.firstHour && read.hour <= choiceTrack.branch.lastHour ? read : null;
  };
  const choices = useMemo(() => {
    const base = allowedTags
      ? allowedCountryChoices(countries, allowedTags, null)
      : roster.key === sourceKey ? countryChoices(roster.countries) : [];
    if (!datedMode) return base;
    return base.map(({ country }) => ({ country, search: country.tag.toLowerCase() }));
  }, [countries, roster, sourceKey, allowedTags, datedMode]);
  const byTag = useMemo(() => new Map(choices.map(({ country }) => [country.tag, country])), [choices]);
  const selected = useMemo(() => selectedTag == null ? countries.find((country) => country.id === playerCountryId)
    : byTag.get(selectedTag) ?? countries.find((country) => country.tag === selectedTag), [countries, playerCountryId, selectedTag, byTag]);
  const searching = !!query.trim();
  const unfiltered = visibleRange(choices.length, view.scrollTop, view.height);
  const named = !open ? [] : searching ? choices : choices.slice(unfiltered.start, unfiltered.end);
  const displayHeaderTag = addLabel ? null : selected?.tag ?? selectedTag;

  const choiceScope = useIdentityScope(client, source, choiceTrack?.branch.id ?? null, datedMode && !!choiceTrack);
  const choiceBounds = useMemo(
    () => (choiceTrack ? [choiceTrack.branch.firstHour, choiceTrack.branch.lastHour] as const : undefined),
    [choiceTrack?.branch.firstHour, choiceTrack?.branch.lastHour],
  );
  const datedAsks = useMemo<readonly IdentityAsk[]>(() => {
    if (!datedMode || !choiceTrack) return NO_ASKS;
    const tags = new Set<string>(named.map(choice => choice.country.tag));
    if (displayHeaderTag) tags.add(displayHeaderTag);
    const seen = new Set<string>();
    const out: IdentityAsk[] = [];
    for (const tag of tags) {
      const read = choiceRead(tag);
      if (!read) continue;
      const key = `${read.hour}\u0000${read.tag}`;
      if (seen.has(key)) continue;
      seen.add(key);
      out.push({ key: countryTagKey(read.tag), hour: read.hour });
    }
    return out;
  }, [datedMode, choiceTrack, named, displayHeaderTag, choiceReads, flagAt?.branchId, flagAt?.hour]);

  const datedIdentities = useIdentities(client, source, datedMode ? choiceScope : null, datedAsks, {
    active: datedMode && !!choiceScope && datedAsks.length > 0,
    needFlags: open,
    flagSize: 'panel',
    branchBounds: choiceBounds,
  });

  const countryNameRead = (country: CountryOption): NameRead => {
    if (!datedMode) return { kind: 'value', value: country.name };
    const read = choiceRead(country.tag);
    if (!read) return ABSENT_NAME;
    return nameReadFromIdentity(datedIdentities.resolve(countryTagKey(read.tag), read.hour));
  };
  const countryName = (country: CountryOption) => nameReadMetadata(countryNameRead(country)).name;
  const filtered = filterChoices(choices, query, choice => countryName(choice.country));
  const range = visibleRange(filtered.length, view.scrollTop, view.height);
  const visible = open ? filtered.slice(range.start, range.end) : [];
  const sole = searching && filtered.length === 1 ? 0 : -1;
  const shownActive = active >= 0 ? active : sole;

  const defaultBranch = source?.metadata.branches[0] ?? null;
  const undatedScope = useIdentityScope(client, source, defaultBranch?.id ?? null, !datedMode && open && !!defaultBranch);
  const undatedBounds = useMemo(
    () => (defaultBranch ? [defaultBranch.firstHour, defaultBranch.lastHour] as const : undefined),
    [defaultBranch?.firstHour, defaultBranch?.lastHour],
  );
  const undatedAsks = useMemo<readonly IdentityAsk[]>(() => {
    if (datedMode || !open || !defaultBranch) return NO_ASKS;
    const out: IdentityAsk[] = [];
    const seen = new Set<string>();
    for (const { country } of visible) {
      if (country.observedBranchId !== defaultBranch.id || country.observedHour == null) continue;
      const key = `${country.observedHour}\u0000${country.tag}`;
      if (seen.has(key)) continue;
      seen.add(key);
      out.push({ key: countryTagKey(country.tag), hour: country.observedHour });
    }
    return out;
  }, [datedMode, open, defaultBranch, visible]);
  const undatedIdentities = useIdentities(client, source, !datedMode && open ? undatedScope : null, undatedAsks, {
    active: !datedMode && open && !!undatedScope && undatedAsks.length > 0,
    needFlags: true,
    flagSize: 'panel',
    branchBounds: undatedBounds,
  });

  const rowFlagState = (country: CountryOption): { readonly image?: CountryFlagImage; readonly pending: boolean; readonly failure?: Failure } => {
    if (datedMode) {
      const read = choiceRead(country.tag);
      if (!read) return { pending: false };
      const res = datedIdentities.resolve(countryTagKey(read.tag), read.hour);
      if (res.kind === 'pending') return { pending: true };
      if (res.kind !== 'known') return { pending: false };
      if (res.flagChoice.kind === 'pending') return { pending: true };
      if (res.flagChoice.kind !== 'known') return { pending: false };
      const drawn = datedIdentities.flagFor(res.flagChoice.value);
      if (!drawn) return { pending: true };
      if (drawn.failed) return { pending: false, failure: drawn.failed };
      return { image: drawn.image, pending: false };
    }
    if (!defaultBranch || country.observedBranchId !== defaultBranch.id || country.observedHour == null) return { pending: false };
    const res = undatedIdentities.resolve(countryTagKey(country.tag), country.observedHour);
    if (res.kind === 'pending') return { pending: true };
    if (res.kind !== 'known' || res.flagChoice.kind !== 'known') return { pending: res.kind === 'known' && res.flagChoice.kind === 'pending' };
    const drawn = undatedIdentities.flagFor(res.flagChoice.value);
    if (!drawn) return { pending: true };
    if (drawn.failed) return { pending: false, failure: drawn.failed };
    return { image: drawn.image, pending: false };
  };

  const flagContext = useMemo(() => {
    if (!source?.metadata.branches.length) return null;
    const witness = selected ?? source.metadata.countries.find((country) => country.id === source.metadata.playerCountryId) ?? source.metadata.countries[0];
    const branch = source.metadata.branches.find((branch) => branch.id === witness?.observedBranchId) ?? source.metadata.branches[0];
    return {
      branchId: branch.id,
      hour: witness?.observedBranchId === branch.id ? witness.observedHour ?? branch.firstHour : branch.firstHour,
    };
  }, [source?.id, source?.revision, source?.metadata.branches, source?.metadata.countries, source?.metadata.playerCountryId, selected]);

  const headerRead = displayHeaderTag && datedMode ? choiceRead(displayHeaderTag) : null;
  const headerTag = datedMode ? headerRead?.tag ?? null : displayHeaderTag;
  const headerSize: CountryFlagSize = field ? 'panel' : 'shield';
  const headerBranchId = datedMode ? headerRead?.branchId ?? null : flagContext?.branchId ?? null;
  const headerHour = datedMode ? headerRead?.hour ?? null : flagContext?.hour ?? null;
  const headerBranch = useMemo(
    () => source?.metadata.branches.find(branch => branch.id === headerBranchId) ?? null,
    [source?.metadata.branches, headerBranchId],
  );
  const headerBounds = useMemo(
    () => (headerBranch ? [headerBranch.firstHour, headerBranch.lastHour] as const : undefined),
    [headerBranch?.firstHour, headerBranch?.lastHour],
  );
  const headerActive = !addLabel && !!source && !!headerTag && headerHour !== null && !!headerBranch;
  const headerScope = useIdentityScope(client, source, headerBranchId, headerActive);
  const headerKey = useMemo(() => (headerTag ? countryTagKey(headerTag) : null), [headerTag]);
  const headerIdentity = useIdentity(client, source, headerActive ? headerScope : null, headerKey, headerHour, {
    active: headerActive && !!headerScope && !!headerKey,
    needFlag: true,
    flagSize: headerSize,
    branchBounds: headerBounds,
  });

  const headerImage = headerActive ? headerIdentity.flag?.image : undefined;
  const headerNameRead: NameRead | undefined = addLabel
    ? undefined
    : datedMode
      ? (!displayHeaderTag ? undefined : !headerRead ? ABSENT_NAME : nameReadFromIdentity(headerIdentity.result))
      : (selected ? { kind: 'value', value: selected.name } : undefined);
  const headerFlagPending = headerActive && headerIdentity.flagArt.status === 'pending';

  const visibleFlagStates = visible.map(({ country }) => rowFlagState(country));
  const flagFailure = headerIdentity.flag?.failed ?? visibleFlagStates.find(item => item.failure)?.failure;
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
    else {
      if (flagFailed) {
        datedIdentities.retry();
        undatedIdentities.retry();
        if (headerScope) identityBrokerFor(client).retry(headerScope);
      }
      setRetry(value => value + 1);
    }
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
              const read = countryNameRead(country);
              const name = nameReadMetadata(read).name.trim();
              const flagState = visibleFlagStates[offset]!;
              return <div key={country.tag} id={optionId(index)} role="option" aria-selected={ticked} aria-setsize={filtered.length} aria-posinset={index + 1}
                data-detail={nameReadFailure(read)?.message} data-tooltip-overflow={name || undefined} className="country-picker-option" data-active={index === shownActive || undefined} data-sole={index === sole && active < 0 || undefined} style={{ top: index * COUNTRY_ROW_HEIGHT }}
                onClick={() => choose(country)}>
                <CountryFlag image={flagState.image} tag={country.tag} pending={flagState.pending} decorative />
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
  const shownName = headerNameRead ? nameReadMetadata(headerNameRead).name.trim() : '';
  const detailReads = [headerNameRead, ...visible.map(({ country }) => countryNameRead(country))];
  return <div data-detail={failureDetails(detailReads.map(nameReadFailure))} className={field ? 'field widget-country-field' : 'header-control header-country'}>
    <span className={field ? undefined : 'sr-only'}>Country</span>
    <button ref={trigger} type="button" className={field ? 'widget-country-trigger' : 'header-country-trigger'} disabled={triggerDisabled} popoverTarget={id}
      aria-label={addLabel ?? `${field ? 'Widget country' : 'Choose country'}${shownName || shownTag ? `, currently ${shownName || shownTag}` : ''}`} aria-haspopup="dialog" aria-expanded={open}
      aria-description={!addLabel && (field || source) && !headerImage ? `Flag ${headerFlagPending ? 'loading' : 'unavailable'}` : undefined}
      data-tooltip-overflow={!addLabel && shownName ? shownName : undefined}>
      {addLabel ? <Icon name="plus" /> : (field || source) && <CountryFlag image={headerImage} tag={shownTag} name={shownName || null}
        pending={headerFlagPending} decorative />}
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
