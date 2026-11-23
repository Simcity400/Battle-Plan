import { useEffect, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent, MutableRefObject } from 'react';
import { Icon } from '../../components/Icons';
import { FailureLine } from '../../components/FailureLine';
import { ConfirmStep } from '../../components/ContextMenu';
import type { Confirmation } from '../../components/ContextMenu';
import { Popover, usePopover } from '../../components/Popover';
import ColorField from './ColorField';
import { THEME_ROLES, contrastWarnings, deriveColors, reseed, seedsOf, themeRoles } from './themes';
import type { ThemeColors, ThemeMode, ThemeRole } from './themes';
import { MAX_THEME_NAME } from './themeFile';
import { ThemeWireframe } from './ThemePreview';
import type { ThemePreview } from './useAppearance';
import { keepLatest } from '../latest';

export interface ThemeSaveFailure { text: string; retry: boolean }

export interface ThemeDraft { id: string | null; name: string; variants: Partial<Record<ThemeMode, ThemeColors>>; mode: ThemeMode; kept?: Partial<Record<ThemeMode, ThemeRole[]>> }

export const draftChanged = (draft: ThemeDraft, original: ThemeDraft) =>
  JSON.stringify([draft.name, draft.variants]) !== JSON.stringify([original.name, original.variants]);

export const discardDraft = (draft: ThemeDraft, detail: string): Confirmation =>
  ({ title: `Discard changes to “${draft.name.trim() || 'this theme'}”?`, detail, action: 'Discard changes' });

export function useConfirmPopover() {
  const menu = usePopover('end');
  const [step, setStep] = useState<{ confirmation: Confirmation; run: () => void } | null>(null);
  const returnTo = useRef<HTMLElement | null>(null);
  const ask = (anchor: HTMLElement | null, confirmation: Confirmation, run: () => void) => {
    returnTo.current = document.activeElement instanceof HTMLElement ? document.activeElement : anchor;
    setStep({ confirmation, run });
    menu.openAt(() => anchor?.isConnected ? anchor.getBoundingClientRect() : null);
  };
  const element = <Popover menu={menu} className="draft-confirm" aria-label={step?.confirmation.title ?? 'Confirm'}
    onToggle={event => {
      if (event.newState !== 'closed') return;
      const target = returnTo.current;
      returnTo.current = null;
      if (target?.isConnected) target.focus({ preventScroll: true });
    }}>
    {step && <ConfirmStep confirmation={step.confirmation} onCancel={menu.close}
      onConfirm={() => { returnTo.current = null; menu.close(); step.run(); }} />}
  </Popover>;
  return { ask, element };
}

const groups = [...new Set(Object.values(THEME_ROLES).map(role => role.group))];
const modeLabel: Record<ThemeMode, string> = { dark: 'Dark', light: 'Light' };
const SEEDS: readonly ThemeRole[] = ['canvas', 'accent', 'glow'];
const listed = (items: string[]) => items.length > 1 ? `${items.slice(0, -1).join(', ')} and ${items.at(-1)}` : items[0] ?? '';

