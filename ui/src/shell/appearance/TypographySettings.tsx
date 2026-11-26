import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent, ReactNode } from 'react';
import { Icon } from '../../components/Icons';
import { FailureLine } from '../../components/FailureLine';
import { Popover, usePopover } from '../../components/Popover';
import { Select } from '../../components/Select';
import { ConfirmRow, SettingsBlock } from '../SettingsBlock';
import { desktopAvailable } from '../api';
import { FONT_PRESETS, canUseFontFamily, defaultFontLabel, fontStack, isMonospaceFamily } from './fonts';
import { discoverInstalledFonts, useFontDiscovery } from './fontDiscovery';
import { DEFAULT_TYPOGRAPHY, FONT_SIZE_LIMITS } from './typography';
import type { SizedRole } from './typography';
import type { FontRole } from './typography';
import type { Appearance } from './useAppearance';

function FontOptions({ label, families, value, placeholder, focusSearch, onPick }: {
  label: string; families: string[]; value: string; placeholder: string; focusSearch: boolean; onPick: (family: string) => void;
}) {
  const id = useId();
  const list = useRef<HTMLDivElement>(null);
  const [query, setQuery] = useState('');
  const [active, setActive] = useState(Math.max(0, families.indexOf(value)));
  const [scrollTop, setScrollTop] = useState(0);
  const fontLabel = (family: string) => family ? FONT_PRESETS.find(preset => preset.family === family)?.label ?? family : placeholder;
  const filtered = useMemo(() => families.filter(family => `${family} ${fontLabel(family)}`.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase())), [families, query, placeholder]);
  const at = Math.min(active, filtered.length - 1);
  const rowHeight = 30, listHeight = 240;
  const start = Math.max(0, Math.floor(scrollTop / rowHeight) - 2);
  const end = Math.min(filtered.length, Math.ceil((scrollTop + listHeight) / rowHeight) + 2);
  useLayoutEffect(() => {
    const node = list.current;
    if (!node || at < 0) return;
    if (at * rowHeight < node.scrollTop) node.scrollTop = at * rowHeight;
    else if ((at + 1) * rowHeight > node.scrollTop + node.clientHeight) node.scrollTop = (at + 1) * rowHeight - node.clientHeight;
    setScrollTop(node.scrollTop);
  }, [at, filtered]);
  const keys = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      if (filtered.length) setActive((at + (event.key === 'ArrowDown' ? 1 : filtered.length - 1)) % filtered.length);
    } else if (event.key === 'Enter' && at >= 0) {
      event.preventDefault(); onPick(filtered[at]);
    }
  };
  return <div onKeyDown={keys}>
    <input type="search" className="font-menu-search" aria-label={`Search ${label.toLowerCase()} fonts`} placeholder="Search fonts" role="combobox" aria-expanded="true" aria-controls={id}
      aria-activedescendant={at >= 0 ? `${id}-${at}` : undefined} data-popover-autofocus={focusSearch || undefined} value={query}
      onChange={event => { setQuery(event.currentTarget.value); setActive(0); setScrollTop(0); if (list.current) list.current.scrollTop = 0; }} />
    <div ref={list} id={id} className="font-menu-list" role="listbox" aria-label={label} onScroll={event => setScrollTop(event.currentTarget.scrollTop)}>
      <div style={{ paddingTop: start * rowHeight, paddingBottom: Math.max(0, filtered.length - end) * rowHeight }}>
        {filtered.slice(start, end).map((family, index) => <button key={family} id={`${id}-${start + index}`} type="button" role="option" tabIndex={-1}
          className={`menu-item${start + index === at ? ' is-active' : ''}`} aria-selected={family === value} onClick={() => onPick(family)}>
          <span className="menu-item-label" style={{ fontFamily: fontStack(family) }}>{fontLabel(family)}</span>{family === value && <Icon name="check" />}
        </button>)}
      </div>
      {!filtered.length && <p className="font-menu-empty">No fonts found.</p>}
    </div>
  </div>;
}

