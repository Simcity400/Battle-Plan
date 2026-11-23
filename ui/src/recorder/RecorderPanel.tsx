import { useCallback, useEffect, useRef, useState } from 'react';
import { settled as writesSettled } from '../shell/pendingWrites';
import type { ChangeEvent, ReactNode, Ref } from 'react';
import BrandMark from '../components/BrandMark';
import { CountryFlag } from '../components/CountryFlag';
import { DismissButton, FailureLine } from '../components/FailureLine';
import { Icon } from '../components/Icons';
import { Segmented } from '../components/Segmented';
import { Select } from '../components/Select';
import { actionFailure, plainFailure } from '../dashboard/queryError';
import { distinguishingFolder, distinguishingFolders, qualifiedRecordingName, recordingDisplayName } from '../dashboard/recordingNames';
import { errorMessage } from '../shell/errors';
import { AUTO_COMPRESS_HIGH } from '../shell/RecordingCompressionSetting';
import { newId } from '../shell/ids';
import * as api from './api';
import type { SettingsPatch } from './api';
import { askedPrompt, askQuestion, captureModeFixed, compressDetail, compressOffered, compressText, dateText, discardConsequence, discardDetail, escapeStep, followStatus, head, nextLoad, PANEL_UNOPENED, panelStage, pendingFailure, promptDiscards, promptTitle, stopConfirmed, recordingName,
  recordsAutomatically, sessionLength, settingsQueue, sizeText, spanText, spoolTotalText, stopPromptVisible, stopQuestion, untilNextMinute, waitingDiscardQuestion,
  whatToRecord } from './status.ts';
import type { CaptureMode, FlagImage, Head, NextLoad, PanelFailure, PromptFailure, RecorderSettings, Recordings, SavedRecording, Snapshot, UnsavedRecording } from './status';
import { AUTO_COMPRESS_NOTE, COMPRESS_TOOLTIP, COMPRESSION_NOTE } from './compression';
import type { Compression } from './compression';
import { setUpdateChannel, updatesAvailable } from '../update/api';
import type { UpdateChannel, UpdateState } from '../update/api';
import { blockedReason, CHANNEL_LABELS, channelDescription, channelLocked, confirmInstall, restartLabel, restartQuestion, UPDATE_READ_FAILED, updateAction, updateButtonLabel,
  updateDescription, updateDisabled, updateFailure, updateNews, updateTooltip } from '../update/logic';
import { UpdateIcon, useUpdateActions } from '../update/UpdateControl';
import { useUpdateState } from '../update/useUpdateState';
import './recorder.css';

export type View = 'main' | 'settings' | 'record' | 'recordings';

export interface PanelActions {
  view(view: View): void;
  toggleAsk(): void;
  askDiscard(): void;
  cancelDiscard(): void;
  stop(discard: boolean): void;
  record(): void;
  open(recording: string | null): void;
  showFiles(): void;
  folder(): void;
  settings(patch: SettingsPatch): void;
  campaign(campaign: string, choice: { record?: boolean; next?: NextLoad }): void;
  saveFolder(): void;
  userFolder(choose: boolean): void;
  waiting(recording: UnsavedRecording, discard: boolean): void;
  askDiscardWaiting(recording: UnsavedRecording | null): void;
  reveal(path: string): void;
  compress(path: string): void;
  relist(): void;
  dismiss(): void;
}

export interface PanelViewProps {
  snapshot: Snapshot;
  settings: RecorderSettings | null;
  view: View;
  asking: boolean;
  error: string | null;
  errorDetail?: string | null;
  errorCaveat?: boolean;
  now: number;
  actions: PanelActions;
  root?: Ref<HTMLDivElement>;
  compact?: boolean;
  recordings?: Recordings | null;
  relisting?: boolean;
  quitting?: boolean;
  discarding?: boolean;
  confirmingWaiting?: string | null;
  promptBusy?: boolean;
  promptFailure?: PromptFailure | null;
  shownPrompt?: number | null;
  onPromptSeen?: (prompt: number) => void;
  windowFailures?: readonly PromptFailure[];
  update?: PanelUpdate;
}

const PANEL_UNSHOWN = 'Couldn’t show the recorder panel.';
const PANEL_UNCLOSED = 'Couldn’t close the recorder panel.';

export interface PanelUpdate {
  state: UpdateState | null;
  failure: string | null;
  retry(): void;
  ask: 'confirm' | 'downloaded' | null;
  pending: boolean;
  press(): void;
  restart(): void;
  dismiss(): void;
  channel(channel: UpdateChannel): void;
}

const PARENT_VIEW: Record<View, View | null> = { main: null, settings: 'main', record: 'settings', recordings: 'main' };
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

