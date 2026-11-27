import { useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import type { CSSProperties } from 'react';
import { SettingsBlock } from '../SettingsBlock';
import { Popover, usePopover } from '../../components/Popover';
import { Icon } from '../../components/Icons';
import { DismissButton, FailureLine } from '../../components/FailureLine';
import { actionFailure } from '../../dashboard/queryError';
import { ConfirmStep, menuKeys } from '../../components/ContextMenu';
import type { IconName } from '../../components/Icons';
import { newId } from '../ids';
import { BUILT_IN_THEMES, THEME_JSON_EXAMPLE, customRoles, themeColors } from './themes';
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

function ThemeCard({ theme, settings, mode: shown, onUse, onUseMode, onEdit, onDuplicate, onDelete, onCopy }: {
  theme: ThemeDefinition; settings: Settings; mode: ThemeMode; onUse: () => void; onUseMode: (mode: ThemeMode) => void;
  onEdit?: () => void; onDuplicate: () => void; onDelete?: () => void; onCopy: () => void;
}) {
  const id = useId();
  const menu = usePopover();
  const [confirming, setConfirming] = useState(false);
  const deleteItem = useRef<HTMLButtonElement>(null);
  const back = () => { setConfirming(false); requestAnimationFrame(() => deleteItem.current?.focus({ preventScroll: true })); };
  const active = (['dark', 'light'] as const).filter(mode => settings.themes[mode] === theme.id);
  const current = settings.themes[shown] === theme.id;
  const fallback = BUILT_IN_THEMES[0].name;
  return <div className="theme-card" data-active={current || undefined}>
    <button type="button" className="theme-card-main" onClick={onUse} aria-label={`Use ${theme.name}`} aria-current={current || undefined}
      data-tooltip-overflow={theme.name}>
      <span className="theme-card-name"><span data-overflow-label>{theme.name}</span></span>
      <span className="theme-card-kind">{theme.builtIn ? 'Built-in' : 'Custom'}</span>
    </button>
    <div className="theme-card-modes">{(['light', 'dark'] as const).map(mode => theme.variants[mode] ? <button key={mode} type="button" className="theme-card-mode" aria-pressed={settings.themes[mode] === theme.id}
      aria-label={`Use ${theme.name} for ${mode} mode`} data-tooltip={`Use for ${mode} mode`} onClick={() => onUseMode(mode)}>
      <ThemeSwatch colors={theme.variants[mode]!} />{settings.themes[mode] === theme.id && <Icon name="check"
        style={{ color: theme.variants[mode]!.text, '--swatch-halo': theme.variants[mode]!.canvas } as CSSProperties} />}</button>
      : <span key={mode} className="theme-card-mode is-missing" aria-label={`No ${mode} variant`}><Icon name={mode === 'dark' ? 'moon' : 'sun'} /></span>)}</div>
    <button ref={menu.trigger} type="button" className="icon-button small theme-card-more" popoverTarget={id} aria-haspopup="menu" aria-expanded={menu.open} aria-label={`${theme.name} actions`}><Icon name="more" /></button>
    <Popover menu={menu} id={id} className={`theme-card-menu${confirming ? ' is-confirming' : ''}`} aria-label={`${theme.name} actions`} role={confirming ? 'dialog' : 'menu'}
      onKeyDownCapture={confirming ? event => { if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); back(); } } : menuKeys} onToggle={event => { if (event.newState === 'closed') setConfirming(false); }}>
      {confirming && onDelete ? <ConfirmStep confirmation={{ title: `Delete “${theme.name}”?`, action: 'Delete theme',
        detail: `${active.length ? `${fallback} replaces it wherever it’s in use. ` : ''}The theme is removed from this device; save it as a file first to keep a copy.` }}
        onCancel={back} onConfirm={() => { menu.close(); onDelete(); }} />
        : <>
          {onEdit && <button type="button" role="menuitem" className="menu-item" data-popover-autofocus onClick={() => { menu.close(); onEdit(); }}><Icon name="edit" /><span className="menu-item-label">Edit</span></button>}
          <button type="button" role="menuitem" className="menu-item" data-popover-autofocus={!onEdit || undefined} onClick={() => { menu.close(); onDuplicate(); }}><Icon name="copy" /><span className="menu-item-label">{onEdit ? 'Duplicate' : 'Duplicate and edit'}</span></button>
          <button type="button" role="menuitem" className="menu-item" onClick={() => { menu.close(); onCopy(); }}><Icon name="link" /><span className="menu-item-label">Copy as JSON</span></button>
          <button type="button" role="menuitem" className="menu-item" onClick={() => { downloadTheme(theme); menu.close(); }}><Icon name="download" /><span className="menu-item-label">Save as file</span></button>
          {onDelete && <hr className="menu-separator" />}
          {onDelete && <button ref={deleteItem} type="button" role="menuitem" className="menu-item is-danger" onClick={() => setConfirming(true)}><Icon name="trash" /><span className="menu-item-label">Delete</span></button>}
        </>}
    </Popover>
  </div>;
}

