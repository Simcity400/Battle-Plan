import { useEffect, useRef, useState } from 'react';
import type { ChangeEvent, ReactNode, Ref } from 'react';
import BrandMark from '../components/BrandMark';
import { CountryFlag } from '../components/CountryFlag';
import { Icon } from '../components/Icons';
import { Segmented } from '../components/Segmented';
import { Select } from '../components/Select';
import { recordingDisplayName, recordingFilename } from '../dashboard/recordingNames';
import { errorMessage } from '../shell/errors';
import { newId } from '../shell/ids';
import * as api from './api';
import type { SettingsPatch } from './api';
import { captureModeFixed, compressOffered, compressText, dateText, discardConsequence, discardDetail, followStatus, head, nextLoad, promptTitle, recordingName, recordsAutomatically, sessionLength,
  settingsQueue, sizeText, spanText, spoolTotalText, stopPromptVisible, stopQuestion, untilNextMinute, waitingDiscardQuestion, whatToRecord } from './status.ts';
import type { CaptureMode, FlagImage, Head, NextLoad, RecorderSettings, Recordings, SavedRecording, Snapshot, UnsavedRecording } from './status';
import { AUTO_COMPRESS_NOTE, COMPRESSION_NOTE } from './compression';
import type { Compression } from './compression';
import { setUpdateChannel, updatesAvailable } from '../update/api';
import type { UpdateChannel, UpdateState } from '../update/api';
import { CHANNEL_LABELS, channelDescription, updateAction, updateButtonLabel, updateDescription, updateDisabled, updateTooltip } from '../update/logic';
import { UpdateIcon, useUpdateActions } from '../update/UpdateControl';
import { useUpdateState } from '../update/useUpdateState';
import './recorder.css';

export type View = 'main' | 'settings' | 'record' | 'recordings';

export interface PanelActions {
  view(view: View): void;
  toggleAsk(): void;
  askDiscard(): void;
  stop(discard: boolean): void;
  record(): void;
  open(recording: string | null): void;
  details(): void;
  folder(): void;
  settings(patch: SettingsPatch): void;
  campaign(campaign: string, choice: { record?: boolean; next?: NextLoad }): void;
  saveFolder(): void;
  userFolder(choose: boolean): void;
  waiting(recording: UnsavedRecording, discard: boolean): void;
  askDiscardWaiting(recording: UnsavedRecording | null): void;
  reveal(path: string): void;
  compress(path: string): void;
  dismiss(): void;
}

export interface PanelViewProps {
  snapshot: Snapshot;
  settings: RecorderSettings | null;
  view: View;
  asking: boolean;
  error: string | null;
  now: number;
  actions: PanelActions;
  root?: Ref<HTMLDivElement>;
  compact?: boolean;
  recordings?: Recordings | null;
  quitting?: boolean;
  discarding?: boolean;
  confirmingWaiting?: string | null;
  update?: PanelUpdate;
}

export interface PanelUpdate {
  state: UpdateState | null;
  ask: 'confirm' | 'downloaded' | null;
  pending: boolean;
  press(): void;
  restart(): void;
  dismiss(): void;
  channel(channel: UpdateChannel): void;
}

const waitingKey = (row: UnsavedRecording) => `${row.session}/${row.campaign}`;
const folderName = (path: string) => path.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || path;
const joined = (parts: readonly (string | null)[]) => parts.filter(part => part).join(' · ');

function Switch({ label, checked, onChange }: { label: string; checked: boolean; onChange: (checked: boolean) => void }) {
  return <label className="recorder-row"><span>{label}</span>
    <input type="checkbox" role="switch" checked={checked} onChange={(event: ChangeEvent<HTMLInputElement>) => onChange(event.target.checked)} /></label>;
}

const CAPTURE_MODES: readonly { value: CaptureMode; title: string; text: string; tag?: string }[] = [
  { value: 'full', title: 'Full', text: 'Everything: the map, battle windows, division details and every dashboard.' },
  { value: 'map', title: 'Map', text: 'Campaign map, counters and battle windows; no dashboards or division details.', tag: 'Smaller files' },
];

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