function FontPicker({ id, label, hint, value, placeholder, onChange, installed, onListFonts, role, size, onSize, onReset, preview, monospace = false }: {
  id?: string; label: string; hint: string; value: string; placeholder: string; onChange: (value: string) => void; installed: string[] | null; onListFonts: () => void;
  role: FontRole; size?: number; onSize: (size: number) => void; onReset: () => void; preview?: ReactNode; monospace?: boolean;
}) {
  const listId = useId();
  const menu = usePopover();
  const input = useRef<HTMLInputElement>(null);
  const inputFocused = useRef(false);
  const anchor = useRef<HTMLDivElement>(null);
  const focusOptions = useRef(true);
  const [message, setMessage] = useState<string | null>(null);
  const families = useMemo(() => [...new Set([
    ...FONT_PRESETS.filter(preset => !monospace || !preset.family || preset.monospace).map(preset => preset.family),
    ...(installed ?? []), ...(canUseFontFamily(value, installed) ? [value] : []),
  ])], [installed, value, monospace]);
  const openFonts = (focusMenu = true) => {
    focusOptions.current = focusMenu;
    onListFonts();
    menu.openAt(() => anchor.current?.getBoundingClientRect() ?? null);
  };
  useLayoutEffect(() => { if (installed && inputFocused.current) openFonts(); }, [installed]);
  const [text, setText] = useState(value);
  const [settled, setSettled] = useState(true);
  useEffect(() => { setText(value); setSettled(true); }, [value]);
  const commit = () => {
    setSettled(true);
    if (canUseFontFamily(text, installed) && (!monospace || isMonospaceFamily(text))) onChange(text.trim());
  };
  useEffect(() => {
    if (text.trim() === value) return;
    const timer = setTimeout(commit, 400);
    return () => clearTimeout(timer);
  }, [text, value, installed, onChange]);
  const available = canUseFontFamily(value, installed);
  const rejected = settled && text.trim() !== value && (!canUseFontFamily(text, installed) || (monospace && !isMonospaceFamily(text)));
  const pick = (family: string) => {
    if (monospace && !isMonospaceFamily(family)) {
      setMessage(`“${family}” isn’t monospace. Your current font is unchanged.`); menu.close(); return;
    }
    setText(family); setSettled(true); setMessage(null); onChange(family); menu.close();
  };
  const sized = role === 'tables' ? null : role;
  const [min, max] = sized ? FONT_SIZE_LIMITS[sized] : [0, -1];
  const changed = !!value || (!!sized && size !== DEFAULT_TYPOGRAPHY.sizes[sized]);
  return <div className="font-setting" id={id}>
    <div className="settings-row font-row">
    <span className="settings-row-text"><strong>{label}</strong><small>{hint}</small></span>
    <div className="font-picker">
      <div className="font-controls">{changed && <button type="button" className="icon-button small font-reset" aria-label={`Reset ${label.toLowerCase()}${sized ? ' and its size' : ''}`}
        data-tooltip={`Reset ${label.toLowerCase()}${sized ? ' and its size' : ''}`} data-tooltip-side="left"
        onClick={() => { setText(''); setSettled(true); setMessage(null); onReset(); }}><Icon name="refresh" /></button>}<div className="font-input" ref={anchor}>
      {installed ? <button ref={menu.trigger} type="button" className="select-trigger font-family-trigger" aria-label={label} aria-haspopup="listbox" aria-expanded={menu.open} aria-controls={listId}
        onClick={() => menu.open ? menu.close() : openFonts()}><span>{FONT_PRESETS.find(preset => preset.family === value && value)?.label ?? (value || placeholder)}</span><Icon name="chevron-down" /></button>
        : <><input ref={input} value={text} placeholder={placeholder} aria-label={label} role="combobox" aria-expanded={menu.open} aria-controls={listId} aria-haspopup="listbox" maxLength={200} autoComplete="off" autoCapitalize="off"
        aria-invalid={rejected || undefined} onFocus={() => { inputFocused.current = true; onListFonts(); }} onBlur={() => { inputFocused.current = false; if (text.trim() !== value) commit(); }}
        onClick={() => openFonts(false)} onChange={event => { setText(event.currentTarget.value); setSettled(false); setMessage(null); }} spellCheck={false}
        onKeyDown={event => {
          if (event.key === 'ArrowDown') { event.preventDefault(); openFonts(); }
          else if (event.key === 'Enter') { event.preventDefault(); commit(); }
          else if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); setText(value); setSettled(true); setMessage(null); menu.close(); }
        }} />
        <button ref={menu.trigger} type="button" className="icon-button small font-dropdown-trigger" aria-label={`Choose ${label.toLowerCase()}`} aria-haspopup="listbox" aria-expanded={menu.open} aria-controls={listId}
          onClick={() => menu.open ? menu.close() : openFonts()}><Icon name="chevron-down" /></button></>}
      </div>{sized && <div className="font-size-control"><Select label={`${label} size`} value={String(size)} onChange={value => onSize(Number(value))}
        options={Array.from({ length: max - min + 1 }, (_, i) => ({ value: String(min + i), label: `${min + i}px` }))} /></div>}</div>
      <Popover menu={menu} id={listId} className="font-menu" aria-label={label}>
        {menu.open && <FontOptions label={label} families={families} value={value} placeholder={placeholder} focusSearch={focusOptions.current}
          onPick={pick} />}
      </Popover>
      {message ? <FailureLine message={message} /> : rejected ? <FailureLine message={`“${text.trim()}” ${canUseFontFamily(text, installed) ? 'isn’t monospace' : 'wasn’t found'}. Your current font is unchanged.`} />
        : value && !available && <small className="font-missing"><Icon name="alert" />“{value}” {desktopAvailable ? 'isn’t installed on this computer' : 'is unavailable in this browser'}. Using the system font.</small>}
    </div>
    </div>{preview && <div className="font-preview">{preview}</div>}
  </div>;
}