export type ImportFailure = { text: string; detail?: string; retry?: boolean };

function ImportTheme({ disabled, onImport }: { disabled: boolean; onImport: (text: string) => ImportFailure | null }) {
  const id = useId();
  const menu = usePopover();
  const [text, setText] = useState('');
  const [error, setError] = useState<ImportFailure | null>(null);
  const [dragging, setDragging] = useState(false);
  const file = useRef<HTMLInputElement>(null);
  const attempt = useRef('');
  const reading = useRef(0);
  const run = (value: string) => { reading.current += 1; attempt.current = value; const failure = onImport(value); setError(failure); if (!failure) { setText(''); menu.close(); } };
  const read = (input: File | undefined) => {
    if (!input) return;
    const sequence = ++reading.current;
    if (input.size > 512 * 1024) { setError({ text: 'Theme files must be smaller than 512 KB.' }); return; }
    const current = () => reading.current === sequence && menu.popup.current?.matches(':popover-open');
    void input.text().then(value => { if (current()) run(value); }, () => {
      if (current()) setError({ text: 'This file couldn’t be read.' });
    });
  };
  return <>
    <button ref={menu.trigger} id="appearance-import-theme" type="button" className="button small quiet" disabled={disabled} popoverTarget={id} aria-haspopup="dialog" aria-expanded={menu.open}><Icon name="upload" />Import…</button>
    <Popover menu={menu} id={id} className="theme-import" aria-label="Import a theme" onToggle={event => { if (event.newState === 'closed') { reading.current += 1; setError(null); setDragging(false); } }}>
      <div className="ui-popover-heading"><strong>Import a theme</strong><button type="button" className="icon-button ui-popover-action" aria-label="Close import" onClick={menu.close}><Icon name="close" /></button></div>
      <div className={`theme-dropzone${dragging ? ' is-dragging' : ''}`} onDragOver={event => { event.preventDefault(); setDragging(true); }} onDragLeave={() => setDragging(false)}
        onDrop={event => { event.preventDefault(); setDragging(false); read(event.dataTransfer.files[0]); }}>
        <Icon name="upload" />
        <span>Drop a theme file here, or <button type="button" className="text-button" onClick={() => file.current?.click()}>choose a file</button></span>
        <small>BattlePlan themes and VS Code colour themes (.json)</small>
        <input ref={file} type="file" accept=".json,application/json" hidden onChange={event => { read(event.currentTarget.files?.[0]); event.currentTarget.value = ''; }} />
      </div>
      <textarea rows={5} value={text} spellCheck={false} placeholder={`…or paste theme JSON, e.g. ${THEME_JSON_EXAMPLE}`}
        aria-label="Theme JSON" onChange={event => { reading.current += 1; setText(event.currentTarget.value); setError(null); }} />
      {error && <FailureLine message={error.text} detail={error.detail} onRetry={error.retry ? () => run(attempt.current) : undefined} />}
      <div className="layout-save-actions"><button type="button" className="button primary" disabled={!text.trim()} onClick={() => run(text)}>Import theme</button></div>
    </Popover>
  </>;
}

type BackdropFailure = { text: string; detail?: string; retry?: () => void };
const reason = (cause: unknown) => cause instanceof Error ? cause.message : String(cause);

function useBackdropLibrary(inUse: readonly (string | null)[], onRemoved: (id: string) => void) {
  const [items, setItems] = useState<BackdropImage[] | null>(null);
  const [error, setError] = useState<BackdropFailure | null>(null);
  const [busy, setBusy] = useState(false);
  const [reads, setReads] = useState(0);
  useEffect(() => {
    let current = true;
    void listBackdrops().then(list => { if (current) { setItems(list); setError(null); } }, cause => { if (current) {
      setItems([]); setError({ text: 'Couldn’t read your saved images.', detail: reason(cause), retry: () => setReads(count => count + 1) });
    } });
    return () => { current = false; };
  }, [reads]);
  const add = async (file: File | undefined, onAdded: (id: string) => void) => {
    if (!file) return;
    setBusy(true); setError(null);
    try { const entry = await importBackdrop(file); setItems(previous => [...(previous ?? []), entry]); onAdded(entry.id); }
    catch (cause) {
      const failure = actionFailure('Couldn’t add that image.', reason(cause));
      setError(cause instanceof DOMException ? { text: 'Couldn’t add that image.', detail: reason(cause), retry: () => void add(file, onAdded) } : { ...failure, retry: failure.detail ? () => void add(file, onAdded) : undefined });
    }
    finally { setBusy(false); }
  };
  const remove = (id: string): void => { setError(null); void deleteBackdrop(id).then(() => { setItems(previous => previous?.filter(entry => entry.id !== id) ?? null); onRemoved(id); })
    .catch(cause => setError({ text: 'Couldn’t delete that image.', detail: reason(cause), retry: () => remove(id) })); };
  return { items, error, busy, add, remove, inUse };
}