function Toggle({ label, description, checked, onChange }: { label: string; description: string; checked: boolean; onChange: (checked: boolean) => void }) {
  return <label className="recorder-row recorder-toggle"><span className="recorder-toggle-text"><span>{label}</span><small>{description}</small></span>
    <input type="checkbox" role="switch" checked={checked} onChange={(event: ChangeEvent<HTMLInputElement>) => onChange(event.target.checked)} /></label>;
}

function State({ head: shown }: { head: Head }) {
  return <span className="recorder-state">
    {shown.dot === 'spin' ? <span className="recorder-spin" aria-hidden="true" /> : shown.dot && <span className={`recorder-dot ${shown.dot}`} aria-hidden="true" />}
    {shown.label && <span className="on">{shown.label}</span>}
    {shown.detail && <span>{shown.detail}</span>}
  </span>;
}

function ItemFlag({ flag }: { flag: FlagImage | null }) {
  return flag ? <span className="recorder-item-flag"><CountryFlag image={flag} /></span> : <span className="recorder-item-flag is-empty" aria-hidden="true" />;
}

const waitingCount = (recordings: Recordings | null | undefined) => recordings?.unsaved.filter(row => row.status !== 'recording').length ?? 0;

export function PanelView({ snapshot, settings, view, asking, error, now, actions, root, compact = false, recordings = null, quitting = false,
  discarding = false, confirmingWaiting = null, update }: PanelViewProps) {
  const problem = error ?? snapshot.problem?.text ?? null;
  const notice: ReactNode = problem && <div className="notice warning" role="status">
    <Icon name="alert" /><span>{problem}</span>
    {!error && snapshot.problem && <button type="button" className="recorder-details" data-tooltip={snapshot.problem.detail || undefined}
      onClick={actions.details}>Details</button>}
    {error && <button type="button" className="icon-button recorder-dismiss" aria-label="Dismiss" data-tooltip="Dismiss" onClick={actions.dismiss}>
      <Icon name="close" /></button>}
  </div>;
  const buildNotice: ReactNode = snapshot.buildWarning && <div className="notice warning" role="status">
    <Icon name="alert" /><span>{snapshot.buildWarning}</span></div>;
  const preparation = snapshot.preparation;
  const preparationNotice: ReactNode = preparation && (preparation.failed
    ? <div className="notice warning" role="status"><Icon name="alert" /><span>{preparation.text}</span></div>
    : <p className="recorder-note" role="status"><Icon name="info" /><span>{preparation.text}</span></p>);
  const shown = head(snapshot);
  const prompt = stopPromptVisible(asking, snapshot);
  const length = sessionLength(snapshot.startedAt, now);
  const discard = <button key="discard" type="button" className="button small recorder-discard" onClick={actions.askDiscard}>Discard</button>;
  const confirm = (tooltip?: string) => <>
    <button key="cancel-discard" type="button" className="button small quiet" autoFocus onClick={actions.toggleAsk}>Cancel</button>
    <button key="confirm-discard" type="button" className="button small is-danger" data-tooltip={tooltip} onClick={() => actions.stop(true)}>Discard</button>
  </>;

  if (compact) return <div ref={root} className="recorder recorder-compact">
    {prompt && <div className="recorder-prompt" role="alertdialog" aria-labelledby="recorder-prompt-title" aria-describedby="recorder-prompt-question">
      <strong id="recorder-prompt-title">{promptTitle(quitting, discarding)}</strong>
      <p id="recorder-prompt-question">{discarding ? discardDetail(snapshot.name, snapshot.startedAt, now, snapshot.continued)
        : stopQuestion(snapshot.name, snapshot.startedAt, now)}</p>
      <div className="recorder-prompt-actions">{discarding ? confirm() : <>
        <button key="cancel" type="button" className="button small quiet" onClick={actions.toggleAsk}>Cancel</button>
        {discard}
        <button key="save" type="button" className="button small primary" autoFocus onClick={() => actions.stop(false)}>Save</button>
      </>}</div>
    </div>}
  </div>;

  const waiting = waitingCount(recordings);
  const footer = (more: ReactNode = null) => <footer className="recorder-foot">
    <button type="button" className="button small quiet recorder-brand" onClick={() => actions.open(null)}><BrandMark size={15} />Open Battle Plan</button>
    <span className="recorder-grow" />
    {more}
  </footer>;
  const back = (to: View) => <button type="button" className="icon-button" aria-label="Back" onClick={() => actions.view(to)}><Icon name="chevron-left" /></button>;

  if (view === 'recordings') {
    const unsaved = recordings?.unsaved ?? [];
    const saved = recordings?.saved ?? [];
    const total = recordings && spoolTotalText(unsaved);
    return <div ref={root} className="recorder">
      <div className="recorder-top" data-tauri-drag-region="deep">{back('main')}Recordings</div>
      {error && notice}
      <div className="recorder-list themed-scrollbar">
        <section className="recorder-section is-first" aria-label="Not saved yet">
          <div className="recorder-label recorder-label-row"><span>Not saved yet</span>{total && <span className="recorder-total">{total}</span>}</div>
          {recordings && (unsaved.length ? unsaved.map(row => <UnsavedRow key={waitingKey(row)} row={row} actions={actions}
            confirming={confirmingWaiting === waitingKey(row)} />)
            : <p className="recorder-empty">Nothing is waiting to be saved.</p>)}
        </section>
        <section className="recorder-section is-last" aria-label="Saved">
          <div className="recorder-label">Saved</div>
          {recordings && (saved.length ? saved.map(row => <SavedRow key={row.path} row={row} settings={settings} actions={actions} />)
            : <p className="recorder-empty">No saved recordings yet.</p>)}
        </section>
      </div>
      {footer(<button type="button" className="icon-button" aria-label="Open recordings folder" data-tooltip="Open recordings folder"
        onClick={actions.folder}><Icon name="folder" /></button>)}
    </div>;
  }

  if (view !== 'main' && settings) {
    if (view === 'record') {
      const fixed = captureModeFixed(snapshot);
      const mode = snapshot.captureMode ?? settings.captureMode;
      return <div ref={root} className="recorder">
        <div className="recorder-top" data-tauri-drag-region="deep">{back('settings')}What to record</div>
        {error && notice}
        <section className="recorder-section is-first is-last">
          <div className="recorder-label">Capture mode</div>
          <CaptureModeChoice value={mode} fixed={fixed} onChange={captureMode => actions.settings({ captureMode })} />
          <p className="recorder-note"><Icon name="info" /><span>{fixed
            ? 'Fixed while attached to the game, including at its menu. A different mode starts a new recording file.'
            : 'A campaign recorded in the other mode continues in a new recording file.'}</span></p>
        </section>
        {footer()}
      </div>;
    }
    const userOptions = [{ value: 'automatic', label: 'Automatic' },
      ...(settings.userFolder ? [{ value: 'custom', label: folderName(settings.userFolder), reason: settings.userFolder }] : []),
      { value: 'choose', label: 'Choose folder…' }];
    return <div ref={root} className="recorder">
      <div className="recorder-top" data-tauri-drag-region="deep">{back('main')}Settings</div>
      {error && notice}
      <section className="recorder-section is-first">
        <div className="recorder-label">Recording</div>
        <Toggle label="Auto record" description="Records each campaign when it loads." checked={settings.recordEveryCampaign}
          onChange={recordEveryCampaign => actions.settings({ recordEveryCampaign })} />
        <Toggle label="Auto save" description="Saves when you load another campaign or close the game." checked={settings.autoSave}
          onChange={autoSave => actions.settings({ autoSave })} />
        <button type="button" className="recorder-row recorder-link" onClick={() => actions.view('record')}><span>What to record</span>
          <span className="recorder-value">{whatToRecord(settings)}<Icon name="chevron-right" /></span></button>
        <div className="recorder-row"><span>Compression</span>
          <Segmented<Compression> label="Compression" value={settings.liveCompression}
            options={[{ value: 'low', label: 'Low' }, { value: 'high', label: 'High' }]} onChange={compression => actions.settings({ compression })} /></div>
        <p className="recorder-note">{COMPRESSION_NOTE}</p>
        {settings.liveCompression === 'low' && <Toggle label="Auto Compress" description={AUTO_COMPRESS_NOTE} checked={settings.autoCompress}
          onChange={autoCompress => actions.settings({ autoCompress })} />}
      </section>
      <section className="recorder-section">
        <div className="recorder-label">Folders</div>
        <div className="recorder-row"><span>Save to</span>
          <button type="button" className="recorder-path" data-tooltip={settings.effectiveSaveFolder} onClick={actions.saveFolder}>
            <span>{settings.effectiveSaveFolder}</span><i><Icon name="folder" /></i></button></div>
        <div className="recorder-row"><span>Paradox user folder</span>
          <Select label="Paradox user folder" value={settings.userFolder ? 'custom' : 'automatic'} options={userOptions}
            onChange={value => { if (value !== 'custom') actions.userFolder(value === 'choose'); }} /></div>
      </section>
      <section className={`recorder-section${update ? '' : ' is-last'}`}>
        <div className="recorder-label">Start automatically</div>
        <Switch label="With Windows" checked={settings.startWithWindows} onChange={startWithWindows => actions.settings({ startWithWindows })} />
        <Switch label="With Battle Plan" checked={settings.startWithReplay} onChange={startWithReplay => actions.settings({ startWithReplay })} />
      </section>
      {update && <UpdateSection update={update} />}
      {footer()}
    </div>;
  }

  const campaign = snapshot.campaign;
  return <div ref={root} className="recorder">
    <div className="recorder-head" data-tauri-drag-region="deep">
      {shown.glyph ? <span className="recorder-glyph"><Icon name="gamepad" /></span>
        : shown.flag && <span className="recorder-flag"><CountryFlag image={shown.flag} /></span>}
      <div className="recorder-who"><strong data-tooltip={snapshot.recording ? recordingFilename(snapshot.recording) : undefined}>
        {shown.title}</strong><State head={shown} />{shown.sizes && <span className="recorder-sizes">{shown.sizes}</span>}</div>
      {shown.action === 'stop' && <button type="button" className={`button small${asking ? ' is-pressed' : ''}`} aria-pressed={asking}
        onClick={actions.toggleAsk}><span className="recorder-square" aria-hidden="true" />Stop</button>}
      {shown.action === 'record' && <button type="button" className="button small" onClick={actions.record}>
        <span className="recorder-round" aria-hidden="true" />Record</button>}
      {shown.action === 'open' && <button type="button" className="button small primary" onClick={() => actions.open(snapshot.recording)}>Open</button>}
      {shown.action === 'recordings' && <button type="button" className="button small" onClick={() => actions.view('recordings')}>Recordings</button>}
    </div>
    {prompt ? <div className="recorder-ask">{discarding ? <>
      <span><b>Discard</b> {quitting ? `${length} and quit` : `this session's ${length}`}?</span>
      {confirm(discardConsequence(snapshot.continued))}
    </> : <>
      <span><b>Keep</b> {quitting ? `${length} before quitting` : `this session's ${length}`}?</span>
      {discard}
      <button key="save" type="button" className="button small primary" onClick={() => actions.stop(false)}>Save</button>
    </>}</div> : <>
      {notice}
      {buildNotice}
      {preparationNotice}
      {shown.busy && <div className="recorder-bar" role="progressbar" aria-label={shown.label ?? undefined}><i /></div>}
      {shown.campaign && campaign && settings && <section className="recorder-section">
        <div className="recorder-label">This campaign</div>
        <Switch label="Record automatically" checked={recordsAutomatically(settings, campaign)} onChange={record => actions.campaign(campaign, { record })} />
        <div className="recorder-row"><span>Next time it loads</span>
          <Segmented<NextLoad> label="Next time it loads" value={nextLoad(settings, campaign)}
            options={[{ value: 'continue', label: 'Continue' }, { value: 'newFile', label: 'New file' }]}
            onChange={next => actions.campaign(campaign, { next })} /></div>
      </section>}
    </>}
    {update?.ask && update.state && <UpdateAsk update={update} />}
    {footer(<>
      {update && <button type="button" className="update-button" aria-label={updateTooltip(update.state)} data-tooltip={updateTooltip(update.state)}
        aria-disabled={updateDisabled(update.state) || update.pending || undefined} onClick={update.press}><UpdateIcon state={update.state} /></button>}
      <button type="button" className="button small quiet recorder-recordings" aria-label={waiting ? `Recordings, ${waiting} waiting to be saved` : undefined}
        onClick={() => actions.view('recordings')}>Recordings{waiting > 0 && <span className="recorder-badge" aria-hidden="true">{waiting}</span>}</button>
      <button type="button" className="icon-button" aria-label="Settings" data-tooltip="Settings" disabled={!settings} onClick={() => actions.view('settings')}>
        <Icon name="settings" /></button>
    </>)}
  </div>;
}