function Title({ text, className }: { text: string; className?: string }) {
  const title = useRef<HTMLElement>(null);
  const [clipped, setClipped] = useState(false);
  useEffect(() => {
    const element = title.current;
    if (!element) return;
    const measure = () => setClipped(element.scrollWidth > element.clientWidth + 1);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [text]);
  return <strong ref={title} className={className} data-tooltip-overflow={text || undefined} tabIndex={clipped ? 0 : undefined}>{text}</strong>;
}

function Flag({ flag, name, className }: { flag: FlagImage | null; name: string; className: string }) {
  return <span className={className}><CountryFlag image={flag ?? undefined} name={name} /></span>;
}

export function PanelPending({ failure, title = PANEL_UNOPENED, retry, root }: { failure: string | null; title?: string; retry(): void; root?: Ref<HTMLDivElement> }) {
  return <div ref={root} className="recorder">
    <div className="recorder-head" data-tauri-drag-region="deep" role={failure ? 'alert' : 'status'}>
      {failure ? <span className="recorder-glyph is-failed"><Icon name="alert" /></span>
        : <span className="recorder-glyph"><span className="recorder-spin" aria-hidden="true" /></span>}
      <div className="recorder-who"><strong data-detail={failure ?? undefined}>{failure ? title : 'Connecting to the recorder…'}</strong></div>
      {failure && <button type="button" className="button small quiet" onClick={retry}>Try again</button>}
    </div>
  </div>;
}

const waitingCount = (recordings: Recordings | null | undefined) => recordings?.unsaved.filter(row => row.status !== 'recording').length ?? 0;

export function PanelView({ snapshot, settings, view, asking, error, errorDetail = null, errorCaveat = false, now, actions, root, compact = false, recordings = null, relisting = false, quitting = false,
  discarding = false, confirmingWaiting = null, promptBusy = false, promptFailure = null, shownPrompt = null, onPromptSeen, windowFailures = [], update }: PanelViewProps) {
  const [disclosed, setDisclosed] = useState<string | null>(null);
  const shown = head(snapshot);
  const files = snapshot.session != null && snapshot.problem?.failed === true;
  const problemActions = ({ detail }: { detail: string }) => (detail || files) && <span className="recorder-problem-actions">
    {detail && <button type="button" className="recorder-text-button" aria-expanded={disclosed === detail}
      onClick={() => setDisclosed(disclosed === detail ? null : detail)}>Details</button>}
    {files && <button type="button" className="recorder-text-button" onClick={actions.showFiles}>Show files</button>}
  </span>;
  const detailText = ({ detail }: { detail: string }) => detail && disclosed === detail && <span className="recorder-detail">{detail}</span>;
  const problem = error ?? (shown.problem ? null : snapshot.problem?.text ?? null);
  const failed = error != null ? !errorCaveat : snapshot.problem?.failed === true;
  const notice: ReactNode = problem && <div className={`notice ${failed ? 'error' : 'warning'}`} role={failed ? 'alert' : 'status'}>
    <Icon name="alert" /><span data-detail={error ? errorDetail ?? undefined : undefined}>{problem}
      {!error && snapshot.problem && <>{' '}{problemActions(snapshot.problem)}{detailText(snapshot.problem)}</>}</span>
    {error && <DismissButton onDismiss={actions.dismiss} />}
  </div>;
  const preparation = snapshot.preparation;
  const preparationNotice: ReactNode = preparation && (preparation.failed
    ? <div className="notice error" role="status"><Icon name="alert" /><span>{preparation.text}</span></div>
    : <p className="recorder-note" role="status"><Icon name="info" /><span>{preparation.text}</span></p>);
  const windowLines = windowFailures.map(failure => <FailureLine key={failure.text} className="recorder-failure" message={failure.text} detail={failure.detail}
    onRetry={failure.retry} />);
  const prompt = stopPromptVisible(asking, snapshot);
  const length = sessionLength(snapshot.startedAt, now);
  const promptFailureLine = promptFailure && <FailureLine className="recorder-failure" message={promptFailure.text} detail={promptFailure.detail}
    onRetry={promptFailure.retry} retryDisabled={promptBusy} />;
  const acted = () => { if (shownPrompt !== null) onPromptSeen?.(shownPrompt); };
  const promptActions = <>{promptFailureLine}
    <div className="recorder-prompt-actions">{discarding ? <>
    <button key="cancel-discard" type="button" className="button small quiet" autoFocus disabled={promptBusy} onClick={actions.cancelDiscard}>Cancel</button>
    <button key="confirm-discard" type="button" className="button small is-danger" disabled={promptBusy} onClick={() => actions.stop(true)}>Discard</button>
  </> : <>
    <button key="cancel" type="button" className="button small quiet" disabled={promptBusy} onClick={actions.toggleAsk}>Cancel</button>
    <button key="discard" type="button" className="button small recorder-discard" data-focus="discard" disabled={promptBusy} onClick={actions.askDiscard}>Discard</button>
    <button key="save" type="button" className="button small primary" autoFocus disabled={promptBusy} onClick={() => actions.stop(false)}>Save</button>
  </>}</div></>;

  if (compact) return <div ref={root} className="recorder recorder-compact">
    {error && notice}{windowLines}
    {!prompt && promptFailureLine}
    {prompt && <div className="recorder-prompt" role="alertdialog" aria-labelledby="recorder-prompt-title" onPointerDownCapture={acted} onKeyDownCapture={acted}
      aria-describedby={discarding ? 'recorder-prompt-question recorder-prompt-consequence' : 'recorder-prompt-question'}>
      <strong id="recorder-prompt-title">{promptTitle(quitting, discarding)}</strong>
      <p id="recorder-prompt-question" className="recorder-prompt-lead">{discarding ? discardDetail(snapshot.name, snapshot.startedAt, now)
        : stopQuestion(snapshot.name, snapshot.startedAt, now)}</p>
      {discarding && <p id="recorder-prompt-consequence">{discardConsequence(snapshot.continued)}</p>}
      {promptActions}
    </div>}
  </div>;

  const waiting = waitingCount(recordings);
  const footer = (more: ReactNode = null) => <footer className="recorder-foot">
    <button type="button" className="button small quiet recorder-brand" onClick={() => actions.open(null)}><BrandMark size={16} />Open BattlePlan</button>
    <span className="recorder-grow" />
    {more}
  </footer>;
  const back = (to: View) => <button type="button" className="icon-button" aria-label="Back" onClick={() => actions.view(to)}><Icon name="chevron-left" /></button>;

  if (view === 'recordings') {
    const unsaved = recordings?.unsaved ?? [];
    const saved = recordings?.saved ?? [];
    const total = recordings && spoolTotalText(unsaved);
    const unreadable = recordings?.unreadable ?? null;
    const folders = distinguishingFolders(saved.map(row => row.path));
    return <div ref={root} className="recorder">
      <div className="recorder-top" data-tauri-drag-region="deep">{back('main')}Recordings</div>
      {error && notice}{windowLines}
      {recordings ? <div className="recorder-list themed-scrollbar">
        <section className="recorder-section is-first" aria-label="Not saved yet">
          <div className="group-label recorder-label recorder-label-row"><span>Not saved yet</span>{total && <span className="recorder-total">{total}</span>}</div>
          {unsaved.length ? unsaved.map(row => <UnsavedRow key={waitingKey(row)} row={row} actions={actions}
            confirming={confirmingWaiting === waitingKey(row)} />)
            : <p className="recorder-empty">Nothing is waiting to be saved.</p>}
        </section>
        <section className="recorder-section is-last" aria-label="Saved">
          <div className="group-label recorder-label">Saved</div>
          {unreadable && <FailureLine className="recorder-failure" detail={unreadable.detail} onRetry={actions.relist} retryDisabled={relisting}
            message={unreadable.whole ? 'Couldn’t read the recordings folder.' : 'Couldn’t read some folders in the recordings folder.'} />}
          {saved.length ? saved.map(row => <SavedRow key={row.path} row={row} folder={distinguishingFolder(folders, row.path)}
              full={qualifiedRecordingName(folders, row.path)} settings={settings} actions={actions} />)
            : !unreadable && <p className="recorder-empty">No saved recordings yet.</p>}
        </section>
      </div> : !error && <p className="recorder-loading" role="status"><span className="recorder-spin" aria-hidden="true" />Loading recordings…</p>}
      {footer(<button type="button" className="icon-button" aria-label="Open recordings folder" data-tooltip="Open recordings folder"
        onClick={actions.folder}><Icon name="folder-open" /></button>)}
    </div>;
  }

  if (view !== 'main' && settings) {
    if (view === 'record') {
      const fixed = captureModeFixed(snapshot);
      const mode = snapshot.captureMode ?? settings.captureMode;
      return <div ref={root} className="recorder">
        <div className="recorder-top" data-tauri-drag-region="deep">{back('settings')}What to record</div>
        {error && notice}{windowLines}
        <section className="recorder-section is-first is-last">
          <div className="group-label recorder-label">Capture mode</div>
          <CaptureModeChoice value={mode} fixed={fixed} onChange={captureMode => actions.settings({ captureMode })} />
          <p className="recorder-note"><Icon name="info" /><span>{fixed
            ? 'Fixed while connected to the game, including at its menu. A different mode starts a new recording file.'
            : 'A campaign recorded in the other mode continues in a new recording file.'}</span></p>
        </section>
        {footer()}
      </div>;
    }
    const saveOptions = [{ value: 'current', label: folderName(settings.effectiveSaveFolder), reason: settings.effectiveSaveFolder },
      { value: 'choose', label: 'Choose folder…' }];
    const userOptions = [{ value: 'automatic', label: 'Automatic' },
      ...(settings.userFolder ? [{ value: 'custom', label: folderName(settings.userFolder), reason: settings.userFolder }] : []),
      { value: 'choose', label: 'Choose folder…' }];
    return <div ref={root} className="recorder">
      <div className="recorder-top" data-tauri-drag-region="deep">{back('main')}Settings</div>
      {error && notice}{windowLines}
      <section className="recorder-section is-first">
        <div className="group-label recorder-label">Recording</div>
        <Toggle label="Auto record" description="Records each campaign when it loads." checked={settings.recordEveryCampaign}
          onChange={recordEveryCampaign => actions.settings({ recordEveryCampaign })} />
        <Toggle label="Auto save" description="Saves when you load another campaign or close the game." checked={settings.autoSave}
          onChange={autoSave => actions.settings({ autoSave })} />
        <button type="button" className="recorder-row recorder-link" data-opens="record" onClick={() => actions.view('record')}><span>What to record</span>
          <span className="recorder-value">{whatToRecord(settings)}<Icon name="chevron-right" /></span></button>
        <div className="recorder-row"><span>Compression</span>
          <Segmented<Compression> label="Compression" value={settings.liveCompression}
            options={[{ value: 'low', label: 'Low' }, { value: 'high', label: 'High' }]} onChange={compression => actions.settings({ compression })} /></div>
        <p className="recorder-note">{COMPRESSION_NOTE}</p>
        {settings.liveCompression === 'low' ? <Toggle label="Auto compress" description={AUTO_COMPRESS_NOTE} checked={settings.autoCompress}
          onChange={autoCompress => actions.settings({ autoCompress })} />
          : <div className="recorder-row recorder-toggle"><span className="recorder-toggle-text"><span>Auto compress</span><small>{AUTO_COMPRESS_HIGH}</small></span>
            <span className="recorder-value">Low only</span></div>}
      </section>
      <section className="recorder-section">
        <div className="group-label recorder-label">Folders</div>
        <div className="recorder-row"><span>Recordings folder</span>
          <Select label="Recordings folder" value="current" options={saveOptions} onChange={value => { if (value === 'choose') actions.saveFolder(); }} /></div>
        <div className="recorder-row"><span>Paradox user folder</span>
          <Select label="Paradox user folder" value={settings.userFolder ? 'custom' : 'automatic'} options={userOptions}
            onChange={value => { if (value !== 'custom') actions.userFolder(value === 'choose'); }} /></div>
      </section>
      <section className={`recorder-section${update ? '' : ' is-last'}`}>
        <div className="group-label recorder-label">Start automatically</div>
        <Switch label="With Windows" checked={settings.startWithWindows} onChange={startWithWindows => actions.settings({ startWithWindows })} />
        <Switch label="With BattlePlan" checked={settings.startWithReplay} onChange={startWithReplay => actions.settings({ startWithReplay })} />
      </section>
      {update && <UpdateSection update={update} />}
      {update?.ask && update.state && <UpdateAsk update={update} />}
      {footer()}
    </div>;
  }

  const campaign = snapshot.campaign;
  const ready = update && !update.ask && updateAction(update.state) === 'install' ? update.state?.downloadedVersion ?? null : null;
  const readyFailure = ready ? updateFailure(update!.state) : null;
  return <div ref={root} className="recorder">
    <div className="recorder-head" data-tauri-drag-region="deep">
      {shown.glyph ? <span className={`recorder-glyph${shown.problem ? shown.problem.failed ? ' is-failed' : ' is-warning' : ''}`}>
        <Icon name={shown.problem ? 'alert' : 'gamepad'} /></span>
        : <Flag flag={shown.flag} name={shown.title} className="recorder-flag" />}
      <div className="recorder-who">{shown.problem ? <strong className="is-problem">{shown.title}</strong> : <Title text={shown.title} />}
        {shown.problem ? (shown.problem.text || problemActions(shown.problem)) && <span className="recorder-state is-problem">
          {shown.problem.text && `${shown.problem.text} `}{problemActions(shown.problem)}</span>
          : <State head={shown} />}
        {shown.problem && detailText(shown.problem)}
        {shown.sizes && <span className="recorder-sizes">{shown.sizes}</span>}</div>
      {shown.action === 'stop' && <button type="button" className={`button small${asking ? ' is-pressed' : ''}`} aria-pressed={asking} data-focus="stop"
        onClick={actions.toggleAsk}><span className="recorder-square" aria-hidden="true" />Stop</button>}
      {shown.action === 'record' && <button type="button" className="button small" onClick={actions.record}>
        <span className="recorder-round" aria-hidden="true" />Record</button>}
      {shown.action === 'open' && <button type="button" className="button small primary" onClick={() => actions.open(snapshot.recording)}>Open</button>}
    </div>
    {notice}{windowLines}
    {preparationNotice}
    <div className="recorder-body">
      {prompt && <div className="recorder-ask" role="alertdialog" aria-labelledby="recorder-ask-question" onPointerDownCapture={acted} onKeyDownCapture={acted}>
        <p id="recorder-ask-question" className="recorder-ask-question">{askQuestion(quitting, discarding, length)}</p>
        {discarding && <p className="recorder-ask-detail">{discardConsequence(snapshot.continued)}</p>}
        {promptActions}
      </div>}
      <div className={`recorder-under${prompt ? ' is-covered' : ''}`} inert={prompt}>
      {!prompt && promptFailureLine}
      {shown.busy && <div className="recorder-bar" role="progressbar" aria-label={shown.label ?? undefined}><i /></div>}
      {shown.campaign && campaign && settings && <section className="recorder-section">
        <div className="group-label recorder-label">This campaign</div>
        <Switch label="Auto record" checked={recordsAutomatically(settings, campaign)} onChange={record => actions.campaign(campaign, { record })} />
        <div className="recorder-row"><span>Next time it loads</span>
          <Segmented<NextLoad> label="Next time it loads" value={nextLoad(settings, campaign)}
            options={[{ value: 'continue', label: 'Continue' }, { value: 'newFile', label: 'New file' }]}
            onChange={next => actions.campaign(campaign, { next })} /></div>
      </section>}
      </div>
    </div>
    {update?.ask && update.state && <UpdateAsk update={update} />}
    {ready && !prompt && <div className="recorder-update-ready" role={readyFailure ? undefined : 'status'}>
      {readyFailure ? <FailureLine message={readyFailure.text} detail={readyFailure.detail} />
        : <><span className="update-dot" aria-hidden="true" /><span>{update!.state?.downgrade ? 'Version' : 'Update'} {ready} is ready</span></>}
      <button type="button" className="button small" data-focus="update" onClick={update!.press}><Icon name="download" />{restartLabel(update!.state)}</button>
    </div>}
    {footer(<>
      {update && !ready && !update.ask && updateNews(update.state) && <button type="button" className="update-button" aria-label={updateTooltip(update.state)} data-tooltip={updateTooltip(update.state)}
        aria-disabled={updateDisabled(update.state) || update.pending || undefined} onClick={update.press}><UpdateIcon state={update.state} /></button>}
      <button type="button" className="button small quiet recorder-recordings" data-opens="recordings"
        aria-label={waiting ? `Recordings, ${waiting} waiting to be saved` : undefined}
        onClick={() => actions.view('recordings')}>Recordings{waiting > 0 && <span className="recorder-badge" aria-hidden="true">{waiting}</span>}</button>
      <button type="button" className="icon-button" aria-label="Settings" data-tooltip="Settings" data-opens="settings" disabled={!settings}
        onClick={() => actions.view('settings')}><Icon name="settings" /></button>
    </>)}
  </div>;
}

function UpdateAsk({ update }: { update: PanelUpdate }) {
  const state = update.state!;
  const version = state.downloadedVersion ?? state.availableVersion ?? '';
  const blocked = state.installBlocked;
  return <div className="recorder-ask" role="alertdialog" aria-labelledby="recorder-update-question" aria-describedby="recorder-update-detail">
    <p id="recorder-update-question" className="recorder-ask-question">{restartQuestion(version, state.downgrade)}</p>
    <p id="recorder-update-detail" className={`recorder-ask-detail${blocked ? ' is-warning' : ''}`} role={blocked ? 'status' : undefined}>
      {blocked ? <><Icon name="alert" /><span>{blockedReason(state)}</span></> : confirmInstall(version, state.downgrade)}</p>
    <div className="recorder-prompt-actions">
      <button key="later" type="button" className="button small quiet" autoFocus onClick={update.dismiss}>{update.ask === 'downloaded' ? 'Later' : 'Cancel'}</button>
      <button key="restart" type="button" className="button small primary" disabled={blocked != null || update.pending}
        onClick={update.restart}>{restartLabel(state)}</button>
    </div>
  </div>;
}

function UpdateSection({ update }: { update: PanelUpdate }) {
  const { state } = update;
  const unread = !state && update.failure != null;
  const shown = updateFailure(state);
  const failure = unread ? { text: UPDATE_READ_FAILED, detail: update.failure, retry: update.retry } : shown && { ...shown, retry: update.press };
  const channelNote = channelDescription(state);
  return <section className="recorder-section is-last">
    <div className="group-label recorder-label">Updates</div>
    <div className="recorder-row recorder-toggle recorder-described"><span className="recorder-toggle-text"><span>BattlePlan Recorder {state?.currentVersion ?? ''}</span>
      {failure ? <FailureLine className="recorder-update-failure" message={failure.text} detail={failure.detail} onRetry={failure.retry} retryDisabled={update.pending} />
        : <small>{updateDescription(state)}</small>}</span>
      {!failure && <button type="button" className={`button small${updateAction(state) === 'install' ? ' primary' : ''}`} disabled={updateDisabled(state) || update.pending}
        data-tooltip={updateTooltip(state)} data-focus="update-settings" onClick={update.press}>
        <Icon name="download" />{updateButtonLabel(state)}</button>}</div>
    <div className="recorder-row recorder-toggle recorder-described"><span className="recorder-toggle-text"><span>Channel</span>
      {channelNote && <small>{channelNote}</small>}</span>
      {state?.enabled ? <Segmented<UpdateChannel> label="Update channel" value={state.channel} disabled={channelLocked(state, update.pending)}
        options={(['stable', 'nightly'] as const).map(value => ({ value, label: CHANNEL_LABELS[value] }))}
        onChange={channel => { if (channel !== state.channel) update.channel(channel); }} />
        : <span className="recorder-value">{state ? CHANNEL_LABELS[state.channel] : unread ? 'Unavailable' : '—'}</span>}</div>
  </section>;
}

function UnsavedRow({ row, actions, confirming }: { row: UnsavedRecording; actions: PanelActions; confirming: boolean }) {
  const status = row.status === 'recording' ? 'Recording' : row.status === 'saving' ? 'Saving…' : 'Waiting';
  const when = row.status === 'recording' ? dateText(row.lastHour) : spanText(row.firstHour, row.lastHour);
  const asking = confirming && row.status === 'waiting';
  const name = recordingName(row.name);
  return <div className={`recorder-item${asking ? ' is-confirming' : ''}`} data-waiting={waitingKey(row)} data-tooltip-side="above">
    <Flag flag={row.flag} name={name} className="recorder-item-flag" />
    <span className="recorder-item-name" data-tooltip-overflow={asking ? undefined : name}>{name}</span>
    {asking ? <span className="recorder-item-meta is-asking" role="status"><span className="recorder-item-question">{waitingDiscardQuestion(row.recording != null)}</span>
      {' '}{discardConsequence(row.recording != null)}</span>
      : <span className="recorder-item-meta"><span className={`recorder-dot ${row.status === 'recording' ? 'red' : 'grey'}`} aria-hidden="true" />
        <span>{joined([status, when, sizeText(row.spoolBytes)])}</span></span>}
    {row.status === 'waiting' && <span className="recorder-item-actions">{asking ? <>
      <button key="cancel-discard" type="button" className="button small quiet recorder-item-action" autoFocus
        onClick={() => actions.askDiscardWaiting(null)}>Cancel</button>
      <button key="confirm-discard" type="button" className="button small is-danger recorder-item-action" onClick={() => actions.waiting(row, true)}>Discard</button>
    </> : <>
      <button key="discard" type="button" className="button small recorder-discard recorder-item-action" onClick={() => actions.askDiscardWaiting(row)}>Discard</button>
      <button key="save" type="button" className="button small primary recorder-item-action" onClick={() => actions.waiting(row, false)}>Save</button>
    </>}</span>}
  </div>;
}

function SavedRow({ row, folder, full, settings, actions }: { row: SavedRecording; folder: string | null; full: string; settings: RecorderSettings | null;
  actions: PanelActions }) {
  const kept = row.job?.state === 'kept' || row.job?.state === 'unconfirmed';
  const name = recordingDisplayName(row.path);
  return <div className="recorder-item is-openable" data-tooltip-side="above">
    <Flag flag={row.flag} name={name} className="recorder-item-flag" />
    <button type="button" className="recorder-item-name recorder-item-open" data-tooltip-overflow={full} aria-label={folder ? full : undefined}
      aria-description="Open in BattlePlan" onClick={() => actions.open(row.path)}>{name}</button>
    <span className={`recorder-item-meta${kept ? ' is-warning' : ''}`} role={row.job ? 'status' : undefined}>
      {folder && <><span className="recorder-item-folder"><bdi>{folder}</bdi></span><span aria-hidden="true">·</span></>}
      <span data-detail={compressDetail(row.job)}>{joined([spanText(row.firstHour, row.lastHour), sizeText(row.bytes), compressText(row.job)])}</span></span>
    <span className="recorder-item-actions">
      {compressOffered(settings, row) && <button type="button" className="button small recorder-item-action" aria-description={COMPRESS_TOOLTIP}
        onClick={() => actions.compress(row.path)}>Compress</button>}
      <button type="button" className="icon-button" aria-label="Show in folder" data-tooltip="Show in folder"
        onClick={() => actions.reveal(row.path)}><Icon name="folder-open" /></button></span>
  </div>;
}

export default function RecorderPanel() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [settings, setSettings] = useState<RecorderSettings | null>(null);
  const [recordings, setRecordings] = useState<Recordings | null>(null);
  const [view, setView] = useState<View>('main');
  const [opened, setOpened] = useState<number | null>(null);
  const [stepped, setStepped] = useState<number | null>(null);
  const [promptAction, setPromptAction] = useState(false);
  const [promptFailure, setPromptFailure] = useState<PromptFailure | null>(null);
  const [confirmingWaiting, setConfirmingWaiting] = useState<string | null>(null);
  const [compact, setCompact] = useState<boolean | null>(null);
  const [fitFailure, setFitFailure] = useState<PromptFailure | null>(null);
  const [closeFailure, setCloseFailure] = useState<PromptFailure | null>(null);
  const [failure, setFailure] = useState<{ text: string; detail?: string; caveat?: boolean } | null>(null);
  const setError = (sentence: string, cause: unknown) => setFailure(actionFailure(sentence, errorMessage(cause)));
  const clearError = () => setFailure(null);
  const [statusFailure, setStatusFailure] = useState<string | null>(null);
  const retryStatus = useRef<(() => void) | null>(null);
  const [, setMinute] = useState(0);
  const root = useRef<HTMLDivElement>(null);
  const [panel] = useState(newId);
  const visit = useRef({ number: 0, open: false });
  const [openFailure, setOpenFailure] = useState<PanelFailure | null>(null);
  const [queue] = useState(() => settingsQueue<RecorderSettings>(api.recorderSettings, setSettings,
    warning => setFailure({ text: 'Saved, but Windows didn’t confirm the change reached the disk.', detail: warning }),
    cause => setFailure(actionFailure('Couldn’t read the recorder settings.', errorMessage(cause)))));
  const { state: updateState, error: updateReadFailure, retry: retryUpdateRead } = useUpdateState();
  const updates = useUpdateActions();
  const [updateAsk, setUpdateAsk] = useState<{ kind: 'confirm' | 'downloaded'; version: string; from: View } | null>(null);
  useEffect(() => { if (updates.failure) setFailure({ text: updates.failure.text, detail: updates.failure.detail }); }, [updates.failure]);

  useEffect(() => {
    let active = true;
    const loadSettings = () => {
      void queue.refresh().catch(cause => { if (active) setError('Couldn’t read the recorder settings.', cause); });
    };
    const status = followStatus(api.onStatus, api.recorderStatus, value => { if (active) setSnapshot(value); },
      cause => { if (active) setStatusFailure(errorMessage(cause)); }, value => value.revision);
    retryStatus.current = status.retry;
    const listeners = [api.onSettings(() => { if (active) loadSettings(); })];
    loadSettings();
    return () => { active = false; status.stop(); for (const listener of listeners) void listener.then(stop => stop(), () => {}); };
  }, [queue, panel]);

  useEffect(() => {
    let active = true;
    const open = () => {
      void api.panelOpened(panel).then(({ compact: alone }) => { if (active) setCompact(alone); },
        cause => { if (active) setOpenFailure({ title: PANEL_UNOPENED, detail: errorMessage(cause), retry: () => { setOpenFailure(null); open(); } }); });
    };
    open();
    return () => { active = false; };
  }, [panel]);

  const listing = compact === false;
  const [changed, setChanged] = useState(0);
  const [relisting, setRelisting] = useState(false);
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
      value => { if (active) setRecordings(value); }, cause => { if (active) setError('Couldn’t list the recordings.', cause); })
      .finally(() => { if (active) setRelisting(false); });
    return () => { active = false; };
  }, [listing, phase, watching, changed, viewOpen, settings, panel]);

  const stage = panelStage(snapshot, compact);
  useEffect(() => {
    const element = root.current;
    if (!element) return;
    let active = true;
    const fit = () => {
      void api.panelFit(Math.ceil(element.getBoundingClientRect().height)).then(() => {
        if (active) setFitFailure(null);
      }, cause => { if (active) setFitFailure({ ...actionFailure(PANEL_UNSHOWN, errorMessage(cause)), retry: fit }); });
    };
    fit();
    const observer = new ResizeObserver(fit);
    observer.observe(element);
    return () => { active = false; observer.disconnect(); };
  }, [stage]);
  const closeSelf = useCallback(function close(revision: number) {
    setCloseFailure(null);
    void writesSettled().then(() => api.closePanel(revision)).catch(cause => setCloseFailure({ ...actionFailure(PANEL_UNCLOSED, errorMessage(cause)), retry: () => close(revision) }));
  }, []);

  const asked = askedPrompt(snapshot, opened);
  const prompting = asked !== null;
  const quitting = asked?.quitting ?? false;
  const discarding = promptDiscards(asked, stepped);
  const hostPrompt = snapshot?.prompt ?? null;
  useEffect(() => { if (hostPrompt) setView('main'); }, [hostPrompt?.id]);
  const refocus = useRef<string | null>(null);
  useEffect(() => {
    if (!refocus.current) return;
    root.current?.querySelector<HTMLElement>(refocus.current)?.focus();
    refocus.current = null;
  });
  const navigate = (to: View) => {
    if (PARENT_VIEW[view] === to) refocus.current = `[data-opens="${view}"]`;
    setView(to);
  };
  const answerPrompt = (sentence: string, work: () => Promise<unknown>, again: () => void) => {
    if (promptAction) return;
    setPromptAction(true);
    setPromptFailure(null);
    void work().then(() => {
      setPromptAction(false);
      setOpened(null);
      setStepped(null);
      refocus.current = '[data-focus="stop"]';
    }, cause => {
      setPromptAction(false);
      setPromptFailure({ ...actionFailure(sentence, errorMessage(cause)), retry: again });
    });
  };
  const cancelPrompt = () => {
    if (promptAction) return;
    clearError();
    if (!hostPrompt) {
      setOpened(null);
      setStepped(null);
      setPromptFailure(null);
      refocus.current = '[data-focus="stop"]';
      return;
    }
    const { id } = hostPrompt;
    const cancel = () => answerPrompt('Couldn’t keep recording.', () => api.cancelStopPrompt(id), cancel);
    cancel();
  };
  const cancelDiscard = () => {
    if (promptAction) return;
    if (asked === null || stepped !== asked.sessionKey) { cancelPrompt(); return; }
    setStepped(null);
    setPromptFailure(null);
    refocus.current = '[data-focus="discard"]';
  };
  const askDiscardWaiting = (recording: UnsavedRecording | null) => {
    if (!recording && confirmingWaiting) refocus.current = `[data-waiting="${CSS.escape(confirmingWaiting)}"] .recorder-discard`;
    setConfirmingWaiting(recording && waitingKey(recording));
  };
  const updateAsked = updateAsk != null && updateAsk.from === view && updateAsk.version === updateState?.downloadedVersion;
  const dismissUpdate = () => {
    refocus.current = updateAsk?.from === 'settings' ? '[data-focus="update-settings"], .recorder-update-failure button' : '[data-focus="update"]';
    setUpdateAsk(null);
  };
  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || event.defaultPrevented || document.querySelector('.select-menu:popover-open')) return;
      event.preventDefault();
      const up = PARENT_VIEW[view];
      const step = escapeStep({ updateAsked, confirmingWaiting: !!confirmingWaiting, prompting, discarding, promptBusy: promptAction, up: !!up });
      if (step === 'update') dismissUpdate();
      else if (step === 'waiting') askDiscardWaiting(null);
      else if (step === 'discard') cancelDiscard();
      else if (step === 'prompt') cancelPrompt();
      else if (step === 'up' && up) navigate(up);
      else if (step === 'close') closeSelf(snapshot?.revision ?? 0);
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  });

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

  const screen = useRef({ asked, stepped });
  useEffect(() => { screen.current = { asked, stepped }; });
  const sessionKey = snapshot?.sessionKey ?? null;
  const askedSession = useRef(sessionKey);
  useEffect(() => {
    if (askedSession.current !== sessionKey) setPromptFailure(null);
    askedSession.current = sessionKey;
  }, [sessionKey]);

  const shownPrompt = hostPrompt !== null && asked === hostPrompt ? hostPrompt.id : null;
  const promptSeen = useCallback((prompt: number) => { void api.promptSeen(prompt).catch(() => {}); }, []);
  const settled = !prompting && !promptAction && !failure && !promptFailure;
  useEffect(() => {
    if (compact && snapshot && settled) closeSelf(snapshot.revision);
  }, [compact, snapshot, settled, closeSelf]);

  if (stage === 'pending') {
    const pending = pendingFailure(openFailure, fitFailure);
    if (pending) return <PanelPending root={root} title={pending.title} failure={pending.detail} retry={pending.retry} />;
    return <PanelPending root={root} failure={snapshot ? null : statusFailure} retry={() => { setStatusFailure(null); retryStatus.current?.(); }} />;
  }
  if (!snapshot || compact == null) return <div ref={root} className="recorder" />;
  const run = (sentence: string, work: () => Promise<unknown>) => {
    clearError();
    void work().catch(cause => setError(sentence, cause));
  };
  const save = (work: () => Promise<RecorderSettings | null>) => run('Couldn’t save the recorder settings.', () => queue.save(work));
  const actions: PanelActions = {
    view: navigate,
    toggleAsk: () => {
      if (!prompting) { setOpened(snapshot.sessionKey ?? null); setStepped(null); setPromptFailure(null); return; }
      cancelPrompt();
    },
    askDiscard: () => setStepped(asked?.sessionKey ?? null),
    cancelDiscard,
    stop: discard => {
      if (!asked || !stopConfirmed(asked, stepped, discard, asked.sessionKey)) return;
      clearError();
      const { sessionKey: key } = asked;
      const stop = () => {
        if (!stopConfirmed(screen.current.asked, screen.current.stepped, discard, key)) { setPromptFailure(null); return; }
        answerPrompt(discard ? 'Couldn’t discard the session.' : 'Couldn’t save the recording.', () => api.stopRecording(discard, key), stop);
      };
      stop();
    },

    record: () => run('Couldn’t start recording.', api.startRecording),
    open: recording => run(recording ? 'Couldn’t open the recording in BattlePlan.' : 'Couldn’t open BattlePlan.', () => api.openReplay(recording)),
    showFiles: () => run('Couldn’t show the session’s files.', api.showSessionFiles),
    folder: () => run('Couldn’t open the recordings folder.', api.openRecordingsFolder),
    settings: patch => save(() => api.updateSettings(patch)),
    campaign: (campaign, choice) => save(() => api.setCampaignChoice(campaign, choice)),
    saveFolder: () => save(api.chooseSaveFolder),
    userFolder: choose => save(() => api.setUserFolder(choose)),
    waiting: (recording, discard) => {
      setConfirmingWaiting(null);
      run(discard ? 'Couldn’t discard the recording.' : 'Couldn’t save the recording.', async () => {
        const caveat = await api.actOnWaiting(recording.session, recording.campaign, discard);
        if (caveat) setFailure({ ...plainFailure(caveat), caveat: true });
      });
    },
    askDiscardWaiting,
    reveal: path => run('Couldn’t show the recording in its folder.', () => api.revealRecording(path)),
    compress: path => run('Couldn’t compress the recording.', () => api.compressRecording(path)),
    relist: () => { setRelisting(true); setChanged(count => count + 1); },
    dismiss: clearError,
  };
  const update: PanelUpdate = {
    state: updateState, failure: updateReadFailure, retry: retryUpdateRead, pending: updates.pending,
    ask: updateAsked ? updateAsk.kind : null,
    press: () => {
      const action = updateAction(updateState);
      clearError();
      if (action === 'check') void updates.check();
      const offered = updateState?.availableVersion ?? null;
      const downloaded = updateState?.downloadedVersion ?? null;
      if (action === 'download' && offered) void updates.download().then(done => { if (done) setUpdateAsk({ kind: 'downloaded', version: offered, from: view }); });
      if (action === 'install' && downloaded) setUpdateAsk({ kind: 'confirm', version: downloaded, from: view });
    },
    restart: () => { setUpdateAsk(null); clearError(); void updates.install(); },
    dismiss: dismissUpdate,
    channel: channel => {
      if (!updateState || channel === updateState.channel || channelLocked(updateState, updates.pending)) return;
      clearError();
      void updates.run('Couldn’t change the update channel.', () => setUpdateChannel(channel));
    },
  };
  return <PanelView snapshot={snapshot} settings={settings} view={view} asking={prompting} error={failure?.text ?? null} errorDetail={failure?.detail}
    errorCaveat={failure?.caveat} now={Date.now()}
    actions={actions} root={root} compact={compact} recordings={recordings} relisting={relisting} quitting={quitting} discarding={discarding} confirmingWaiting={confirmingWaiting}
    promptBusy={promptAction} promptFailure={promptFailure} shownPrompt={shownPrompt} onPromptSeen={promptSeen}

    windowFailures={[fitFailure, closeFailure].filter((failure): failure is PromptFailure => failure !== null)} update={updatesAvailable ? update : undefined} />;
}