function RemoveBackdrop({ image, inUse, onRemove }: { image: BackdropImage; inUse: boolean; onRemove: () => void }) {
  const id = useId();
  const menu = usePopover();
  return <>
    <button ref={menu.trigger} type="button" className="icon-button small backdrop-remove" popoverTarget={id} aria-haspopup="dialog" aria-expanded={menu.open}
      aria-label={`Delete ${image.name}`} data-tooltip="Delete"><Icon name="trash" /></button>
    <Popover menu={menu} id={id} className="backdrop-remove-menu" aria-label={`Delete ${image.name}`}>
      <ConfirmStep confirmation={{ title: `Delete “${image.name}”?`, action: 'Delete image',
        detail: `The image is deleted from this device.${inUse ? ' The backdrop that shows it is left without an image.' : ''}` }}
        onCancel={() => menu.close()} onConfirm={() => { menu.close(); onRemove(); }} />
    </Popover>
  </>;
}

function BackdropLibrary({ library: { items, error, busy, add, remove, inUse }, selected, onSelect }: {
  library: ReturnType<typeof useBackdropLibrary>; selected: string | null; onSelect: (id: string) => void;
}) {
  const choices = useRef(0);
  const file = useRef<HTMLInputElement>(null);
  const addButton = useRef<HTMLButtonElement>(null);
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
        <RemoveBackdrop image={item} inUse={inUse.includes(item.id)} onRemove={() => remove(item.id)} />
      </div>)}
      <button ref={addButton} type="button" className={`backdrop-add${busy ? ' is-busy' : ''}`} aria-disabled={busy || undefined} onClick={() => { if (!busy) file.current?.click(); }}>
        {busy ? <span className="spinner" /> : <Icon name="image" />}<span role={busy ? 'status' : undefined}>{busy ? 'Importing…' : 'Add image…'}</span>
      </button>
      <input ref={file} type="file" accept="image/*" hidden tabIndex={-1} onChange={event => { added(event.currentTarget.files?.[0]); event.currentTarget.value = ''; addButton.current?.focus({ preventScroll: true }); }} />
    </div>
    {error && <FailureLine message={error.text} detail={error.detail} onRetry={error.retry} retryDisabled={busy} />}
    {items !== null && <p className="field-hint">Images are resized to at most 2560 pixels and stored on this device only.</p>}
  </>;
}