function UpdateAsk({ update }: { update: PanelUpdate }) {
  const state = update.state!;
  const version = state.downloadedVersion ?? state.availableVersion ?? '';
  const blocked = state.installBlocked;
  return <div className="recorder-ask recorder-update-ask" role="alertdialog" aria-label="Update">
    <span data-tooltip={blocked ?? 'Battle Plan and Battle Plan Recorder close and reopen on the new version.'}>
      {update.ask === 'downloaded' ? <><b>Update {version}</b> downloaded</> : <><b>Restart</b> to update to {version}?</>}</span>
    <button key="later" type="button" className="button small quiet" onClick={update.dismiss}>{update.ask === 'downloaded' ? 'Later' : 'Cancel'}</button>
    <button key="restart" type="button" className="button small primary" autoFocus disabled={blocked != null || update.pending}
      data-tooltip={blocked ?? undefined} onClick={update.restart}>Restart</button>
    {blocked && <p className="recorder-update-blocked" role="status">{blocked}</p>}
  </div>;
}

function UpdateSection({ update }: { update: PanelUpdate }) {
  const { state } = update;
  return <section className="recorder-section is-last">
    <div className="recorder-label">Updates</div>
    <div className="recorder-row"><span>Battle Plan Recorder {state?.currentVersion ?? ''}</span>
      <button type="button" className={`button small${updateAction(state) === 'install' ? ' primary' : ''}`} disabled={updateDisabled(state) || update.pending}
        data-tooltip={updateTooltip(state)} onClick={update.press}>{updateButtonLabel(state)}</button></div>
    <div className="recorder-row"><span>Channel</span>
      {state?.enabled ? <Segmented<UpdateChannel> label="Update channel" value={state.channel}
        options={(['stable', 'nightly'] as const).map(value => ({ value, label: CHANNEL_LABELS[value] }))} onChange={update.channel} />
        : <span className="recorder-value">{state ? CHANNEL_LABELS[state.channel] : ''}</span>}</div>
    <p className="recorder-note">{updateDescription(state)} {channelDescription(state)}</p>
  </section>;
}

