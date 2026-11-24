import { Popover, PopoverBody, usePopover } from '../components/Popover';
import { useContext, useEffect, useId, useMemo, useRef, useState } from 'react';
import type { CSSProperties } from 'react';
import { Icon } from '../components/Icons';
import { StyledIcon } from '../shell/appearance/StyledIcon';
import { WidgetIconsContext } from '../shell/appearance/WidgetIconMenu';
import { widgetDefinitions } from './definitions';
import type { WidgetDefinition, WidgetPresets, WidgetRuntime } from './definitions';
import type { BoundTrack, WidgetSpec } from './model';
import { useProduct } from './products';
import { availabilityReason, availabilityRequest, scopeReason, validateWidgetAvailability } from './widgetAvailability';
import WidgetPreview from './WidgetPreview';
import { catalogueEntries, matchesWidget, placedCount, widgetGroups } from './widgetCategories';
import { presetWidget, widgetViewOptions, type PresetWidget } from '../widgets/manifest';
import type { CatalogueEntry, WidgetGroup } from './widgetCategories';

type Add = (kind: string, initial?: Pick<WidgetSpec, 'title' | 'options'>) => void;
type Entry = CatalogueEntry<WidgetDefinition>;
const initialOf = (entry: Entry) => (entry.initial ? presetWidget(entry.definition.kind, entry.initial as PresetWidget) : undefined) as Pick<WidgetSpec, 'title' | 'options'> | undefined;
interface Shared { runtime: WidgetRuntime; track: BoundTrack | undefined; active: boolean; disabled: boolean; onAdd: Add }

function useAdded(): [boolean, () => void] {
  const [added, setAdded] = useState(0);
  useEffect(() => {
    if (!added) return;
    const timer = setTimeout(() => setAdded(0), 1400);
    return () => clearTimeout(timer);
  }, [added]);
  return [added > 0, () => setAdded(count => count + 1)];
}

function useUnavailable(definition: WidgetDefinition, { runtime, track, active }: Shared) {
  const scoped = scopeReason(definition, track?.source.metadata.captureMode);
  const { data, error } = useProduct(runtime.client, scoped ? null : availabilityRequest(definition, track), active, validateWidgetAvailability, track?.source);
  return { reason: scoped ?? availabilityReason(!!definition.availability, !!track, data, error), detail: error?.message };
}

const groupColour = (group: WidgetGroup<unknown>) => ({ '--group-colour': group.page ? `var(--sidebar-icon-${group.page})` : 'var(--muted)' }) as CSSProperties;

function Card({ entry, placed, selected, onSelect, ...shared }: Shared & { entry: Entry; placed: number; selected: boolean; onSelect: () => void }) {
  const { definition } = entry;
  const { reason, detail } = useUnavailable(definition, shared);
  const [added, markAdded] = useAdded();
  const icon = useContext(WidgetIconsContext).style(definition.kind);
  const add = () => { shared.onAdd(definition.kind, initialOf(entry)); markAdded(); };
  return <div role="listitem" className="widget-card" data-selected={selected || undefined} data-variant={entry.variant || undefined} style={groupColour(entry.group)}>
    <button type="button" className="widget-card-main" aria-pressed={selected} data-tooltip={entry.title} onClick={onSelect}
      onDoubleClick={() => { if (!shared.disabled && reason === null) add(); }}>
      <span className="widget-card-icon" aria-hidden="true"><StyledIcon icon={icon.icon} fallback={definition.icon} colour={icon.colour} /></span>
      { }
      <span className="widget-card-text"><strong>{entry.title}</strong>
        {reason ? <small data-detail={detail}>{reason}</small> : placed > 0 && <small className="widget-card-placed">{placed > 1 ? `On page ×${placed}` : 'On page'}</small>}</span>
    </button>
    <button type="button" className="icon-button widget-card-add" aria-label={`Add ${entry.title}`} data-tooltip={reason ?? 'Add to page'}
      disabled={shared.disabled || reason !== null} onClick={add}>{added ? <Icon name="check" /> : <Icon name="plus" />}</button>
  </div>;
}

function Details({ entry, ...shared }: Shared & { entry: Entry }) {
  const { definition } = entry;
  const { reason, detail } = useUnavailable(definition, shared);
  const [added, markAdded] = useAdded();
  const countries = !definition.data ? null : definition.countrySelection === false ? 'Whole world'
    : (definition.countries?.max ?? 1) > 1 ? 'Several countries' : 'One country';
  return <aside className="widget-details" aria-label={`${entry.title} details`}>
    <WidgetPreview definition={definition} initial={initialOf(entry)} runtime={shared.runtime} />
    <div className="widget-details-text">
      <h3>{entry.title}</h3>
      {definition.description && <p>{definition.description}</p>}
      <div className="widget-details-tags">
        <span style={groupColour(entry.group)}><i aria-hidden="true" />{entry.group.label}</span>
        <span>{definition.scope === 'history' ? 'Over time' : 'One moment'}</span>
        {countries && <span>{countries}</span>}
        {entry.variant && <span>View of {definition.title}</span>}
      </div>
    </div>
    {(reason ?? entry.note) && <p className="widget-details-reason" role="status" data-detail={detail ?? entry.detail}>{reason ?? entry.note}</p>}
    <button type="button" className="button primary widget-details-add" disabled={shared.disabled || reason !== null}
      onClick={() => { shared.onAdd(definition.kind, initialOf(entry)); markAdded(); }}>
      {added ? <><Icon name="check" />Added</> : <><Icon name="plus" />Add to page</>}</button>
  </aside>;
}

