import { colorLabels, themes } from './theme';
import type { Theme } from './theme';
import { Icon } from './Icons';
import ReplayFolderSetting from './ReplayFolderSetting';
import type { DashboardClient } from './dashboard/client';
import type { ReplayLibraryState } from './dashboard/replayLibrary';
import MapDiagnostics from './MapDiagnostics';
import MapAppearance from './dashboard/MapAppearance';
import GameFolderSettings from './GameFolderSettings';
import type { ContentFolders, DesktopState, ReplayMetadata } from './api';

export default function Settings({ theme, onChange, notice, library, client, native, metadata, folders, folderError, onFoldersChange }: { folders: ContentFolders | null; folderError: string | null; onFoldersChange: (folders: ContentFolders) => void; native: DesktopState | null; metadata: ReplayMetadata | null; library: ReplayLibraryState; client: DashboardClient; theme: Theme; onChange: (theme: Theme) => void; notice: string | null }) {
  return <div className="settings-page">
    <section id="game-content-settings" className="settings-section folders-section" aria-labelledby="folders-heading">
      <div className="settings-section-heading"><h2 id="folders-heading">Folders</h2></div>
      {folders && !folders.contentReady && <p className="folder-setup-notice" role="status">Welcome to Battle Plan. Choose your game installation and Paradox user folders to load the game’s original assets and mods.</p>}
      <div className="folder-list">
        <GameFolderSettings folders={folders} native={native} client={client} onFoldersChange={onFoldersChange} />
        <ReplayFolderSetting compact library={library} client={client} />
      </div>
      {folderError && <p className="map-menu-error" role="alert">{folderError}</p>}
    </section>
    <section className="settings-section global-map-settings" aria-label="Global map settings">
      <MapAppearance key={`${folders?.game ?? ''}:${folders?.user ?? ''}`} global source={null} client={client} userFolder={folders?.user} />
    </section>
    <section className="settings-section" aria-labelledby="preset-heading">
      <div className="settings-section-heading"><div><h2 id="preset-heading">Appearance</h2></div><span className="theme-autosave">{notice ? 'Session only' : 'Saved automatically'}</span></div>
      <div className="theme-presets">{themes.map((preset) => <button type="button" className="theme-preset" key={preset.id}
        aria-pressed={JSON.stringify(theme.colors) === JSON.stringify(preset.colors)} onClick={() => onChange(preset)}>
        <span className="theme-miniature" style={{ background: preset.colors.background, borderColor: preset.colors.border }}>
          <span style={{ background: preset.colors.raised }} /><span style={{ background: preset.colors.surface, borderColor: preset.colors.border }} /><span style={{ background: preset.colors.surface, borderColor: preset.colors.border }} />
          <i style={{ background: preset.colors.accent }} /><i style={{ background: preset.colors.selection }} />
        </span><span>{preset.name}</span>
      </button>)}</div>
      <details className="theme-customizer"><summary>Customize theme</summary>
      <div className="theme-color-grid">{Object.entries(colorLabels).map(([key, label]) => <label className="theme-color-field" key={key}>
        <input type="color" aria-label={label} value={theme.colors[key as keyof Theme['colors']]} onChange={(event) => onChange({ ...theme, id: 'custom', name: 'Custom theme', colors: { ...theme.colors, [key]: event.currentTarget.value } })} />
        <span>{label}<small>{theme.colors[key as keyof Theme['colors']].toUpperCase()}</small></span>
      </label>)}</div>
      <div className="theme-custom-actions"><label className="theme-mode">Control appearance<select value={theme.mode} onChange={(event) => onChange({ ...theme, mode: event.currentTarget.value as Theme['mode'] })}><option value="dark">Dark</option><option value="light">Light</option></select></label>
        <button type="button" className="button quiet" onClick={() => onChange(themes[0])}><Icon name="refresh" />Reset</button></div>
      </details>
    </section>
    <MapDiagnostics native={native} metadata={metadata} />
    {notice && <p className="settings-notice" role="alert">{notice}</p>}
  </div>;
}