export default function ThemeEditor({ draft: initial, original, onDraft, onSave, onClose, onPreview, focusName, onFocused, escape }: {
  draft: ThemeDraft; original: ThemeDraft; onDraft: (draft: ThemeDraft, typing: boolean) => void; onSave: (draft: ThemeDraft) => ThemeSaveFailure | null; onClose: () => void; onPreview: (preview: ThemePreview | null) => void;
  focusName: boolean; onFocused: () => void; escape: MutableRefObject<((anchor: HTMLElement | null) => void) | null>;
}) {
  const [draft, showDraft] = useState(initial);
  const [error, setError] = useState<ThemeSaveFailure | null>(null);
  const [typos, showTypos] = useState<Partial<Record<string, string>>>({});
  const [pending, showPending] = useState<readonly ThemeRole[]>([]);
  const typing = pending.length > 0 || Object.keys(typos).length > 0;
  const draftNow = useRef(initial), typosNow = useRef(typos), pendingNow = useRef(pending);
  const publish = () => onDraft(draftNow.current, pendingNow.current.length > 0 || Object.keys(typosNow.current).length > 0);
  const setDraft = keepLatest(draftNow, next => { showDraft(next); publish(); });
  const setTypos = keepLatest(typosNow, next => { showTypos(next); publish(); });
  const setPending = keepLatest(pendingNow, next => { showPending(next); publish(); });
  useEffect(() => { publish(); }, [onDraft]);
  const typo = (role: ThemeRole) => ({ fieldId: `theme-colour-${role}`, typed: typos[`${draft.mode}:${role}`],
    onTyped: (text: string | null) => setTypos(previous => { const next = { ...previous }; if (text === null) delete next[`${draft.mode}:${role}`]; else next[`${draft.mode}:${role}`] = text; return next; }),
    onPending: (on: boolean) => setPending(previous => previous.includes(role) === on ? previous : on ? [...previous, role] : previous.filter(item => item !== role)) });
  const [advanced, setAdvanced] = useState(original.id !== null);
  const [filter, setFilter] = useState('');
  const confirm = useConfirmPopover();
  const closeButton = useRef<HTMLButtonElement>(null);
  const nameInput = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (!focusName) return;
    nameInput.current?.focus({ preventScroll: true });
    nameInput.current?.select();
    onFocused();
  }, []);
  const variantButtons = useRef<HTMLDivElement>(null);
  const colors = draft.variants[draft.mode];
  useEffect(() => { onPreview(colors ? { colors, mode: draft.mode } : null); }, [colors, draft.mode, onPreview]);
  useEffect(() => () => onPreview(null), [onPreview]);
  const warnings = useMemo(() => colors ? contrastWarnings(colors) : [], [colors]);
  const kept = draft.kept?.[draft.mode] ?? [];
  const handSet = kept.filter(role => !SEEDS.includes(role));
  const handSetText = listed(handSet.map(role => THEME_ROLES[role].label));
  const one = handSet.length === 1;
  const setColors = (next: ThemeColors, keep: ThemeRole[] = kept) => setDraft(previous => next === previous.variants[previous.mode] ? previous
    : { ...previous, variants: { ...previous.variants, [previous.mode]: next }, kept: { ...previous.kept, [previous.mode]: keep } });
  const setRole = (role: ThemeRole, value: string) => colors && colors[role] !== value && setColors({ ...colors, [role]: value }, kept.includes(role) ? kept : [...kept, role]);
  const setSeed = (seed: 'canvas' | 'accent' | 'glow', value: string) => colors && setColors(reseed(colors, seed, value, kept));
  const deriveAll = (anchor: HTMLElement) => confirm.ask(anchor, { title: 'Derive every colour?', action: 'Derive colours',
    detail: `${handSetText} ${one ? 'is' : 'are'} replaced with ${one ? 'a colour' : 'colours'} derived from canvas, accent and ambient glow.` },
    () => { if (colors) setColors(deriveColors(seedsOf(colors)), kept.filter(role => SEEDS.includes(role))); });
  const addVariant = (mode: ThemeMode) => {
    const other = draft.variants[mode === 'dark' ? 'light' : 'dark'];
    const seeds = mode === 'dark' ? { canvas: '#0b0f16', accent: other?.accent ?? '#5cb2ff', glow: other?.glow } : { canvas: '#eef1f5', accent: other?.accent ?? '#1f6fd1', glow: other?.glow };
    setDraft(previous => ({ ...previous, mode, variants: { ...previous.variants, [mode]: deriveColors(seeds) }, kept: { ...previous.kept, [mode]: [] } }));
  };
  const removeVariant = (anchor: HTMLElement) => {
    const mode = modeLabel[draft.mode].toLowerCase(), other = draft.mode === 'dark' ? 'light' : 'dark';
    const saved = original.id !== null ? original.variants[draft.mode] : undefined;
    const edited = !!saved && JSON.stringify(saved) !== JSON.stringify(colors);
    const detail = !saved ? `Its colours haven’t been saved and are lost. The theme keeps only its ${other} variant.`
      : `${edited ? `Your unsaved changes to the ${mode} colours are lost. ` : ''}When you save, the theme keeps only its ${other} variant. Until then, the saved ${mode} colours stay as they are.`;
    confirm.ask(anchor, { title: `Remove the ${mode} variant?`, action: 'Remove variant', detail }, () => {
      setTypos(typed => Object.fromEntries(Object.entries(typed).filter(([key]) => !key.startsWith(`${draft.mode}:`))));
      setDraft(previous => {
        const variants = { ...previous.variants }, kept = { ...previous.kept };
        delete variants[previous.mode]; delete kept[previous.mode];
        return { ...previous, variants, kept, mode: previous.mode === 'dark' ? 'light' : 'dark' };
      });
      requestAnimationFrame(() => variantButtons.current?.querySelector<HTMLElement>('[aria-pressed="true"]')?.focus());
    });
  };
  const close = (anchor: HTMLElement | null) => {
    if (!typing && !draftChanged(draft, original)) { onClose(); return; }
    confirm.ask(anchor ?? closeButton.current, discardDraft(draft, original.id ? 'Your changes to this theme are lost; the saved theme stays as it was.' : 'This new theme and its colours are lost.'), onClose);
  };
  escape.current = close;
  const save = () => {
    const bad = Object.keys(typos).sort((a, b) => Number(!a.startsWith(`${draft.mode}:`)) - Number(!b.startsWith(`${draft.mode}:`)))[0];
    if (!bad) { setError(onSave({ ...draft, name })); return; }
    const [mode, role] = bad.split(':') as [ThemeMode, ThemeRole];
    if (mode !== draft.mode) setDraft(previous => ({ ...previous, mode }));
    if (!SEEDS.includes(role)) setAdvanced(true);
    if (!`${THEME_ROLES[role].label} ${role}`.toLowerCase().includes(filter.trim().toLowerCase())) setFilter('');
    requestAnimationFrame(() => document.getElementById(`theme-colour-${role}`)?.focus());
  };
  const keys = (event: KeyboardEvent<HTMLElement>) => {
    if (event.defaultPrevented || !(event.target instanceof Node) || !event.currentTarget.contains(event.target)) return;
    if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); close(closeButton.current); }
  };
  const swatches = colors ? [...new Set([colors.accent, colors.glow, colors.positive, colors.warning, colors.error, colors.text, colors.surface, colors.canvas])] : [];
  const name = draft.name.trim();
  const query = filter.trim().toLowerCase();
  const matching = groups.map(group => [group, themeRoles.filter(role => THEME_ROLES[role].group === group && `${THEME_ROLES[role].label} ${role}`.toLowerCase().includes(query))] as const)
    .filter(([, roles]) => roles.length > 0);
  return <aside className="theme-editor" aria-label="Theme editor" onKeyDown={keys}>
    <header className="theme-editor-header">
      <div><span className="group-label">{original.id ? 'Edit theme' : 'New theme'}</span>
        <input ref={nameInput} className="theme-editor-name" aria-label="Theme name" value={draft.name} maxLength={MAX_THEME_NAME} placeholder="Theme name"
          onChange={event => setDraft({ ...draft, name: event.currentTarget.value })} /></div>
      <button ref={closeButton} type="button" className="icon-button" aria-label="Close theme editor" onClick={event => close(event.currentTarget)}><Icon name="close" /></button>
    </header>
    <div className="theme-editor-body">
      <div className="theme-editor-variant">
        <div ref={variantButtons} className="segmented" role="group" aria-label="Variant">
          {(['dark', 'light'] as const).map(mode => <button key={mode} type="button" aria-pressed={draft.mode === mode}
            onClick={() => draft.variants[mode] ? setDraft({ ...draft, mode }) : addVariant(mode)}>
            <Icon name={mode === 'dark' ? 'moon' : 'sun'} />{draft.variants[mode] ? modeLabel[mode] : `Add ${modeLabel[mode].toLowerCase()}`}</button>)}
        </div>
        {draft.variants.dark && draft.variants.light && <button type="button" className="button small quiet" onClick={event => removeVariant(event.currentTarget)}>Remove {modeLabel[draft.mode].toLowerCase()} variant</button>}
      </div>
      {colors && <>
        <div className="theme-editor-preview"><ThemeWireframe colors={colors} /></div>
        <div className="segmented theme-editor-modes" role="group" aria-label="Editing mode">
          <button type="button" aria-pressed={!advanced} onClick={() => setAdvanced(false)}>Simple</button>
          <button type="button" aria-pressed={advanced} onClick={() => setAdvanced(true)}>Advanced</button>
        </div>
        {!advanced ? <div className="theme-editor-group">
          <p className="field-hint">Choose three colours; panels, text and states are derived with readable contrast.</p>
          <ColorField label="Canvas" value={colors.canvas} onChange={value => setSeed('canvas', value)} swatches={swatches} help={THEME_ROLES.canvas.help} {...typo('canvas')} />
          <ColorField label="Accent" value={colors.accent} onChange={value => setSeed('accent', value)} swatches={swatches} help={THEME_ROLES.accent.help} {...typo('accent')} />
          <ColorField label="Ambient glow" value={colors.glow} onChange={value => setSeed('glow', value)} swatches={swatches} help={THEME_ROLES.glow.help} {...typo('glow')} />
          {handSet.length > 0 && <div className="theme-editor-kept"><p className="field-hint">{handSetText} {one ? 'was' : 'were'} set in Advanced and {one ? 'stays' : 'stay'} as {one ? 'it is' : 'they are'}.</p>
            <button type="button" className="button small quiet" onClick={event => deriveAll(event.currentTarget)}><Icon name="refresh" />Derive {one ? 'it' : 'them'} too</button></div>}
        </div> : <>
          <label className="search-field theme-editor-filter"><Icon name="search" /><input type="search" value={filter} placeholder="Filter colours" aria-label="Filter colours" onChange={event => setFilter(event.currentTarget.value)} /></label>
          {matching.map(([group, roles]) => <section className="theme-editor-group" key={group}>
            <h3 className="group-label">{group}</h3>
            {roles.map(role => <ColorField key={role} label={THEME_ROLES[role].label} help={THEME_ROLES[role].help} value={colors[role]} swatches={swatches} onChange={value => setRole(role, value)} {...typo(role)} />)}
          </section>)}
          {matching.length === 0 && <p className="theme-editor-empty" role="status">No matching colours.</p>}
        </>}
        {warnings.length > 0 && <div className="notice warning" role="status"><Icon name="alert" /><div>{warnings.map(warning => <div key={warning}>{warning}</div>)}</div></div>}
      </>}
    </div>
    <footer className="theme-editor-footer">
      {error && <FailureLine message={error.text} onRetry={error.retry ? save : undefined} />}
      <button type="button" className="button quiet" onClick={event => close(event.currentTarget)}>Cancel</button>
      <button type="button" className="button primary" disabled={!name || (!draft.variants.dark && !draft.variants.light)} onClick={save}><Icon name="check" />Save theme</button>
    </footer>
    {confirm.element}
  </aside>;
}
