import { useEffect, useRef, useState } from 'react';
import type { ChangeEvent, ReactNode, Ref } from 'react';
import BrandMark from '../components/BrandMark';
import { CountryFlag } from '../components/CountryFlag';
import { Icon } from '../components/Icons';
import { Segmented } from '../components/Segmented';
import { Select } from '../components/Select';
import { errorMessage } from '../shell/errors';
import * as api from './api';
import type { SettingsPatch } from './api';
import { captureModeFixed, discardTooltip, head, nextLoad, recordsAutomatically, sessionLength, settingsQueue, stopPromptVisible, untilNextMinute, whatToRecord } from './status';
import type { CaptureMode, Head, NextLoad, RecorderSettings, Snapshot } from './status';
import './recorder.css';

export type View = 'main' | 'settings' | 'record';

/** What the panel's controls do. */
export interface PanelActions {
  view(view: View): void;
  /** Stop pressed: show or cancel the Stop prompt. */
  toggleAsk(): void;
  stop(discard: boolean): void;
  record(): void;
  /** Open Battle Plan, on a recording when given. */
  open(recording: string | null): void;
  details(): void;
  folder(): void;
  settings(patch: SettingsPatch): void;
  campaign(campaign: string, choice: { record?: boolean; next?: NextLoad }): void;
  saveFolder(): void;
  userFolder(choose: boolean): void;
}

export interface PanelViewProps {
  snapshot: Snapshot;
  settings: RecorderSettings | null;
  view: View;
  asking: boolean;
  /** A command that just failed. */
  error: string | null;
  now: number;
  actions: PanelActions;
  root?: Ref<HTMLDivElement>;
}

const folderName = (path: string) => path.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || path;

function Switch({ label, checked, onChange, unavailable }: { label: string; checked: boolean; onChange: (checked: boolean) => void; unavailable?: string }) {
  return <label className={`recorder-row${unavailable ? ' is-unavailable' : ''}`}><span>{label}{unavailable && <span className="recorder-hint">{unavailable}</span>}</span>
    <input type="checkbox" role="switch" checked={!unavailable && checked} disabled={!!unavailable}
      onChange={(event: ChangeEvent<HTMLInputElement>) => onChange(event.target.checked)} /></label>;
}

const CAPTURE_MODES: readonly { value: CaptureMode; title: string; text: string; tag?: string }[] = [
  { value: 'full', title: 'Full', text: 'Everything: the map, battle windows, division details and every dashboard.' },
  { value: 'map', title: 'Map', text: 'Campaign map, counters and battle windows; no dashboards or division details.', tag: 'Smaller files' },
];

/** The two capture modes as cards; a running session's mode is shown fixed. */
function CaptureModeChoice({ value, fixed, onChange }: { value: CaptureMode; fixed: boolean; onChange: (mode: CaptureMode) => void }) {
  return <div className={`recorder-modes${fixed ? ' is-fixed' : ''}`} role="radiogroup" aria-label="Capture mode">
    {CAPTURE_MODES.map(mode => <label key={mode.value} className={`recorder-mode${value === mode.value ? ' is-selected' : ''}`}>
      <input type="radio" name="recorder-capture-mode" value={mode.value} checked={value === mode.value} disabled={fixed}
        onChange={() => onChange(mode.value)} />
      <span className="recorder-mode-title">{mode.title}{mode.tag && <span className="recorder-mode-tag">{mode.tag}</span>}</span>
      <span className="recorder-mode-text">{mode.text}</span>
    </label>)}
  </div>;
}

function State({ head: shown }: { head: Head }) {
  return <span className="recorder-state">
    {shown.dot === 'spin' ? <span className="recorder-spin" aria-hidden="true" /> : shown.dot && <span className={`recorder-dot ${shown.dot}`} aria-hidden="true" />}
    {shown.label && <span className="on">{shown.label}</span>}
    {shown.detail && <span>{shown.detail}</span>}
  </span>;
}

