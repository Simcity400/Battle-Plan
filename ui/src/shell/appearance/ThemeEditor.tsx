import { useEffect, useMemo, useState } from 'react';
import { Icon } from '../../components/Icons';
import ColorField from './ColorField';
import { THEME_ROLES, contrastWarnings, deriveColors, themeRoles } from './themes';
import type { ThemeColors, ThemeMode, ThemeRole } from './themes';
import { MAX_THEME_NAME } from './themeFile';
import { ThemeWireframe } from './ThemePreview';
import type { ThemePreview } from './useAppearance';

export interface ThemeDraft { id: string | null; name: string; variants: Partial<Record<ThemeMode, ThemeColors>>; mode: ThemeMode }

const groups = [...new Set(Object.values(THEME_ROLES).map(role => role.group))];
const modeLabel: Record<ThemeMode, string> = { dark: 'Dark', light: 'Light' };

export default function ThemeEditor({ draft: initial, onSave, onClose, onPreview }: {
  draft: ThemeDraft; onSave: (draft: ThemeDraft) => string | null; onClose: () => void; onPreview: (preview: ThemePreview | null) => void;
}) {
  const [draft, setDraft] = useState(initial);
  const [error, setError] = useState<string | null>(null);
  const [advanced, setAdvanced] = useState(initial.id !== null);
  const [filter, setFilter] = useState('');
  const colors = draft.variants[draft.mode];
  useEffect(() => { onPreview(colors ? { colors, mode: draft.mode } : null); }, [colors, draft.mode, onPreview]);
  useEffect(() => () => onPreview(null), [onPreview]);
  const warnings = useMemo(() => colors ? contrastWarnings(colors) : [], [colors]);
  const setColors = (next: ThemeColors) => setDraft(previous => ({ ...previous, variants: { ...previous.variants, [previous.mode]: next } }));
  const setRole = (role: ThemeRole, value: string) => colors && setColors({ ...colors, [role]: value });
  const setSeed = (seed: 'canvas' | 'accent' | 'glow', value: string) => colors && setColors(deriveColors({ canvas: colors.canvas, accent: colors.accent, glow: colors.glow, [seed]: value }));
  const addVariant = (mode: ThemeMode) => {
    const other = draft.variants[mode === 'dark' ? 'light' : 'dark'];
    const seeds = mode === 'dark' ? { canvas: '#0b0f16', accent: other?.accent ?? '#5cb2ff', glow: other?.glow } : { canvas: '#eef1f5', accent: other?.accent ?? '#1f6fd1', glow: other?.glow };
    setDraft(previous => ({ ...previous, mode, variants: { ...previous.variants, [mode]: deriveColors(seeds) } }));
  };
  const removeVariant = () => setDraft(previous => {
    const variants = { ...previous.variants };
    delete variants[previous.mode];
    return { ...previous, variants, mode: previous.mode === 'dark' ? 'light' : 'dark' };
  });
  const swatches = colors ? [...new Set([colors.accent, colors.glow, colors.positive, colors.warning, colors.error, colors.text, colors.surface, colors.canvas])] : [];
  const name = draft.name.trim();
  const query = filter.trim().toLowerCase();
  return <aside className="theme-editor" aria-label="Theme editor">
    <header className="theme-editor-header">
      <div><span className="eyebrow">{initial.id ? 'Edit theme' : 'New theme'}</span>
        <input className="theme-editor-name" aria-label="Theme name" value={draft.name} maxLength={MAX_THEME_NAME} placeholder="Theme name"
          onChange={event => setDraft({ ...draft, name: event.currentTarget.value })} /></div>
      <button type="button" className="icon-button" aria-label="Close theme editor" onClick={onClose}><Icon name="close" /></button>
    </header>
    <div className="theme-editor-body">
      <div className="theme-editor-variant">
        <div className="segmented" role="group" aria-label="Variant">
          {(['dark', 'light'] as const).map(mode => <button key={mode} type="button" aria-pressed={draft.mode === mode}
            onClick={() => draft.variants[mode] ? setDraft({ ...draft, mode }) : addVariant(mode)}>
            <Icon name={mode === 'dark' ? 'moon' : 'sun'} />{draft.variants[mode] ? modeLabel[mode] : `Add ${modeLabel[mode].toLowerCase()}`}</button>)}
        </div>
        {draft.variants.dark && draft.variants.light && <button type="button" className="button small quiet" onClick={removeVariant}>Remove {modeLabel[draft.mode].toLowerCase()}</button>}
      </div>
      {colors && <>
        <div className="theme-editor-preview"><ThemeWireframe colors={colors} /></div>
        <div className="segmented theme-editor-modes" role="group" aria-label="Editing mode">
          <button type="button" aria-pressed={!advanced} onClick={() => setAdvanced(false)}>Simple</button>
          <button type="button" aria-pressed={advanced} onClick={() => setAdvanced(true)}>Advanced</button>
        </div>
        {!advanced ? <div className="theme-editor-group">
          <p className="field-hint">Choose three colours; panels, text and states are derived with readable contrast. Changing one here replaces advanced edits for this variant.</p>
          <ColorField label="Canvas" value={colors.canvas} onChange={value => setSeed('canvas', value)} swatches={swatches} help={THEME_ROLES.canvas.help} />
          <ColorField label="Accent" value={colors.accent} onChange={value => setSeed('accent', value)} swatches={swatches} help={THEME_ROLES.accent.help} />
          <ColorField label="Ambient glow" value={colors.glow} onChange={value => setSeed('glow', value)} swatches={swatches} help={THEME_ROLES.glow.help} />
        </div> : <>
          <label className="search-field theme-editor-filter"><Icon name="search" /><input value={filter} placeholder="Filter colours" aria-label="Filter colours" onChange={event => setFilter(event.currentTarget.value)} /></label>
          {groups.map(group => {
            const roles = themeRoles.filter(role => THEME_ROLES[role].group === group && `${THEME_ROLES[role].label} ${role}`.toLowerCase().includes(query));
            return roles.length > 0 && <section className="theme-editor-group" key={group}>
              <h3>{group}</h3>
              {roles.map(role => <ColorField key={role} label={THEME_ROLES[role].label} help={THEME_ROLES[role].help} value={colors[role]} swatches={swatches} onChange={value => setRole(role, value)} />)}
            </section>;
          })}
        </>}
        {warnings.length > 0 && <div className="notice warning" role="status"><Icon name="alert" /><div>{warnings.map(warning => <div key={warning}>{warning}</div>)}</div></div>}
      </>}
    </div>
    {error && <p className="map-menu-error" role="alert">{error}</p>}
    <footer className="theme-editor-footer">
      <button type="button" className="button quiet" onClick={onClose}>Cancel</button>
      <button type="button" className="button primary" disabled={!name || (!draft.variants.dark && !draft.variants.light)} onClick={() => setError(onSave({ ...draft, name }))}><Icon name="check" />Save theme</button>
    </footer>
  </aside>;
}
