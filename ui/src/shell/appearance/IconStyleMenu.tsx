import { useCallback, useEffect, useId, useRef, useState } from 'react';
import type { ChangeEvent } from 'react';
import type { MenuItem } from '../../components/ContextMenu';
import { useContextMenu } from '../../components/ContextMenu';
import { Icon } from '../../components/Icons';
import { FailureLine } from '../../components/FailureLine';
import type { IconName } from '../../components/Icons';
import { ICON_NAMES } from '../../components/iconNames';
import { Popover, usePopover } from '../../components/Popover';
import { ColorPicker } from './ColorField';
import { HEX_HINT, colorInput } from './color';
import { ICON_IMAGE_TYPES, latestIconImage } from './iconImage';
import { iconStyleOf, isIconImage, withIconStyle } from './iconStyle';
import type { IconStyle, IconStyles } from './iconStyle';
import type { AppearanceSettings } from './settings';
import type { Appearance } from './useAppearance';
import '../settings.css';
import './iconStyle.css';

export interface IconStyleSource<Id extends string> {
  label: string;
  read: (settings: AppearanceSettings) => IconStyles<Id>;
  write: (settings: AppearanceSettings, styles: IconStyles<Id>) => AppearanceSettings;
  defaultIcon: (id: Id) => IconName;
  defaultColour: (id: Id) => string;
  swatches: readonly string[];
}

type Part = 'icon' | 'colour';
type Change = { [Part in keyof IconStyle]?: IconStyle[Part] | null };
export const IMAGE_COLOURS = 'Images keep their own colours';
const iconLabel = (name: IconName) => name[0].toUpperCase() + name.slice(1).replaceAll('-', ' ');

export function restoreFocus(target: HTMLElement | null, popup: HTMLElement | null, active: Element | null, body: Element | null) {
  if (!target?.isConnected) return false;
  if (active && active !== body && !popup?.contains(active)) return false;
  target.focus({ preventScroll: true });
  return true;
}

