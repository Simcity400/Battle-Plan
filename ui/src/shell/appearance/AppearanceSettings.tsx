import { useEffect, useId, useRef, useState } from 'react';
import type { CSSProperties, ReactNode } from 'react';
import { Popover, usePopover } from '../../components/Popover';
import { Icon } from '../../components/Icons';
import type { IconName } from '../../components/Icons';
import { newId } from '../ids';
import { BUILT_IN_THEMES, themeColors } from './themes';
import type { ThemeDefinition, ThemeMode } from './themes';
import { parseThemeText, serializeTheme, uniqueThemeName } from './themeFile';
import { LIMITS, MAX_CUSTOM_THEMES } from './settings';
import type { AppearanceSettings as Settings, BackdropKind, ColorScheme, MotionPreference } from './settings';
import { deleteBackdrop, importBackdrop, listBackdrops } from './backdrops';
import type { BackdropImage } from './backdrops';
import { FONT_PRESETS, fontAvailable, fontStack, installedFonts } from './fonts';
import { ThemeSwatch, ThemeWireframe } from './ThemePreview';
import type { Appearance } from './useAppearance';
import type { ThemeDraft } from './ThemeEditor';

/** A titled group of settings: the title sits above one card whose rows are divided. */
export function SettingsBlock({ id, title, description, actions, children }: { id?: string; title: string; description?: string; actions?: ReactNode; children: ReactNode }) {
  return <section className="settings-block" id={id} aria-label={title}>
    <div className="settings-block-heading"><div><h3>{title}</h3>{description && <p>{description}</p>}</div>{actions && <div className="settings-block-actions">{actions}</div>}</div>
    <div className="settings-group">{children}</div>
  </section>;
}

function SliderRow({ id, label, hint, value, min, max, unit, onChange }: { id?: string; label: string; hint: string; value: number; min: number; max: number; unit: string; onChange: (value: number) => void }) {
  return <label className="settings-row" id={id}>
    <span className="settings-row-text"><strong>{label}</strong><small>{hint}</small></span>
    <span className="settings-slider"><input type="range" className="slider" min={min} max={max} value={value} aria-label={label}
      style={{ '--fill': `${(100 * (value - min)) / (max - min)}%` } as CSSProperties} onChange={event => onChange(Number(event.currentTarget.value))} />
      <output>{value}{unit}</output></span>
  </label>;
}

function Segmented<T extends string>({ value, options, onChange, label }: { value: T; options: readonly { value: T; label: string; icon?: IconName }[]; onChange: (value: T) => void; label: string }) {
  return <div className="segmented" role="group" aria-label={label}>{options.map(option => <button key={option.value} type="button" aria-pressed={value === option.value} onClick={() => onChange(option.value)}>
    {option.icon && <Icon name={option.icon} />}{option.label}</button>)}</div>;
}

export const newThemeId = () => `theme-${newId().replaceAll('-', '').slice(0, 20)}`;