function UnsavedRow({ row, actions, confirming }: { row: UnsavedRecording; actions: PanelActions; confirming: boolean }) {
  const status = row.status === 'recording' ? 'Recording' : row.status === 'saving' ? 'Saving…' : 'Waiting';
  const when = row.status === 'recording' ? dateText(row.lastHour) : spanText(row.firstHour, row.lastHour);
  const asking = confirming && row.status === 'waiting';
  return <div className="recorder-item">
    <ItemFlag flag={row.flag} />
    <span className="recorder-item-name" data-tooltip={row.recording ? recordingFilename(row.recording) : undefined}>{recordingName(row.name)}</span>
    {asking ? <span className="recorder-item-meta is-warning" role="status">{waitingDiscardQuestion(row.recording != null)}</span>
      : <span className="recorder-item-meta"><span className={`recorder-dot ${row.status === 'recording' ? 'red' : 'grey'}`} aria-hidden="true" />
        <span>{joined([status, when, sizeText(row.spoolBytes)])}</span></span>}
    {row.status === 'waiting' && <span className="recorder-item-actions">{asking ? <>
      <button key="cancel-discard" type="button" className="button small quiet recorder-item-action" autoFocus
        onClick={() => actions.askDiscardWaiting(null)}>Cancel</button>
      <button key="confirm-discard" type="button" className="button small is-danger recorder-item-action" onClick={() => actions.waiting(row, true)}>Discard</button>
    </> : <>
      <button key="discard" type="button" className="button small recorder-discard recorder-item-action" onClick={() => actions.askDiscardWaiting(row)}>Discard</button>
      <button key="save" type="button" className="button small recorder-item-action" onClick={() => actions.waiting(row, false)}>Save</button>
    </>}</span>}
  </div>;
}

