import { useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import type { CSSProperties, ReactNode } from 'react';
import { Popover, usePopover } from '../../components/Popover';
import { Icon } from '../../components/Icons';
import type { IconName } from '../../components/Icons';
import { newId } from '../ids';
import { BUILT_IN_THEMES, themeColors } from './themes';
import type { ThemeDefinition, ThemeMode } from './themes';
import { parseThemeText, serializeTheme, uniqueThemeName } from './themeFile';
import { CONTRAST_STEP, LIMITS, MAX_CUSTOM_THEMES } from './settings';
import type { AppearanceSettings as Settings, Backdrop, BackdropKind, ChartMotion, PlaybackFrameRate, ColorScheme, CountryColours, HeaderFlag, MotionPreference } from './settings';
import { deleteBackdrop, importBackdrop, listBackdrops } from './backdrops';
import type { BackdropImage } from './backdrops';
import TypographySettings from './TypographySettings';
import { ThemeSwatch, ThemeWireframe } from './ThemePreview';
import { Segmented } from '../../components/Segmented';
import type { Appearance } from './useAppearance';
import type { ThemeDraft } from './ThemeEditor';

/** A titled group of settings: the title sits above one card whose rows are divided. */
export function SettingsBlock({ id, title, description, actions, children, plain = false }: { id?: string; title: string; description?: string; actions?: ReactNode; children: ReactNode; plain?: boolean }) {
  return <section className={`settings-block${plain ? ' settings-block-plain' : ''}`} id={id} aria-label={title}>
    <div className="settings-block-heading"><div><h3>{title}</h3>{description && <p>{description}</p>}</div>{actions && <div className="settings-block-actions">{actions}</div>}</div>
    <div className="settings-group">{children}</div>
  </section>;
}

function SliderRow({ id, label, hint, value, min, max, step, unit, onChange }: { id?: string; label: string; hint: string; value: number; min: number; max: number; step?: number; unit: string; onChange: (value: number) => void }) {
  return <label className="settings-row" id={id}>
    <span className="settings-row-text"><strong>{label}</strong><small>{hint}</small></span>
    <span className="settings-slider"><output>{value}{unit}</output><input type="range" className="slider" min={min} max={max} step={step} value={value} aria-label={label}
      style={{ '--fill': `${(100 * (value - min)) / (max - min)}%` } as CSSProperties} onChange={event => onChange(Number(event.currentTarget.value))} />
      </span>
  </label>;
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

/** The images stored on this device, shared by every backdrop that can show one. */
function useBackdropLibrary(onRemoved: (id: string) => void) {
  const [items, setItems] = useState<BackdropImage[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => { let current = true; void listBackdrops().then(list => { if (current) setItems(list); }, cause => { if (current) { setItems([]); setError(String(cause?.message ?? cause)); } }); return () => { current = false; }; }, []);
  const add = async (file: File | undefined, onAdded: (id: string) => void) => {
    if (!file) return;
    setBusy(true); setError(null);
    try { const entry = await importBackdrop(file); setItems(previous => [...(previous ?? []), entry]); onAdded(entry.id); }
    catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
    finally { setBusy(false); }
  };
  const remove = (id: string) => void deleteBackdrop(id).then(() => { setItems(previous => previous?.filter(entry => entry.id !== id) ?? null); onRemoved(id); })
    .catch(cause => setError(cause instanceof Error ? cause.message : String(cause)));
  return { items, error, busy, add, remove };
}

function BackdropLibrary({ library: { items, error, busy, add, remove }, selected, onSelect }: {
  library: ReturnType<typeof useBackdropLibrary>; selected: string | null; onSelect: (id: string) => void;
}) {
  // An import selects its image only if no other choice was made while it ran, even one back to the same image.
  const choices = useRef(0);
  useLayoutEffect(() => { choices.current += 1; }, [selected]);
  const choose = (id: string) => { choices.current += 1; onSelect(id); };
  const added = (file: File | undefined) => {
    const before = choices.current;
    void add(file, id => { if (choices.current === before) onSelect(id); });
  };
  return <>
    <div className="backdrop-grid">
      {items?.map(item => <div key={item.id} className="backdrop-thumb" data-selected={item.id === selected || undefined}>
        <button type="button" style={{ backgroundImage: `url("${item.thumbnail}")` }} aria-pressed={item.id === selected} aria-label={`Use ${item.name}`} onClick={() => choose(item.id)} />
        {item.id === selected && <span className="backdrop-check"><Icon name="check" /></span>}
        <button type="button" className="icon-button small backdrop-remove" aria-label={`Remove ${item.name}`} data-tooltip="Remove"
          onClick={() => remove(item.id)}><Icon name="trash" /></button>
      </div>)}
      <label className={`backdrop-add${busy ? ' is-busy' : ''}`}>
        <input type="file" accept="image/*" hidden disabled={busy} onChange={event => { added(event.currentTarget.files?.[0]); event.currentTarget.value = ''; }} />
        {busy ? <span className="spinner" /> : <Icon name="image" />}<span>{busy ? 'Importing…' : 'Add image'}</span>
      </label>
    </div>
    {error && <p className="map-menu-error" role="alert">{error}</p>}
    {items !== null && <p className="field-hint">Images are resized to at most 2560 pixels and stored on this device only.</p>}
  </>;
}

const BACKDROP_KINDS: readonly { value: BackdropKind; label: string; icon: IconName }[] = [
  { value: 'ambient', label: 'Ambient', icon: 'sparkle' }, { value: 'solid', label: 'Solid', icon: 'layers' }, { value: 'image', label: 'Image', icon: 'image' }];

/** One backdrop's kind, and for an image its picture, dim and blur. */
function BackdropFields<Kind extends string>({ label, hint, kinds, value, library, onChange }: {
  label: string; hint: string; kinds: readonly { value: Kind; label: string; icon: IconName }[]; value: Backdrop<Kind>;
  library: ReturnType<typeof useBackdropLibrary>; onChange: (change: Partial<Backdrop<Kind>>) => void;
}) {
  return <>
    <div className="settings-row"><span className="settings-row-text"><strong>{label}</strong><small>{hint}</small></span>
      <Segmented<Kind> label={label} value={value.kind} onChange={kind => onChange({ kind })} options={kinds} /></div>
    {value.kind === 'image' && <>
      <div className="settings-group-body">
        <BackdropLibrary library={library} selected={value.imageId} onSelect={imageId => onChange({ imageId })} />
      </div>
      <SliderRow label="Dim" hint="Darkens or lightens the image toward the canvas colour." value={value.dim} min={LIMITS.dim[0]} max={LIMITS.dim[1]} unit="%"
        onChange={dim => onChange({ dim })} />
      <SliderRow label="Blur" hint="Softens the image." value={value.blur} min={LIMITS.blur[0]} max={LIMITS.blur[1]} unit="px"
        onChange={blur => onChange({ blur })} />
    </>}
  </>;
}

export default function AppearanceSettings({ appearance, onEdit, searchTarget }: { appearance: Appearance; searchTarget?: string; onEdit: (draft: ThemeDraft) => void }) {
  const { settings, resolved, update, notice } = appearance;
  const [message, setMessage] = useState<{ text: string; tone: 'positive' | 'warning' } | null>(null);
  const themes = [...BUILT_IN_THEMES, ...settings.custom];
  const say = (text: string, tone: 'positive' | 'warning' = 'positive') => setMessage({ text, tone });
  const use = (theme: ThemeDefinition, modes: ThemeMode[]) => update(previous => {
    const next = { ...previous, themes: { ...previous.themes } };
    for (const mode of modes) if (theme.variants[mode]) next.themes[mode] = theme.id;
    // Choosing a one-sided theme also switches to the scheme that shows it.
    if (previous.scheme !== 'system' && !theme.variants[previous.scheme]) next.scheme = theme.variants.dark ? 'dark' : 'light';
    return next;
  });
  const full = settings.custom.length >= MAX_CUSTOM_THEMES;
  const limit = `You can keep up to ${MAX_CUSTOM_THEMES} custom themes. Delete one first.`;
  const edit = (theme: ThemeDefinition, copy: boolean) => copy && full ? say(limit, 'warning') : onEdit({
    id: copy ? null : theme.id, name: copy ? uniqueThemeName(theme.name, themes.map(item => item.name), ' copy') : theme.name,
    variants: structuredClone(theme.variants), mode: theme.variants[resolved.mode] ? resolved.mode : theme.variants.dark ? 'dark' : 'light',
  });
  const importTheme = (text: string): string | null => {
    if (full) return limit;
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
  const setWorkspaceBackdrop = (change: Partial<Settings['workspaceBackdrop']>) =>
    update(previous => ({ ...previous, workspaceBackdrop: { ...previous.workspaceBackdrop, ...change } }));
  // A removed image leaves every backdrop that showed it.
  const library = useBackdropLibrary(id => update(previous => ({ ...previous,
    backdrop: previous.backdrop.imageId === id ? { ...previous.backdrop, imageId: null } : previous.backdrop,
    workspaceBackdrop: previous.workspaceBackdrop.imageId === id ? { ...previous.workspaceBackdrop, imageId: null } : previous.workspaceBackdrop })));
  const schemeColors = (mode: ThemeMode) => themeColors(themes.find(theme => theme.id === settings.themes[mode]), mode);
  const current = themes.find(theme => theme.id === settings.themes[resolved.mode]) ?? BUILT_IN_THEMES[0];

  return <div className="appearance-settings">
    <SettingsBlock id="appearance-scheme" title="Colour scheme" plain>
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

    <SettingsBlock id="appearance-themes" title="Themes" plain
      actions={<><ImportTheme disabled={full} onImport={importTheme} />
        <button type="button" className="button small" disabled={full} onClick={() => edit(current, true)}><Icon name="plus" />Create theme</button></>}>
      <div className="settings-group-body">
        <p className="field-hint">Choose a theme, or set light and dark modes separately with the swatches.</p>
        <div className="theme-grid">{themes.map(theme => <ThemeCard key={theme.id} theme={theme} settings={settings}
          onUse={() => use(theme, ['dark', 'light'])} onUseMode={mode => use(theme, [mode])} onDuplicate={() => edit(theme, true)}
          onEdit={theme.builtIn ? undefined : () => edit(theme, false)} onDelete={theme.builtIn ? undefined : () => remove(theme)} onMessage={say} />)}</div>
        {message && <div className={`notice${message.tone === 'warning' ? ' warning' : ''}`} role="status"><Icon name={message.tone === 'warning' ? 'alert' : 'check'} /><span>{message.text}</span></div>}
      </div>
    </SettingsBlock>

    <SettingsBlock id="appearance-backdrop" title="Backdrop">
      <BackdropFields label="Sidebar and top bar" hint="Also behind the main view while that uses the same." kinds={BACKDROP_KINDS}
        value={settings.backdrop} library={library} onChange={setBackdrop} />
      <BackdropFields<BackdropKind | 'same'> label="Main view" hint="Behind the widgets and pages." value={settings.workspaceBackdrop} library={library}
        onChange={setWorkspaceBackdrop} kinds={[{ value: 'same', label: 'Same', icon: 'link' }, ...BACKDROP_KINDS]} />
    </SettingsBlock>

    <SettingsBlock id="appearance-interface" title="Interface">
      <SliderRow id="appearance-contrast" label="Contrast" hint="Soften text and borders. 100% uses the theme’s full contrast."
        value={settings.contrast} min={LIMITS.contrast[0]} max={LIMITS.contrast[1]} step={CONTRAST_STEP} unit="%" onChange={contrast => update(previous => ({ ...previous, contrast }))} />
      <SliderRow id="appearance-glass" label="Main view glass" hint="Opacity of widgets and panels. 100% makes them solid." value={settings.glass} min={LIMITS.glass[0]} max={LIMITS.glass[1]} unit="%"
        onChange={glass => update(previous => ({ ...previous, glass }))} />
      <SliderRow id="appearance-chrome-glass" label="Sidebar and top bar glass" hint="Opacity of the sidebar and top bar. 100% makes them solid." value={settings.chromeGlass}
        min={LIMITS.chromeGlass[0]} max={LIMITS.chromeGlass[1]} unit="%" onChange={chromeGlass => update(previous => ({ ...previous, chromeGlass }))} />
      <SliderRow id="appearance-radius" label="Corner radius" hint="Rounding of panels, menus and controls." value={settings.radius} min={LIMITS.radius[0]} max={LIMITS.radius[1]} unit="px"
        onChange={radius => update(previous => ({ ...previous, radius }))} />
      <div className="settings-row" id="appearance-motion"><span className="settings-row-text"><strong>Motion</strong><small>Interface animations. System follows your device’s preference.</small></span>
        <Segmented<MotionPreference> label="Motion" value={settings.motion} onChange={motion => update(previous => ({ ...previous, motion }))}
          options={[{ value: 'system', label: 'System' }, { value: 'full', label: 'Full' }, { value: 'reduced', label: 'Reduced' }]} /></div>
      <div className="settings-row" id="appearance-header-flag"><span className="settings-row-text"><strong>Top-bar flag</strong><small>Original game size or a larger, smoothed flag.</small></span>
        <Segmented<HeaderFlag> label="Top-bar flag" value={settings.headerFlag} onChange={headerFlag => update(previous => ({ ...previous, headerFlag }))}
          options={[{ value: 'native', label: 'Native' }, { value: 'large', label: 'Large' }]} /></div>
    </SettingsBlock>

    <SettingsBlock id="appearance-charts" title="Charts">
      <div className="settings-row" id="appearance-playback-rate"><span className="settings-row-text"><strong>Playback frame rate</strong><small>Uncapped follows the display refresh rate.</small></span>
        <Segmented<PlaybackFrameRate> label="Playback frame rate" value={settings.charts.frameRate}
          onChange={frameRate => update(previous => ({ ...previous, charts: { ...previous.charts, frameRate } }))}
          options={[{ value: '30', label: '30' }, { value: '60', label: '60' }, { value: '120', label: '120' }, { value: 'uncapped', label: 'Uncapped' }]} /></div>
      <div className="settings-row" id="appearance-chart-motion"><span className="settings-row-text"><strong>Chart motion</strong><small>How charts transition between values during playback.</small></span>
        <Segmented<ChartMotion> label="Chart motion" value={settings.charts.motion} onChange={motion => update(previous => ({ ...previous, charts: { ...previous.charts, motion } }))}
          options={[{ value: 'snappy', label: 'Snappy' }, { value: 'smooth', label: 'Smooth' }, { value: 'cinematic', label: 'Cinematic' }]} /></div>
      <div className="settings-row" id="appearance-country-colours"><span className="settings-row-text"><strong>Country colours</strong><small>Recorded map colours, or adjusted for readability on this theme.</small></span>
        <Segmented<CountryColours> label="Country colours" value={settings.charts.countryColours} onChange={countryColours => update(previous => ({ ...previous, charts: { ...previous.charts, countryColours } }))}
          options={[{ value: 'raw', label: 'Raw' }, { value: 'fixed', label: 'Adjusted' }]} /></div>
    </SettingsBlock>

    <TypographySettings appearance={appearance} searchTarget={searchTarget} />
    {notice && <p className="settings-notice" role="alert">{notice}</p>}
  </div>;
}