const BACKDROP_KINDS: readonly { value: BackdropKind; label: string; icon: IconName }[] = [
  { value: 'ambient', label: 'Ambient', icon: 'sparkle' }, { value: 'solid', label: 'Solid', icon: 'layers' }, { value: 'image', label: 'Image', icon: 'image' }];

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
  const { settings, resolved, update, commit } = appearance;
  const [message, setMessage] = useState<{ text: string; tone: 'positive' | 'error'; retry?: () => void } | null>(null);
  useEffect(() => {
    if (message?.tone !== 'positive') return;
    const timer = setTimeout(() => setMessage(null), 8000);
    return () => clearTimeout(timer);
  }, [message]);
  const themes = [...BUILT_IN_THEMES, ...settings.custom];
  const say = (text: string, tone: 'positive' | 'error' = 'positive', retry?: () => void) => setMessage({ text, tone, retry });
  const select = (previous: Settings, theme: ThemeDefinition, modes: ThemeMode[]): Settings => {
    const next = { ...previous, themes: { ...previous.themes } };
    for (const mode of modes) if (theme.variants[mode]) next.themes[mode] = theme.id;
    if (previous.scheme !== 'system' && !theme.variants[previous.scheme]) next.scheme = theme.variants.dark ? 'dark' : 'light';
    return next;
  };
  const use = (theme: ThemeDefinition, modes: ThemeMode[]) => { setMessage(null); update(previous => select(previous, theme, modes)); };
  const full = settings.custom.length >= MAX_CUSTOM_THEMES;
  const limit = `You can keep up to ${MAX_CUSTOM_THEMES} custom themes. Delete one first.`;
  const edit = (theme: ThemeDefinition, copy: boolean) => copy && full ? say(limit, 'error') : (setMessage(null), onEdit({
    id: copy ? null : theme.id, name: copy ? uniqueThemeName(theme.name, themes.map(item => item.name), ' copy') : theme.name,
    variants: structuredClone(theme.variants), mode: theme.variants[resolved.mode] ? resolved.mode : theme.variants.dark ? 'dark' : 'light',
    kept: theme.builtIn ? {} : Object.fromEntries(Object.entries(theme.variants).map(([mode, colors]) => [mode, customRoles(colors)])),
  }));
  const importTheme = (text: string): ImportFailure | null => {
    if (full) return { text: limit };
    let imported: ReturnType<typeof parseThemeText>;
    try { imported = parseThemeText(text); }
    catch (cause) { return actionFailure('Couldn’t import that theme.', cause instanceof Error ? cause.message : String(cause)); }
    const theme: ThemeDefinition = { id: newThemeId(), name: uniqueThemeName(imported.name, themes.map(item => item.name)), variants: imported.variants };
    if (commit(previous => select({ ...previous, custom: [...previous.custom, theme] }, theme, ['dark', 'light']))) return { text: 'Couldn’t save the imported theme on this device.', retry: true };
    say(`Imported “${theme.name}”.${imported.notes.length ? ` ${imported.notes.join(' ')}` : ''}`);
    return null;
  };
  const remove = (theme: ThemeDefinition) => {
    setMessage(null);
    const failed = commit(previous => ({ ...previous, custom: previous.custom.filter(item => item.id !== theme.id),
      themes: { dark: previous.themes.dark === theme.id ? BUILT_IN_THEMES[0].id : previous.themes.dark, light: previous.themes.light === theme.id ? BUILT_IN_THEMES[0].id : previous.themes.light } }));
    if (failed) say(`Couldn’t delete “${theme.name}” on this device.`, 'error', () => remove(theme));
    else say(`Deleted “${theme.name}”.`);
  };
  const copy = (theme: ThemeDefinition) => {
    setMessage(null);
    void navigator.clipboard.writeText(serializeTheme(theme)).then(() => say(`Copied “${theme.name}” as JSON.`),
      () => say('Couldn’t copy to the clipboard. Try again, or save the theme as a file instead.', 'error', () => copy(theme)));
  };
  const setBackdrop = (change: Partial<Settings['backdrop']>) => update(previous => ({ ...previous, backdrop: { ...previous.backdrop, ...change } }));
  const setWorkspaceBackdrop = (change: Partial<Settings['workspaceBackdrop']>) =>
    update(previous => ({ ...previous, workspaceBackdrop: { ...previous.workspaceBackdrop, ...change } }));
  const library = useBackdropLibrary([settings.backdrop.kind === 'image' ? settings.backdrop.imageId : null,
    settings.workspaceBackdrop.kind === 'image' ? settings.workspaceBackdrop.imageId : null], id => update(previous => ({ ...previous,
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
        <button id="appearance-create-theme" type="button" className="button small" disabled={full} onClick={() => edit(current, true)}><Icon name="plus" />Create theme</button></>}>
      <div className="settings-group-body">
        <p className="field-hint">Choose a theme, or set light and dark modes separately with the swatches.</p>
        <div className="theme-grid">{themes.map(theme => <ThemeCard key={theme.id} theme={theme} settings={settings} mode={resolved.mode}
          onUse={() => use(theme, ['dark', 'light'])} onUseMode={mode => use(theme, [mode])} onDuplicate={() => edit(theme, true)}
          onEdit={theme.builtIn ? undefined : () => edit(theme, false)} onDelete={theme.builtIn ? undefined : () => remove(theme)} onCopy={() => copy(theme)} />)}</div>
        {message && (message.tone === 'error' ? <FailureLine message={message.text} onRetry={message.retry} onDismiss={() => setMessage(null)} />
          : <div className="notice theme-result" role="status"><Icon name="check" /><span>{message.text}</span>
            <DismissButton onDismiss={() => setMessage(null)} /></div>)}
      </div>
    </SettingsBlock>

    <SettingsBlock id="appearance-backdrop" title="Backdrop">
      <BackdropFields label="Sidebar and top bar" hint="The main view also uses this backdrop while Main view is set to Same." kinds={BACKDROP_KINDS}
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
      <div className="settings-row" id="appearance-header-flag"><span className="settings-row-text"><strong>Top-bar flag</strong><small>Original game size, or a larger, smoothed flag.</small></span>
        <Segmented<HeaderFlag> label="Top-bar flag" value={settings.headerFlag} onChange={headerFlag => update(previous => ({ ...previous, headerFlag }))}
          options={[{ value: 'native', label: 'Original' }, { value: 'large', label: 'Large' }]} /></div>
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
  </div>;
}