/** The panel for one snapshot: every state of the approved design. */
export function PanelView({ snapshot, settings, view, asking, error, now, actions, root }: PanelViewProps) {
  const problem = error ?? snapshot.problem?.text ?? null;
  const notice: ReactNode = problem && <div className="notice warning" role="status">
    <Icon name="alert" /><span>{problem}</span>
    {!error && snapshot.problem && <button type="button" className="recorder-details" data-tooltip={snapshot.problem.detail || undefined}
      onClick={actions.details}>Details</button>}
  </div>;

  if (view !== 'main' && settings) {
    const back = <button type="button" className="icon-button" aria-label="Back" onClick={() => actions.view(view === 'record' ? 'settings' : 'main')}>
      <Icon name="chevron-left" /></button>;
    if (view === 'record') {
      const fixed = captureModeFixed(snapshot);
      // While attached, the mode the session records, not one chosen since.
      const mode = snapshot.captureMode ?? settings.captureMode;
      const fullOnly = mode === 'map' ? 'Full only' : undefined;
      return <div ref={root} className="recorder">
        <div className="recorder-top">{back}What to record</div>
        {error && notice}
        <section className="recorder-section is-first">
          <div className="recorder-label">Capture mode</div>
          <CaptureModeChoice value={mode} fixed={fixed} onChange={captureMode => actions.settings({ captureMode })} />
          <p className="recorder-note"><Icon name="info" /><span>{fixed
            ? 'Fixed while attached to the game, including at its menu. A different mode starts a new recording file.'
            : 'A campaign recorded in the other mode continues in a new recording file.'}</span></p>
        </section>
        <section className="recorder-section is-last">
          <Switch label="Game score" checked={settings.gameScore} onChange={gameScore => actions.settings({ gameScore })} />
          <Switch label="Operation outcomes" checked={settings.operationOutcomes} unavailable={fullOnly}
            onChange={operationOutcomes => actions.settings({ operationOutcomes })} />
          <Switch label="Politics" checked={settings.politics} unavailable={fullOnly} onChange={politics => actions.settings({ politics })} />
        </section>
      </div>;
    }
    const userOptions = [{ value: 'automatic', label: 'Automatic' },
      ...(settings.userFolder ? [{ value: 'custom', label: folderName(settings.userFolder), reason: settings.userFolder }] : []),
      { value: 'choose', label: 'Choose folder…' }];
    return <div ref={root} className="recorder">
      <div className="recorder-top">{back}Settings</div>
      {error && notice}
      <section className="recorder-section is-first">
        <Switch label="Record every campaign" checked={settings.recordEveryCampaign} onChange={recordEveryCampaign => actions.settings({ recordEveryCampaign })} />
        <div className="recorder-row"><span>Save to</span>
          <button type="button" className="recorder-path" data-tooltip={settings.effectiveSaveFolder} onClick={actions.saveFolder}>
            <span>{settings.effectiveSaveFolder}</span><i><Icon name="folder" /></i></button></div>
        <div className="recorder-row"><span>Paradox user folder</span>
          <Select label="Paradox user folder" value={settings.userFolder ? 'custom' : 'automatic'} options={userOptions}
            onChange={value => { if (value !== 'custom') actions.userFolder(value === 'choose'); }} /></div>
      </section>
      <section className="recorder-section">
        <div className="recorder-label">Start</div>
        <Switch label="With Windows" checked={settings.startWithWindows} onChange={startWithWindows => actions.settings({ startWithWindows })} />
        <Switch label="With Battle Plan" checked={settings.startWithReplay} onChange={startWithReplay => actions.settings({ startWithReplay })} />
      </section>
      <section className="recorder-section is-last">
        <button type="button" className="recorder-row recorder-link" onClick={() => actions.view('record')}><span>What to record</span>
          <span className="recorder-value">{whatToRecord(settings)}<Icon name="chevron-right" /></span></button>
      </section>
    </div>;
  }

  const shown = head(snapshot);
  const prompt = stopPromptVisible(asking, snapshot);
  const campaign = snapshot.campaign;
  return <div ref={root} className="recorder">
    <div className="recorder-head">
      {shown.glyph ? <span className="recorder-glyph"><Icon name="gamepad" /></span>
        : shown.flag && <span className="recorder-flag"><CountryFlag image={shown.flag} /></span>}
      <div className="recorder-who"><strong>{shown.title}</strong><State head={shown} /></div>
      {shown.action === 'stop' && <button type="button" className={`button small${asking ? ' is-pressed' : ''}`} aria-pressed={asking}
        onClick={actions.toggleAsk}><span className="recorder-square" aria-hidden="true" />Stop</button>}
      {shown.action === 'record' && <button type="button" className="button small" onClick={actions.record}>
        <span className="recorder-round" aria-hidden="true" />Record</button>}
      {shown.action === 'open' && <button type="button" className="button small primary" onClick={() => actions.open(snapshot.recording)}>Open</button>}
    </div>
    {prompt ? <div className="recorder-ask">
      <span><b>Keep</b> this session's {sessionLength(snapshot.startedAt, now)}?</span>
      <button type="button" className="button small recorder-discard" data-tooltip={discardTooltip(snapshot.continued)} onClick={() => actions.stop(true)}>Discard</button>
      <button type="button" className="button small primary" onClick={() => actions.stop(false)}>Save</button>
    </div> : <>
      {notice}
      {shown.bar !== undefined && <div className={`recorder-bar${shown.bar == null ? ' is-open' : ''}`} role="progressbar" aria-label={shown.label ?? undefined}
        aria-valuemin={0} aria-valuemax={100} aria-valuenow={shown.bar == null ? undefined : Math.round(shown.bar * 100)}>
        <i style={shown.bar == null ? undefined : { width: `${shown.bar * 100}%` }} /></div>}
      {shown.campaign && campaign && settings && <section className="recorder-section">
        <div className="recorder-label">This campaign</div>
        <Switch label="Record automatically" checked={recordsAutomatically(settings, campaign)} onChange={record => actions.campaign(campaign, { record })} />
        <div className="recorder-row"><span>Next time it loads</span>
          <Segmented<NextLoad> label="Next time it loads" value={nextLoad(settings, campaign)}
            options={[{ value: 'continue', label: 'Continue' }, { value: 'newFile', label: 'New file' }]}
            onChange={next => actions.campaign(campaign, { next })} /></div>
      </section>}
      {shown.footer && <footer className="recorder-foot">
        <button type="button" className="button small quiet recorder-brand" onClick={() => actions.open(null)}><BrandMark size={15} />Open Battle Plan</button>
        <span className="recorder-grow" />
        <button type="button" className="icon-button" aria-label="Recordings folder" data-tooltip="Recordings folder" onClick={actions.folder}>
          <Icon name="folder" /></button>
        <button type="button" className="icon-button" aria-label="Settings" data-tooltip="Settings" disabled={!settings} onClick={() => actions.view('settings')}>
          <Icon name="settings" /></button>
      </footer>}
    </>}
  </div>;
}