export function useIconStyleMenu<Id extends string>(appearance: Appearance, source: IconStyleSource<Id>) {
  const actions = useContextMenu(`${source.label} actions`);
  const picker = usePopover('start');
  const [editing, setEditing] = useState<{ id: Id; label: string; part: Part } | null>(null);
  const [text, setText] = useState('');
  const [query, setQuery] = useState('');
  const [refusal, setRefusal] = useState<string | null>(null);
  const file = useRef<HTMLInputElement>(null);
  const revisions = useRef(new Map<Id, number>());
  const returnTo = useRef<HTMLElement | null>(null);
  const live = useRef({ appearance, source });
  live.current = { appearance, source };
  const styles = source.read(appearance.settings);
  const style = (id: Id) => iconStyleOf(styles, id);
  const colour = (id: Id) => style(id).colour ?? source.defaultColour(id);
  const value = editing ? colour(editing.id) : '';
  useEffect(() => setText(value), [value]);
  const change = useCallback((id: Id, parts: Change) => {
    const { appearance, source } = live.current;
    revisions.current.set(id, (revisions.current.get(id) ?? 0) + 1);
    const next = { ...parts };
    if (next.icon === source.defaultIcon(id)) next.icon = null;
    if (next.colour === source.defaultColour(id)) next.colour = null;
    appearance.update(previous => source.write(previous, withIconStyle(source.read(previous), id, next)));
  }, []);
  const { openAt, trigger } = picker;
  const edit = useCallback((id: Id, anchor: HTMLElement, label: string, part: Part, focusAfter?: HTMLElement | null) => {
    setEditing({ id, label, part });
    setQuery('');
    setRefusal(null);
    setText(iconStyleOf(live.current.source.read(live.current.appearance.settings), id).colour ?? live.current.source.defaultColour(id));
    trigger.current = anchor instanceof HTMLButtonElement ? anchor : null;
    returnTo.current = focusAfter ?? trigger.current;
    openAt(() => anchor.isConnected ? anchor.getBoundingClientRect() : null);
  }, [openAt, trigger]);
  const items = useCallback((id: Id, anchor: HTMLElement, label: string, focusAfter?: HTMLElement | null): MenuItem[] => {
    const image = isIconImage(iconStyleOf(live.current.source.read(live.current.appearance.settings), id).icon);
    return [
      { label: 'Change icon', icon: 'image', run: () => edit(id, anchor, label, 'icon', focusAfter) },
      { label: 'Change icon colour', icon: 'palette', disabled: image, hint: image ? IMAGE_COLOURS : undefined, run: () => edit(id, anchor, label, 'colour', focusAfter) },
    ];
  }, [edit]);
  const resets = useCallback((id: Id): MenuItem[] => {
    const own = iconStyleOf(live.current.source.read(live.current.appearance.settings), id);
    return [
      ...own.icon ? [{ label: 'Reset icon', icon: 'refresh' as const, run: () => change(id, { icon: null }) }] : [],
      ...own.colour ? [{ label: 'Reset icon colour', icon: 'refresh' as const, run: () => change(id, { colour: null }) }] : [],
    ];
  }, [change]);
  const [invalid, setInvalid] = useState(false);
  const hintId = useId();
  useEffect(() => setInvalid(false), [value, editing]);
  const commit = () => {
    const next = colorInput(text);
    setInvalid(!next);
    if (next && editing) change(editing.id, { colour: next });
  };
  const upload = async (event: ChangeEvent<HTMLInputElement>) => {
    const input = event.currentTarget;
    const chosen = input.files?.[0];
    input.value = '';
    if (!chosen || !editing) return;
    const id = editing.id;
    const drawn = await latestIconImage(id, chosen, revisions.current);
    if (!drawn) return;
    const { image } = drawn;
    if (image) { setRefusal(null); change(id, { icon: image }); }
    else setRefusal(`“${chosen.name}” couldn’t be read as an image. The icon is unchanged.`);
  };
  const current = editing ? style(editing.id) : {};
  const shown = editing ? current.icon ?? source.defaultIcon(editing.id) : null;
  const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const matches = ICON_NAMES.filter(name => words.every(word => name.replaceAll('-', ' ').includes(word)));
  const heading = (part: Part) => editing && <div className="ui-popover-heading"><strong>{editing.label} icon</strong><div className="ui-popover-actions">
    {current[part] && <button type="button" className="icon-button ui-popover-action" aria-label={part === 'icon' ? 'Reset icon' : 'Reset icon colour'}
      data-tooltip={part === 'icon' ? 'Reset icon' : 'Reset icon colour'} onClick={() => change(editing.id, { [part]: null })}><Icon name="refresh" /></button>}
    <button type="button" className="icon-button ui-popover-action" aria-label={part === 'icon' ? 'Close icon picker' : 'Close icon colour picker'} onClick={picker.close}><Icon name="close" /></button>
  </div></div>;
  const menu = <>
    {actions.menu}
    <Popover menu={picker} className={`color-picker icon-style-picker${editing?.part === 'icon' ? ' is-icons' : ''}`}
      onToggle={event => { if (event.newState === 'closed') restoreFocus(returnTo.current, picker.popup.current, document.activeElement, document.body); }}
      aria-label={editing ? `${editing.label} ${editing.part === 'icon' ? 'icon' : 'icon colour'}` : 'Icon'}>
      {editing?.part === 'colour' && <>
        {heading('colour')}
        {picker.open && <ColorPicker value={value} onChange={colour => change(editing.id, { colour })} swatches={source.swatches} />}
        <label className="icon-style-hex"><span>Hex</span><input data-popover-autofocus className="color-hex" aria-label="Icon hex value" value={text} spellCheck={false} maxLength={9}
          aria-invalid={invalid || undefined} aria-describedby={invalid ? hintId : undefined}
          onChange={event => { setText(event.currentTarget.value); setInvalid(false); }} onBlur={commit} onKeyDown={event => { if (event.key === 'Enter') { event.preventDefault(); commit(); } }} /></label>
        {invalid && <div className="icon-style-note" id={hintId}><FailureLine message={HEX_HINT} /></div>}
      </>}
      {editing?.part === 'icon' && <>
        {heading('icon')}
        <input type="search" className="icon-style-search" data-popover-autofocus aria-label="Search icons" placeholder="Search icons" value={query}
          onChange={event => setQuery(event.currentTarget.value)}
          onKeyDown={event => { if (event.key === 'Escape' && query) { event.preventDefault(); event.stopPropagation(); setQuery(''); } }} />
        {picker.open && <div className="icon-style-grid" role="group" aria-label="Icons">
          {matches.map(name => <button key={name} type="button" aria-label={iconLabel(name)} data-tooltip={iconLabel(name)} aria-pressed={name === shown}
            onClick={() => change(editing.id, { icon: name })}><Icon name={name} style={{ color: colour(editing.id) }} /></button>)}
          {!matches.length && <p className="icon-style-empty">No icons match “{query.trim()}”.</p>}
        </div>}
        <div className="icon-style-upload">
          {isIconImage(current.icon) && <img className="icon-image" src={current.icon} alt="Your image" />}
          <button type="button" className="button" onClick={() => file.current?.click()}><Icon name="upload" />{isIconImage(current.icon) ? 'Replace image…' : 'Upload image…'}</button>
          <input ref={file} type="file" hidden accept={ICON_IMAGE_TYPES} onChange={event => void upload(event)} />
        </div>
      </>}
      {editing && refusal && <FailureLine className="icon-style-note" message={refusal} onRetry={() => file.current?.click()} />}
      {editing && !refusal && appearance.notice && <div className="notice warning icon-style-note" role="status"><Icon name="alert" /><span>{appearance.notice}</span></div>}
    </Popover>
  </>;
  return { style, colour, items, resets, edit, change, open: actions.open, menu };
}