const ON_PAGE = Symbol('on this page');

function Catalogue({ open, placed, ...shared }: Shared & { open: boolean; placed: readonly WidgetSpec[] }) {
  const [query, setQuery] = useState('');
  const [category, setCategory] = useState<string | typeof ON_PAGE | null>(null);
  const [chosen, setChosen] = useState<string | null>(null);
  const search = useRef<HTMLInputElement>(null);
  useEffect(() => { if (!open) setQuery(''); }, [open]);
  const definitions = useMemo(() => widgetDefinitions(), [open]);
  const groups = useMemo(() => widgetGroups(definitions), [definitions]);
  const presets = new Map<string, WidgetPresets>();
  for (const definition of definitions) if (definition.usePresets) presets.set(definition.kind, definition.usePresets(definition, shared.runtime, shared.track, shared.active));
  const byGroup = catalogueEntries(groups, presets).map(entries => entries.filter(entry =>
    matchesWidget(query, entry.title, entry.definition.title, entry.definition.description, entry.group.label)));
  const resolvedPlaced = useMemo(() => placed.map(widget => ({ kind: widget.kind, options: widgetViewOptions(widget.kind, widget.options) })), [placed]);
  const count = (entry: Entry) => placedCount(entry, resolvedPlaced);
  const visible = byGroup.flatMap((entries, index) => category === null || category === ON_PAGE || groups[index].label === category ? entries : [])
    .filter(entry => category !== ON_PAGE || count(entry) > 0);
  const selected = visible.find(entry => entry.key === chosen) ?? visible[0];
  const total = byGroup.reduce((sum, entries) => sum + entries.length, 0);
  const onPage = byGroup.reduce((sum, entries) => sum + entries.filter(entry => count(entry) > 0).length, 0);
  return <div className="widget-catalogue-body" onKeyDownCapture={event => {
    if (event.key === '/' && event.target !== search.current) { event.preventDefault(); search.current?.focus(); }
  }}>
    <label className="widget-catalogue-search"><Icon name="search" />
      <input ref={search} type="search" data-popover-autofocus aria-label="Search widgets" placeholder={`Search ${total === 1 ? '1 widget' : `${total} widgets`}`}
        value={query} onChange={event => { setQuery(event.currentTarget.value); setCategory(view => view === ON_PAGE ? view : null); }}
        onKeyDown={event => { if (event.key === 'Escape' && query) { event.preventDefault(); event.stopPropagation(); setQuery(''); } }} />
      <kbd aria-hidden="true">/</kbd></label>
    <nav className="widget-catalogue-rail" aria-label="Widget categories">
      <button type="button" aria-current={category === null || undefined} onClick={() => setCategory(null)}>
        <Icon name="grid" /><span>All widgets</span><small>{total}</small></button>
      <button type="button" aria-current={category === ON_PAGE || undefined} onClick={() => setCategory(ON_PAGE)}>
        <Icon name="check" /><span>On this page</span><small>{onPage}</small></button>
      <hr />
      {groups.map((group, index) => <button key={group.label} type="button" aria-current={category === group.label || undefined} disabled={!byGroup[index].length}
        style={groupColour(group)} onClick={() => setCategory(group.label)}>
        {group.icon ? <Icon name={group.icon} /> : <i aria-hidden="true" />}<span>{group.label}</span><small>{byGroup[index].length}</small></button>)}
    </nav>
    <div className="widget-catalogue-cards" role="list" aria-label={category === ON_PAGE ? 'On this page' : category ?? 'All widgets'}>
      {visible.map(entry => <Card key={entry.key} entry={entry} placed={count(entry)} selected={entry.key === selected?.key}
        onSelect={() => setChosen(entry.key)} {...shared} />)}
      {!visible.length && <p className="widget-catalogue-empty">{query.trim() ? `No widgets match “${query.trim()}”.`
        : category === ON_PAGE ? 'No widgets on this page yet.' : 'No widgets in this category.'}</p>}
    </div>
    {selected ? <Details key={selected.key} entry={selected} {...shared} />
      : <aside className="widget-details is-empty"><p>Choose a widget to see it here.</p></aside>}
  </div>;
}

export default function WidgetPicker({ disabled, onAdd, onOpen, runtime, placed = [] }: {
  disabled: boolean; onAdd: Add; onOpen: () => void; runtime: WidgetRuntime;
  placed?: readonly WidgetSpec[];
}) {
  const id = useId();
  const menu = usePopover();
  const { trigger, open } = menu;
  const track = runtime.tracks.find(item => item.roleId === runtime.defaultRoleId);
  return <>
    <button ref={trigger} type="button" className="icon-button header-add-widget" aria-label="Add widget" data-tooltip="Add widgets" disabled={disabled}
      popoverTarget={id} aria-haspopup="dialog" aria-expanded={open} onClick={onOpen}><Icon name="plus" /></button>
    <Popover menu={menu} id={id} className="widget-catalogue" aria-label="Add widgets">
      <div className="ui-popover-heading"><strong>Add widgets</strong><button type="button" className="icon-button ui-popover-action" aria-label="Close widget catalogue" popoverTarget={id} popoverTargetAction="hide"><Icon name="close" /></button></div>
      <PopoverBody open={open}>{() => <Catalogue open={open} placed={placed} runtime={runtime} track={track} active={open} disabled={disabled} onAdd={onAdd} />}</PopoverBody>
    </Popover>
  </>;
}