function downloadTheme(theme: ThemeDefinition) {
  const url = URL.createObjectURL(new Blob([serializeTheme(theme)], { type: 'application/json' }));
  const link = document.createElement('a');
  link.href = url; link.download = `${theme.name.replace(/[^\p{L}\p{N} _-]+/gu, '').trim() || 'theme'}.json`;
  document.body.append(link); link.click(); link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

function ThemeCard({ theme, settings, onUse, onUseMode, onEdit, onDuplicate, onDelete, onMessage }: {
  theme: ThemeDefinition; settings: Settings; onUse: () => void; onUseMode: (mode: ThemeMode) => void;
  onEdit?: () => void; onDuplicate: () => void; onDelete?: () => void; onMessage: (text: string, tone?: 'positive' | 'warning') => void;
}) {
  const id = useId();
  const menu = usePopover();
  const active = (['dark', 'light'] as const).filter(mode => settings.themes[mode] === theme.id);
  const copy = () => { void navigator.clipboard.writeText(serializeTheme(theme)).then(() => onMessage(`Copied “${theme.name}” as JSON.`), () => onMessage('The clipboard is unavailable. Save the theme as a file instead.', 'warning')); menu.close(); };
  return <div className="theme-card" data-active={active.length > 0 || undefined}>
    <button type="button" className="theme-card-main" onClick={onUse} aria-label={`Use ${theme.name}`}>
      <span className="theme-card-name">{theme.name}</span>
      <span className="theme-card-kind">{theme.builtIn ? 'Built-in' : 'Custom'}{active.length > 0 && <span className="pill accent">In use</span>}</span>
    </button>
    <div className="theme-card-modes">{(['light', 'dark'] as const).map(mode => theme.variants[mode] ? <button key={mode} type="button" className="theme-card-mode" aria-pressed={settings.themes[mode] === theme.id}
      aria-label={`Use ${theme.name} for ${mode} mode`} data-tooltip={`Use for ${mode} mode`} onClick={() => onUseMode(mode)}>
      <ThemeSwatch colors={theme.variants[mode]!} />{settings.themes[mode] === theme.id && <Icon name="check" />}</button>
      : <span key={mode} className="theme-card-mode is-missing" aria-label={`No ${mode} variant`}><Icon name={mode === 'dark' ? 'moon' : 'sun'} /></span>)}</div>
    <button ref={menu.trigger} type="button" className="icon-button small theme-card-more" popoverTarget={id} aria-haspopup="menu" aria-expanded={menu.open} aria-label={`${theme.name} actions`}><Icon name="more" /></button>
    <Popover menu={menu} id={id} className="theme-card-menu" aria-label={`${theme.name} actions`}>
      {onEdit && <button type="button" className="menu-item" onClick={() => { menu.close(); onEdit(); }}><Icon name="edit" /><span className="menu-item-label">Edit</span></button>}
      <button type="button" className="menu-item" onClick={() => { menu.close(); onDuplicate(); }}><Icon name="copy" /><span className="menu-item-label">{onEdit ? 'Duplicate' : 'Duplicate and edit'}</span></button>
      <button type="button" className="menu-item" onClick={copy}><Icon name="link" /><span className="menu-item-label">Copy as JSON</span></button>
      <button type="button" className="menu-item" onClick={() => { downloadTheme(theme); menu.close(); }}><Icon name="download" /><span className="menu-item-label">Save as file</span></button>
      {onDelete && <button type="button" className="menu-item is-danger" onClick={() => { menu.close(); onDelete(); }}><Icon name="trash" /><span className="menu-item-label">Delete</span></button>}
    </Popover>
  </div>;
}

function ImportTheme({ disabled, onImport }: { disabled: boolean; onImport: (text: string) => string | null }) {
  const id = useId();
  const menu = usePopover();
  const [text, setText] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  const file = useRef<HTMLInputElement>(null);
  const run = (value: string) => { const failure = onImport(value); setError(failure); if (!failure) { setText(''); menu.close(); } };
  const read = (input: File | undefined) => {
    if (!input) return;
    if (input.size > 512 * 1024) { setError('Theme files must be smaller than 512 KB.'); return; }
    void input.text().then(run, () => setError('This file could not be read.'));
  };
  return <>
    <button ref={menu.trigger} type="button" className="button small quiet" disabled={disabled} popoverTarget={id} aria-haspopup="dialog" aria-expanded={menu.open}><Icon name="upload" />Import</button>
    <Popover menu={menu} id={id} className="theme-import" aria-label="Import a theme" onToggle={event => { if (event.newState === 'closed') { setError(null); setDragging(false); } }}>
      <div className="ui-popover-heading"><strong>Import a theme</strong><button type="button" className="icon-button ui-popover-action" aria-label="Close import" onClick={menu.close}><Icon name="close" /></button></div>
      <div className={`theme-dropzone${dragging ? ' is-dragging' : ''}`} onDragOver={event => { event.preventDefault(); setDragging(true); }} onDragLeave={() => setDragging(false)}
        onDrop={event => { event.preventDefault(); setDragging(false); read(event.dataTransfer.files[0]); }}>
        <Icon name="upload" />
        <span>Drop a theme file here, or <button type="button" className="text-button" onClick={() => file.current?.click()}>choose a file</button></span>
        <small>Battle Plan themes and VS Code colour themes (.json)</small>
        <input ref={file} type="file" accept=".json,application/json" hidden onChange={event => { read(event.currentTarget.files?.[0]); event.currentTarget.value = ''; }} />
      </div>
      <textarea rows={5} value={text} spellCheck={false} placeholder='…or paste theme JSON, e.g. { "name": "Dusk", "canvas": "#14121b", "accent": "#f2a65a" }'
        aria-label="Theme JSON" onChange={event => { setText(event.currentTarget.value); setError(null); }} />
      {error && <p className="map-menu-error" role="alert">{error}</p>}
      <div className="layout-save-actions"><button type="button" className="button primary" disabled={!text.trim()} onClick={() => run(text)}>Import theme</button></div>
    </Popover>
  </>;
}

function BackdropLibrary({ selected, onSelect, onRemoved }: { selected: string | null; onSelect: (id: string) => void; onRemoved: (id: string) => void }) {
  const [items, setItems] = useState<BackdropImage[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => { let current = true; void listBackdrops().then(list => { if (current) setItems(list); }, cause => { if (current) { setItems([]); setError(String(cause?.message ?? cause)); } }); return () => { current = false; }; }, []);
  const add = async (file: File | undefined) => {
    if (!file) return;
    setBusy(true); setError(null);
    try { const entry = await importBackdrop(file); setItems(previous => [...(previous ?? []), entry]); onSelect(entry.id); }
    catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
    finally { setBusy(false); }
  };
  return <>
    <div className="backdrop-grid">
      {items?.map(item => <div key={item.id} className="backdrop-thumb" data-selected={item.id === selected || undefined}>
        <button type="button" style={{ backgroundImage: `url("${item.thumbnail}")` }} aria-pressed={item.id === selected} aria-label={`Use ${item.name}`} onClick={() => onSelect(item.id)} />
        {item.id === selected && <span className="backdrop-check"><Icon name="check" /></span>}
        <button type="button" className="icon-button small backdrop-remove" aria-label={`Remove ${item.name}`} data-tooltip="Remove"
          onClick={() => { void deleteBackdrop(item.id).then(() => { setItems(previous => previous?.filter(entry => entry.id !== item.id) ?? null); onRemoved(item.id); })
            .catch(cause => setError(cause instanceof Error ? cause.message : String(cause))); }}><Icon name="trash" /></button>
      </div>)}
      <label className={`backdrop-add${busy ? ' is-busy' : ''}`}>
        <input type="file" accept="image/*" hidden disabled={busy} onChange={event => { void add(event.currentTarget.files?.[0]); event.currentTarget.value = ''; }} />
        {busy ? <span className="spinner" /> : <Icon name="image" />}<span>{busy ? 'Importing…' : 'Add image'}</span>
      </label>
    </div>
    {error && <p className="map-menu-error" role="alert">{error}</p>}
    {items !== null && <p className="field-hint">Images are resized to at most 2560 pixels and stored on this device only.</p>}
  </>;
}

function FontPicker({ id, label, hint, value, placeholder, onChange, installed, onListFonts }: {
  id?: string; label: string; hint: string; value: string; placeholder: string; onChange: (value: string) => void; installed: string[] | null; onListFonts: () => void;
}) {
  const listId = useId();
  const [text, setText] = useState(value);
  useEffect(() => setText(value), [value]);
  useEffect(() => {
    if (text.trim() === value) return;
    const timer = setTimeout(() => onChange(text.trim()), 350);
    return () => clearTimeout(timer);
  }, [text, value, onChange]);
  const available = !value || fontAvailable(value);
  return <div className="settings-row font-row" id={id}>
    <span className="settings-row-text"><strong>{label}</strong><small>{hint}</small></span>
    <div className="font-picker">
      <div className="font-input"><input list={listId} value={text} placeholder={placeholder} aria-label={label} onChange={event => setText(event.currentTarget.value)} spellCheck={false} />
        {window.queryLocalFonts && <button type="button" className="icon-button small" aria-label="List installed fonts" data-tooltip="List installed fonts" onClick={onListFonts}><Icon name="search" /></button>}</div>
      <datalist id={listId}>{[...FONT_PRESETS.map(preset => preset.family).filter(Boolean), ...(installed ?? [])].map(family => <option key={family} value={family} />)}</datalist>
      <div className="font-chips">{FONT_PRESETS.map(preset => <button key={preset.label} type="button" className="font-chip" aria-pressed={value === preset.family} data-tooltip={preset.note}
        style={{ fontFamily: fontStack(preset.family) }} onClick={() => { setText(preset.family); onChange(preset.family); }}>{preset.label}</button>)}</div>
      {value && !available && <small className="font-missing"><Icon name="alert" />“{value}” isn’t installed on this computer, so the default font is used.</small>}
    </div>
  </div>;
}

export default function AppearanceSettings({ appearance, onEdit }: { appearance: Appearance; onEdit: (draft: ThemeDraft) => void }) {
  const { settings, resolved, update, notice } = appearance;
  const [message, setMessage] = useState<{ text: string; tone: 'positive' | 'warning' } | null>(null);
  const [fonts, setFonts] = useState<string[] | null>(null);
  const themes = [...BUILT_IN_THEMES, ...settings.custom];
  const say = (text: string, tone: 'positive' | 'warning' = 'positive') => setMessage({ text, tone });
  const use = (theme: ThemeDefinition, modes: ThemeMode[]) => update(previous => {
    const next = { ...previous, themes: { ...previous.themes } };
    for (const mode of modes) if (theme.variants[mode]) next.themes[mode] = theme.id;
    // Choosing a one-sided theme also switches to the scheme that shows it.
    if (previous.scheme !== 'system' && !theme.variants[previous.scheme]) next.scheme = theme.variants.dark ? 'dark' : 'light';
    return next;
  });
  const edit = (theme: ThemeDefinition, copy: boolean) => onEdit({
    id: copy ? null : theme.id, name: copy ? uniqueThemeName(`${theme.name} copy`, themes.map(item => item.name)) : theme.name,
    variants: structuredClone(theme.variants), mode: theme.variants[resolved.mode] ? resolved.mode : theme.variants.dark ? 'dark' : 'light',
  });
  const importTheme = (text: string): string | null => {
    if (settings.custom.length >= MAX_CUSTOM_THEMES) return `You can keep up to ${MAX_CUSTOM_THEMES} custom themes. Delete one first.`;
    try {
      const imported = parseThemeText(text);
      const theme: ThemeDefinition = { id: newThemeId(), name: uniqueThemeName(imported.name, themes.map(item => item.name)), variants: imported.variants };
      update(previous => ({ ...previous, custom: [...previous.custom, theme] }));
      use(theme, ['dark', 'light']);
      say(`Imported “${theme.name}”${imported.notes.length ? `. ${imported.notes.join(' ')}` : ''}`);
      return null;
    } catch (cause) { return cause instanceof Error ? cause.message : String(cause); }
  };
  const remove = (theme: ThemeDefinition) => {
    update(previous => ({ ...previous, custom: previous.custom.filter(item => item.id !== theme.id),
      themes: { dark: previous.themes.dark === theme.id ? BUILT_IN_THEMES[0].id : previous.themes.dark, light: previous.themes.light === theme.id ? BUILT_IN_THEMES[0].id : previous.themes.light } }));
    say(`Deleted “${theme.name}”.`);
  };
  const setBackdrop = (change: Partial<Settings['backdrop']>) => update(previous => ({ ...previous, backdrop: { ...previous.backdrop, ...change } }));
  const listFonts = () => { void installedFonts().then(list => { if (list) setFonts(list); else say('Installed fonts can’t be listed here. Type any installed font name instead.', 'warning'); }); };
  const schemeColors = (mode: ThemeMode) => themeColors(themes.find(theme => theme.id === settings.themes[mode]), mode);
  const current = themes.find(theme => theme.id === settings.themes[resolved.mode]) ?? BUILT_IN_THEMES[0];

  return <div className="appearance-settings">
    <SettingsBlock id="appearance-scheme" title="Colour scheme">
      <div className="settings-group-body">
        <div className="scheme-tiles" role="group" aria-label="Colour scheme">{([['system', 'System', 'monitor'], ['light', 'Light', 'sun'], ['dark', 'Dark', 'moon']] as [ColorScheme, string, IconName][]).map(([scheme, label, icon]) =>
          <button key={scheme} type="button" className="scheme-tile" aria-pressed={settings.scheme === scheme} onClick={() => update(previous => ({ ...previous, scheme }))}>
            <span className={`scheme-preview${scheme === 'system' ? ' is-split' : ''}`}>
              {scheme === 'system' ? <><ThemeWireframe colors={schemeColors('light')} /><ThemeWireframe colors={schemeColors('dark')} /></> : <ThemeWireframe colors={schemeColors(scheme)} />}
            </span>
            <span className="scheme-label"><Icon name={icon} />{label}{settings.scheme === scheme && <Icon name="check" className="scheme-check" />}</span>
          </button>)}</div>
      </div>
    </SettingsBlock>

    <SettingsBlock id="appearance-themes" title="Themes"
      actions={<><ImportTheme disabled={settings.custom.length >= MAX_CUSTOM_THEMES} onImport={importTheme} />
        <button type="button" className="button small" disabled={settings.custom.length >= MAX_CUSTOM_THEMES} onClick={() => edit(current, true)}><Icon name="plus" />Create theme</button></>}>
      <div className="settings-group-body">
        <p className="field-hint">Click a theme to use it everywhere, or a swatch to use it only in light or dark mode.</p>
        <div className="theme-grid">{themes.map(theme => <ThemeCard key={theme.id} theme={theme} settings={settings}
          onUse={() => use(theme, ['dark', 'light'])} onUseMode={mode => use(theme, [mode])} onDuplicate={() => edit(theme, true)}
          onEdit={theme.builtIn ? undefined : () => edit(theme, false)} onDelete={theme.builtIn ? undefined : () => remove(theme)} onMessage={say} />)}</div>
        {message && <div className={`notice${message.tone === 'warning' ? ' warning' : ''}`} role="status"><Icon name={message.tone === 'warning' ? 'alert' : 'check'} /><span>{message.text}</span></div>}
      </div>
    </SettingsBlock>

    <SettingsBlock id="appearance-backdrop" title="Backdrop">
      <div className="settings-row"><span className="settings-row-text"><strong>Background</strong><small>What shows through the glass behind panels and the sidebar.</small></span>
        <Segmented<BackdropKind> label="Backdrop" value={settings.backdrop.kind} onChange={kind => setBackdrop({ kind })}
          options={[{ value: 'ambient', label: 'Ambient', icon: 'sparkle' }, { value: 'solid', label: 'Solid', icon: 'layers' }, { value: 'image', label: 'Image', icon: 'image' }]} /></div>
      {settings.backdrop.kind === 'image' && <>
        <div className="settings-group-body">
          <BackdropLibrary selected={settings.backdrop.imageId} onSelect={imageId => setBackdrop({ kind: 'image', imageId })}
            onRemoved={id => { if (settings.backdrop.imageId === id) setBackdrop({ imageId: null }); }} />
        </div>
        <SliderRow label="Dim" hint="Darkens or lightens the image toward the canvas colour." value={settings.backdrop.dim} min={LIMITS.dim[0]} max={LIMITS.dim[1]} unit="%" onChange={dim => setBackdrop({ dim })} />
        <SliderRow label="Blur" hint="Softens the image behind the interface." value={settings.backdrop.blur} min={LIMITS.blur[0]} max={LIMITS.blur[1]} unit="px" onChange={blur => setBackdrop({ blur })} />
      </>}
    </SettingsBlock>

    <SettingsBlock id="appearance-interface" title="Interface">
      <SliderRow id="appearance-glass" label="Glass" hint="Panel opacity. At 100% panels are solid and the frosted blur is off." value={settings.glass} min={LIMITS.glass[0]} max={LIMITS.glass[1]} unit="%"
        onChange={glass => update(previous => ({ ...previous, glass }))} />
      <SliderRow id="appearance-radius" label="Corner radius" hint="Rounding of panels, menus and controls." value={settings.radius} min={LIMITS.radius[0]} max={LIMITS.radius[1]} unit="px"
        onChange={radius => update(previous => ({ ...previous, radius }))} />
      <div className="settings-row" id="appearance-motion"><span className="settings-row-text"><strong>Motion</strong><small>Animated transitions. System follows the Windows animation setting.</small></span>
        <Segmented<MotionPreference> label="Motion" value={settings.motion} onChange={motion => update(previous => ({ ...previous, motion }))}
          options={[{ value: 'system', label: 'System' }, { value: 'full', label: 'Full' }, { value: 'reduced', label: 'Reduced' }]} /></div>
    </SettingsBlock>

    <SettingsBlock id="appearance-typography" title="Typography">
      <FontPicker id="appearance-interface-font" label="Interface font" hint="Text, labels and numbers. The system font is the default and Inter is bundled; any installed font can be named." value={settings.fonts.interface} placeholder="System font (default)"
        installed={fonts} onListFonts={listFonts} onChange={family => update(previous => ({ ...previous, fonts: { ...previous.fonts, interface: family } }))} />
      <FontPicker id="appearance-display-font" label="Display font" hint="Headings, country names and widget titles." value={settings.fonts.display} placeholder="Same as interface"
        installed={fonts} onListFonts={listFonts} onChange={family => update(previous => ({ ...previous, fonts: { ...previous.fonts, display: family } }))} />
      <div className="settings-group-body">
        <div className="type-preview" aria-hidden="true">
          <span className="type-preview-heading">German Reich</span>
          <span>Research completed · Improved Machine Tools</span>
          <span className="type-preview-numbers">12 Mar 1939 · 14:00 &nbsp; 0123456789 &nbsp; ≈ 86 d left</span>
        </div>
      </div>
    </SettingsBlock>
    {notice && <p className="settings-notice" role="alert">{notice}</p>}
  </div>;
}