export default function TypographySettings({ appearance, searchTarget }: { appearance: Appearance; searchTarget?: string }) {
  const { settings, update } = appearance;
  const { typography } = settings;
  const discovery = useFontDiscovery();
  const [resetting, setResetting] = useState(false);
  const resetTrigger = useRef<HTMLButtonElement>(null);
  const cancelReset = () => { setResetting(false); requestAnimationFrame(() => resetTrigger.current?.focus()); };
  const fonts = discovery.status === 'granted' ? discovery.families : null;
  const defaults = useMemo(() => ({ ui: defaultFontLabel(), mono: defaultFontLabel(true) }), []);
  const discover = () => { void discoverInstalledFonts(); };
  const isMac = /mac|iphone|ipad|ipod/i.test(navigator.platform);
  const lastTarget = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (!searchTarget) { lastTarget.current = undefined; return; }
    if (lastTarget.current === searchTarget) return;
    lastTarget.current = searchTarget;
    if (['appearance-display-font', 'appearance-diagnostics-font', ...(isMac ? ['appearance-font-smoothing'] : [])].includes(searchTarget)) {
      update(previous => ({ ...previous, typography: { ...previous.typography, advanced: true } }));
    }
  }, [searchTarget, isMac, update]);
  const row = (role: FontRole, label: string, hint: string, preview: ReactNode, monospace = false) => <FontPicker key={role}
    id={`appearance-${role}-font`} role={role} label={label} hint={hint} value={settings.fonts[role]} size={role === 'tables' ? undefined : typography.sizes[role]} preview={preview} monospace={monospace}
    placeholder={role === 'display' || role === 'tables' ? settings.fonts.interface || defaults.ui : monospace ? defaults.mono : defaults.ui} installed={fonts} onListFonts={discover}
    onChange={family => update(previous => ({ ...previous, fonts: { ...previous.fonts, [role]: family } }))}
    onSize={size => update(previous => ({ ...previous, typography: { ...previous.typography, sizes: { ...previous.typography.sizes, [role]: size } } }))}
    onReset={() => update(previous => ({ ...previous, fonts: { ...previous.fonts, [role]: '' }, typography: role === 'tables' ? previous.typography
      : { ...previous.typography, sizes: { ...previous.typography.sizes, [role]: DEFAULT_TYPOGRAPHY.sizes[role as SizedRole] } } }))} />;
  const headings = <div className="font-preview-headings"><h2>Campaign overview</h2><span>Research and production</span></div>;
  const diagnostics = <pre className="font-preview-diagnostics">{`Playback ready\nSamples: 12345    Coverage: complete\nFrame: 16.7 ms   Cache: 128 MiB`}</pre>;
  const changed = Object.values(settings.fonts).some(Boolean) || JSON.stringify({ ...typography, advanced: false }) !== JSON.stringify(DEFAULT_TYPOGRAPHY);
  return <SettingsBlock id="appearance-typography" title="Typography" actions={
    <label className="typography-advanced">Advanced<input type="checkbox" role="switch" aria-label="Show advanced typography settings" checked={typography.advanced}
      onChange={event => update(previous => ({ ...previous, typography: { ...previous.typography, advanced: event.currentTarget.checked } }))} /></label>}>
    {row('interface', 'Interface font', 'Menus, labels and controls.', typography.advanced ? null : <>{headings}<div className="font-preview-interface"><span>Example campaign · 12 Mar · 14:00</span><button type="button" className="button small">Preview control</button></div></>)}
    {typography.advanced && row('display', 'Headings font', 'Page headings, country names and widget titles.', headings)}
    {row('tables', 'Tables font', 'Data tables. Empty uses the interface font, as charts do.', <TypographyTablePreview />)}
    {typography.advanced && row('diagnostics', 'Diagnostics font', 'Technical reports and recording diagnostics.', diagnostics, true)}
    {typography.advanced && isMac && <label className="settings-row" id="appearance-font-smoothing">
      <span className="settings-row-text"><strong>Font smoothing</strong><small>Use thinner grayscale text smoothing instead of the macOS default.</small></span>
      <span className="settings-row-control">{!typography.smoothing && <button type="button" className="icon-button small font-reset" aria-label="Reset font smoothing" data-tooltip="Reset font smoothing" data-tooltip-side="left" onClick={() => update(previous => ({ ...previous, typography: { ...previous.typography, smoothing: true } }))}><Icon name="refresh" /></button>}
      <input type="checkbox" role="switch" aria-label="Font smoothing" checked={typography.smoothing} onChange={event => update(previous => ({ ...previous, typography: { ...previous.typography, smoothing: event.currentTarget.checked } }))} /></span>
    </label>}
    <label className="settings-row" id="appearance-word-wrap"><span className="settings-row-text"><strong>Word wrapping</strong><small>Wrap long text in tables and technical reports.</small></span>
      <span className="settings-row-control">{!typography.wordWrap && <button type="button" className="icon-button small font-reset" aria-label="Reset word wrapping" data-tooltip="Reset word wrapping" data-tooltip-side="left" onClick={() => update(previous => ({ ...previous, typography: { ...previous.typography, wordWrap: true } }))}><Icon name="refresh" /></button>}
      <input type="checkbox" role="switch" aria-label="Word wrapping" checked={typography.wordWrap} onChange={event => update(previous => ({ ...previous, typography: { ...previous.typography, wordWrap: event.currentTarget.checked } }))} /></span>
    </label>
    {changed && (resetting ? <ConfirmRow title="Reset typography?" action="Reset typography" onCancel={cancelReset}
      detail="Every font, text size and wrapping choice here returns to its default." onConfirm={() => {
        setResetting(false);
        update(previous => ({ ...previous, fonts: { interface: '', display: '', tables: '', diagnostics: '' }, typography: { ...structuredClone(DEFAULT_TYPOGRAPHY), advanced: previous.typography.advanced } }));
      }} />
      : <div className="settings-row"><span className="settings-row-text"><strong>Typography defaults</strong>
        <small>Returns every font, text size and wrapping choice here to its default.</small></span>
        <button ref={resetTrigger} type="button" className="button small" onClick={() => setResetting(true)}><Icon name="refresh" />Reset typography</button></div>)}
  </SettingsBlock>;
}

const previewRows = [
  { key: 'production', label: 'Equipment production across all active lines', value: '12,345.67' },
  { key: 'coverage', label: 'Coverage', value: '98.5%' },
];
function TypographyTablePreview() {
  return <table className="font-preview-table" aria-label="Typography table preview">
    <thead>
      <tr>
        <th scope="col">Category</th>
        <th scope="col" className="is-numeric">Value</th>
      </tr>
    </thead>
    <tbody>
      {previewRows.map(row => <tr key={row.key}>
        <td>{row.label}</td>
        <td className="is-numeric">{row.value}</td>
      </tr>)}
    </tbody>
  </table>;
}