// Asked once per panel window: the recorder answers whether it opens on the Stop prompt.
let opened: Promise<{ askStop: boolean }> | null = null;

/** The Battle Plan Recorder tray panel. */
export default function RecorderPanel() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [settings, setSettings] = useState<RecorderSettings | null>(null);
  const [view, setView] = useState<View>('main');
  const [asking, setAsking] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [, setMinute] = useState(0);
  const root = useRef<HTMLDivElement>(null);
  const [queue] = useState(() => settingsQueue<RecorderSettings>(api.recorderSettings, setSettings));

  useEffect(() => {
    let active = true;
    const loadSettings = () => {
      void queue.refresh().catch(cause => { if (active) setError(errorMessage(cause)); });
    };
    const listeners = [
      api.onStatus(value => { if (active) setSnapshot(value); }),
      api.onAskStop(() => { if (active) { setView('main'); setAsking(true); } }),
      api.onSettings(() => { if (active) loadSettings(); }),
    ];
    void api.recorderStatus().then(value => { if (active) setSnapshot(value); }, cause => { if (active) setError(errorMessage(cause)); });
    loadSettings();
    void (opened ??= api.panelOpened()).then(({ askStop }) => { if (active && askStop) setAsking(true); }, () => {});
    return () => { active = false; for (const listener of listeners) void listener.then(stop => stop(), () => {}); };
  }, [queue]);

  // The window takes the panel's height and stays above the tray icon.
  const ready = snapshot != null;
  useEffect(() => {
    const element = root.current;
    if (!ready || !element) return;
    const fit = () => { void api.panelFit(Math.ceil(element.getBoundingClientRect().height)).catch(() => {}); };
    fit();
    const observer = new ResizeObserver(fit);
    observer.observe(element);
    return () => observer.disconnect();
  }, [ready]);

  // Esc closes the panel (and with it a pending Stop prompt), after any open menu or tooltip.
  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || event.defaultPrevented || document.querySelector('.select-menu:popover-open')) return;
      void api.closePanel();
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, []);

  // Paused games emit no new hour status, so the Stop prompt needs its own minute timer.
  const prompting = snapshot != null && stopPromptVisible(asking, snapshot);
  const startedAt = snapshot?.startedAt ?? null;
  useEffect(() => {
    if (!prompting) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const schedule = () => {
      timer = setTimeout(() => { setMinute(minute => minute + 1); schedule(); }, untilNextMinute(startedAt, Date.now()));
    };
    schedule();
    return () => clearTimeout(timer);
  }, [prompting, startedAt]);

  if (!snapshot) return <div ref={root} className="recorder" />;
  const run = (work: () => Promise<unknown>) => {
    setError(null);
    void work().catch(cause => setError(errorMessage(cause)));
  };
  const save = (work: () => Promise<RecorderSettings | null>) => run(() => queue.save(work));
  const actions: PanelActions = {
    view: setView,
    // Pressing Stop again cancels the prompt, and a Quit waiting on it.
    toggleAsk: () => {
      if (!asking) { setAsking(true); return; }
      setAsking(false);
      run(api.cancelStopPrompt);
    },
    stop: discard => { setAsking(false); run(() => api.stopRecording(discard)); },
    record: () => run(api.startRecording),
    open: recording => run(() => api.openReplay(recording)),
    details: () => run(api.openDetails),
    folder: () => run(api.openRecordingsFolder),
    settings: patch => save(() => api.updateSettings(patch)),
    campaign: (campaign, choice) => save(() => api.setCampaignChoice(campaign, choice)),
    saveFolder: () => save(api.chooseSaveFolder),
    userFolder: choose => save(() => api.setUserFolder(choose)),
  };
  return <PanelView snapshot={snapshot} settings={settings} view={view} asking={asking} error={error} now={Date.now()} actions={actions} root={root} />;
}