function SavedRow({ row, settings, actions }: { row: SavedRecording; settings: RecorderSettings | null; actions: PanelActions }) {
  const kept = row.job?.state === 'kept' || row.job?.state === 'unconfirmed';
  return <div className="recorder-item is-openable">
    <ItemFlag flag={row.flag} />
    <button type="button" className="recorder-item-name recorder-item-open" data-tooltip={`Open in Battle Plan · ${recordingFilename(row.path)}`}
      onClick={() => actions.open(row.path)}>{recordingDisplayName(row.path)}</button>
    <span className={`recorder-item-meta${kept ? ' is-warning' : ''}`} role={row.job ? 'status' : undefined}>
      <span>{joined([spanText(row.firstHour, row.lastHour), sizeText(row.bytes), compressText(row.job)])}</span></span>
    <span className="recorder-item-actions">
      {compressOffered(settings, row) && <button type="button" className="button small recorder-item-action"
        data-tooltip="Compress to the smaller High size in the background" onClick={() => actions.compress(row.path)}>Compress</button>}
      <button type="button" className="icon-button" aria-label="Show in folder" data-tooltip="Show in folder"
        onClick={() => actions.reveal(row.path)}><Icon name="folder-open" /></button></span>
  </div>;
}

export default function RecorderPanel() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [settings, setSettings] = useState<RecorderSettings | null>(null);
  const [recordings, setRecordings] = useState<Recordings | null>(null);
  const [view, setView] = useState<View>('main');
  const [asking, setAsking] = useState(false);
  const [quitting, setQuitting] = useState(false);
  const [discarding, setDiscarding] = useState(false);
  const [confirmingWaiting, setConfirmingWaiting] = useState<string | null>(null);
  const [compact, setCompact] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [, setMinute] = useState(0);
  const root = useRef<HTMLDivElement>(null);
  const [panel] = useState(newId);
  const visit = useRef({ number: 0, open: false });
  const opened = useRef<ReturnType<typeof api.panelOpened> | null>(null);
  const [queue] = useState(() => settingsQueue<RecorderSettings>(api.recorderSettings, setSettings));
  const { state: updateState } = useUpdateState();
  const updates = useUpdateActions();
  const [updateAsk, setUpdateAsk] = useState<{ kind: 'confirm' | 'downloaded'; version: string } | null>(null);
  useEffect(() => { if (updates.failure) setError(`${updates.failure.title}: ${updates.failure.detail}`); }, [updates.failure]);

  useEffect(() => {
    let active = true;
    const loadSettings = () => {
      void queue.refresh().catch(cause => { if (active) setError(errorMessage(cause)); });
    };
    const status = followStatus(api.onStatus, api.recorderStatus, value => { if (active) setSnapshot(value); });
    const listeners = [
      status.listening,
      api.onAskStop(ask => {
        if (!active) return;
        setView('main');
        setAsking(true);
        if (ask.quitting) setQuitting(true);
        if (ask.discarding) setDiscarding(true);
      }),
      api.onSettings(() => { if (active) loadSettings(); }),
    ];
    void status.first.catch(cause => { if (active) setError(errorMessage(cause)); });
    loadSettings();
    void (opened.current ??= api.panelOpened(panel)).then(({ askStop, compact: alone, quitting: quit, discarding: discard }) => {
      if (!active) return;
      setCompact(alone);
      if (askStop) setAsking(true);
      if (quit) setQuitting(true);
      if (discard) setDiscarding(true);
    }, () => { if (active) setCompact(false); });
    return () => { active = false; for (const listener of listeners) void listener.then(stop => stop(), () => {}); };
  }, [queue, panel]);

  const listing = compact === false;
  const [changed, setChanged] = useState(0);
  useEffect(() => {
    if (!listing) return;
    let active = true;
    const listener = api.onRecordings(() => { if (active) setChanged(count => count + 1); });
    return () => { active = false; void listener.then(stop => stop(), () => {}); };
  }, [listing]);
  const phase = snapshot?.phase;
  const viewOpen = view === 'recordings';
  const watching = viewOpen ? snapshot : null;
  useEffect(() => {
    if (!listing) return;
    let active = true;
    if (viewOpen && !visit.current.open) visit.current.number += 1;
    visit.current.open = viewOpen;
    void api.recordings({ panel, visit: visit.current.number, open: viewOpen }).then(
      value => { if (active) setRecordings(value); }, cause => { if (active) setError(errorMessage(cause)); });
    return () => { active = false; };
  }, [listing, phase, watching, changed, viewOpen, settings, panel]);

  const ready = snapshot != null && compact != null;
  useEffect(() => {
    const element = root.current;
    if (!ready || !element) return;
    const fit = () => { void api.panelFit(Math.ceil(element.getBoundingClientRect().height)).catch(() => {}); };
    fit();
    const observer = new ResizeObserver(fit);
    observer.observe(element);
    return () => observer.disconnect();
  }, [ready]);

  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || event.defaultPrevented || document.querySelector('.select-menu:popover-open')) return;
      void api.closePanel();
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, []);

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

  const askedAbout = snapshot != null && stopPromptVisible(true, snapshot) ? `${snapshot.startedAt}/${snapshot.campaign}` : null;
  const previousAsked = useRef(askedAbout);
  useEffect(() => {
    if (previousAsked.current !== null && previousAsked.current !== askedAbout) {
      setAsking(false);
      setQuitting(false);
      setDiscarding(false);
    }
    previousAsked.current = askedAbout;
  }, [askedAbout]);

  useEffect(() => {
    if (compact && snapshot && !prompting) void api.closePanel();
  }, [compact, snapshot, prompting]);

  if (!snapshot || compact == null) return <div ref={root} className="recorder" />;
  const run = (work: () => Promise<unknown>) => {
    setError(null);
    void work().catch(cause => setError(errorMessage(cause)));
  };
  const save = (work: () => Promise<RecorderSettings | null>) => run(() => queue.save(work));
  const actions: PanelActions = {
    view: setView,
    toggleAsk: () => {
      if (!asking) { setAsking(true); setDiscarding(false); return; }
      setAsking(false);
      setQuitting(false);
      setDiscarding(false);
      run(api.cancelStopPrompt);
    },
    askDiscard: () => setDiscarding(true),
    stop: discard => { setAsking(false); setDiscarding(false); run(() => api.stopRecording(discard)); },
    record: () => run(api.startRecording),
    open: recording => run(() => api.openReplay(recording)),
    details: () => run(api.openDetails),
    folder: () => run(api.openRecordingsFolder),
    settings: patch => save(() => api.updateSettings(patch)),
    campaign: (campaign, choice) => save(() => api.setCampaignChoice(campaign, choice)),
    saveFolder: () => save(api.chooseSaveFolder),
    userFolder: choose => save(() => api.setUserFolder(choose)),
    waiting: (recording, discard) => {
      setConfirmingWaiting(null);
      run(async () => { setError(await api.actOnWaiting(recording.session, recording.campaign, discard)); });
    },
    askDiscardWaiting: recording => setConfirmingWaiting(recording && waitingKey(recording)),
    reveal: path => run(() => api.revealRecording(path)),
    compress: path => run(() => api.compressRecording(path)),
    dismiss: () => setError(null),
  };
  const update: PanelUpdate = {
    state: updateState, pending: updates.pending,
    ask: updateAsk && updateAsk.version === updateState?.downloadedVersion ? updateAsk.kind : null,
    press: () => {
      const action = updateAction(updateState);
      setError(null);
      if (action === 'check') void updates.check();
      const offered = updateState?.availableVersion ?? null;
      const downloaded = updateState?.downloadedVersion ?? null;
      if (action === 'download' && offered) void updates.download().then(done => { if (done) { setView('main'); setUpdateAsk({ kind: 'downloaded', version: offered }); } });
      if (action === 'install' && downloaded) { setView('main'); setUpdateAsk({ kind: 'confirm', version: downloaded }); }
    },
    restart: () => { setUpdateAsk(null); setError(null); void updates.install(); },
    dismiss: () => setUpdateAsk(null),
    channel: channel => { setError(null); void updates.run('Could not change the update channel', () => setUpdateChannel(channel)); },
  };
  return <PanelView snapshot={snapshot} settings={settings} view={view} asking={asking} error={error} now={Date.now()} actions={actions} root={root}
    compact={compact} recordings={recordings} quitting={quitting} discarding={discarding} confirmingWaiting={confirmingWaiting}
    update={updatesAvailable ? update : undefined} />;
}
