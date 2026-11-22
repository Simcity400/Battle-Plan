import { countryNameDetails } from '../../charts/tooltip.ts';
import { useCallback, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { CountryFlagUrl } from '../../components/CountryFlag';
import { ChipSet } from '../../components/ChipSet';
import ContentSetPicker from '../../components/ContentSetPicker';
import { Icon } from '../../components/Icons';
import ArtFallback from '../../components/ArtFallback';
import WidgetState from '../../components/WidgetState';
import LiveText from '../../charts/LiveText';
import type { LiveTexts } from '../../charts/LiveText';
import LogList from '../../charts/log/LogList';
import type { LogRow } from '../../charts/log/logData';
import { logSettings } from '../../charts/log/logOptions';
import { changesAfter } from '../../charts/log/logStates';
import { jumpItem } from '../../charts/jump';
import { AHEAD_ROWS, aheadEnd, useRevealClock } from '../../charts/reveal';
import { useAheadHour } from '../../dashboard/aheadHour';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { contentSet, resolveOptions, toggleContentSet } from '../../dashboard/options';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { OPTIONS, PROJECT_CHOICES } from './definition';
import { buildingChoices, constructionPlayback, describeRow, drawLimit, heldThrough, jumpHours, liveKey, liveProjectTexts, logComplete, placeTitle, projectAt, projectTooltip, projectType, rowCountryRequests, rowFactories, statusLabel, statusTone, typeName } from './constructionData.ts';
import type { ConstructionLog, ConstructionPlayback, ProjectAt } from './constructionData';
import { useConstructionLog } from './useConstructionLog';
import './constructionLog.css';

const NO_ROWS: readonly LogRow<ProjectAt>[] = [];

function LiveFactories({ live, at }: { live: LiveTexts; at: string }) {
  const cell = useRef<HTMLSpanElement>(null);
  useLayoutEffect(() => {
    const paint = () => {
      const node = cell.current, text = live.text(at);
      if (!node) return;
      const count = node.lastElementChild as HTMLElement | null;
      if (count && count.textContent !== text) count.textContent = text;
      const none = text === '—' || text === '0';
      node.classList.toggle('is-muted', none);
      node.dataset.tooltip = text === '—' ? 'Factories not recorded' : 'Factories on it';
      const icon = node.firstElementChild as SVGElement | null;
      if (icon) icon.style.display = text === '—' ? 'none' : '';
    };
    paint();
    return live.subscribe(paint);
  }, [live, at]);
  const text = live.text(at);
  return <span ref={cell} className={`log-value${text === '—' || text === '0' ? ' is-muted' : ''}`}
    data-tooltip={text === '—' ? 'Factories not recorded' : 'Factories on it'}>
    <Icon name="hammer" className="construction-factory-icon" style={text === '—' ? { display: 'none' } : undefined} /><span>{text}</span>
  </span>;
}
const QUEUE_CHIP = [{ value: 'queue', label: 'In the queue', hint: 'Only projects still in the queue at the moment shown' }];
const TYPES_FIELD = OPTIONS.find(field => field.key === 'types');
const EVERY_TYPE = TYPES_FIELD?.kind === 'content-set' ? TYPES_FIELD.automatic : 'Every type';

export default function ConstructionLogWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const live = widget.time.mode === 'follow';
  const options = resolveOptions(OPTIONS, widget.options);
  const settings = logSettings(options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const following = active && live && !!track;
  const trackHour = useCallback((clockHour: number) => {
    const at = track ? resolveWindow(widget, track, runtime.tracks, clockHour)?.lastHour : undefined;
    return at == null ? null : Math.floor(at);
  }, [track, widget, runtime.tracks]);
  const changes = useRef<{ windows: readonly { firstHour: number; lastHour: number }[]; log: ConstructionLog | null; playback: ConstructionPlayback | null }>({ windows: [], log: null, playback: null });
  const end = useCallback((clockHour: number) => {
    const at = trackHour(clockHour), held = changes.current;
    if (at == null || !held.playback || !held.log) return Math.floor(clockHour) + 1;
    const limit = drawLimit(held.windows, held.log, at);
    const { hoursPerSecond, playing } = runtime.clock.getSnapshot();
    const next = changesAfter(hour => held.playback!.span(hour).end, at, limit, AHEAD_ROWS + 1);
    return Math.floor(clockHour) + Math.min(aheadEnd(next, at, hoursPerSecond, playing), limit) - at;
  }, [trackHour, runtime.clock]);
  const block = useAheadHour(runtime.clock, following, end);
  const clockHour = block.hour;
  const filtered = widget.countries.kind !== 'all';
  const oneCountry = widget.countries.kind === 'selected' || (widget.countries.kind === 'tags' && widget.countries.tags.length === 1);
  const groups = useCountryGroups(runtime, track, active && filtered);
  const window = track && resolveWindow(widget, track, runtime.tracks, clockHour);
  const target = window ? Math.floor(window.lastHour) : null;
  const from = widget.time.mode === 'range' ? window?.firstHour ?? null : null;
  const inBranch = target != null && track != null && target >= track.branch.firstHour && target <= track.branch.lastHour;
  const { manifest, log, hour, error, retry, onDrawn } = useConstructionLog(runtime, track, active, inBranch ? target : null, live);
  const tags = selectedCountryKeys(widget.countries, track);
  const tagsKey = JSON.stringify(tags);
  const fallback = useMemo(() => tags, [tagsKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const types = typeof options.types === 'string' ? options.types : '';
  const projects = options.projects === 'new' || options.projects === 'repair' ? options.projects : 'all';
  const queue = options.queue === true;
  const order = options.order === 'start' ? 'start' : 'change';
  const rowPlayback = useMemo(() => log && match.ready
    ? constructionPlayback(log, { types: contentSet(types), projects, queue, country: match.matches, from }, order) : null,
  [log, match, types, projects, queue, from, order]);
  changes.current = { windows: manifest?.windows ?? [], log, playback: rowPlayback };
  const ahead = following && hour != null && hour === target && log != null;
  const last = ahead ? Math.min(hour + block.until - 1 - Math.floor(block.hour), drawLimit(manifest?.windows ?? [], log, hour) - 1) : hour;
  const rows = useMemo(() => hour != null && last != null ? rowPlayback?.states(hour, last) ?? NO_ROWS : NO_ROWS, [rowPlayback, hour, last]);
  const revealHour = useCallback((at: number) => trackHour(at) ?? -Infinity, [trackHour]);
  const reveal = useRevealClock(runtime.clock, ahead, revealHour);
  const [visible, setVisible] = useState<readonly LogRow<ProjectAt>[]>(NO_ROWS);
  const onVisible = useCallback((drawn: readonly LogRow<ProjectAt>[]) => { setVisible(drawn); onDrawn(rows, drawn, from); }, [rows, onDrawn, from]);
  const naming = inBranch && log != null && match.ready;
  const names = useMemo(() => rowCountryRequests(naming ? visible : NO_ROWS), [visible, naming]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, names.wanted, active && naming, names.maxHours);
  const country = useCallback((at: ProjectAt) => labelAt(at.project.firstRead)(at.project.tag), [labelAt]);
  const choices = useMemo(() => buildingChoices(manifest), [manifest]);
  const paintedHour = useRef<() => number>(() => 0);
  paintedHour.current = () => {
    if (hour == null || !log) return hour ?? 0;
    const at = following ? trackHour(runtime.clock.getHour()) : null;
    return at == null ? hour : Math.min(Math.max(at, hour), heldThrough(log));
  };
  const painted = useMemo(() => liveProjectTexts(runtime.clock, () => paintedHour.current(), new Map((log?.projects ?? []).map(project => [project.id, project]))),
    [runtime.clock, log]);
  useShownEachFrame(runtime.clock, () => !log || hour == null ? null : reveal && last != null ? Math.min(paintedHour.current(), last) : hour);
  const warnings = useMemo(() => [...new Set([...manifest?.warnings ?? [], ...[...log?.windows.values() ?? []].flatMap(reply => reply.warnings)])],
    [manifest, log]);
  useDiagnostics(onDiagnostics, warnings);

  if (!track) return <WidgetState kind="empty" icon="hammer" message="Load a campaign to see its construction." />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  const groupFailure = filtered && groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return groupFailure;
  if (!manifest || !match.ready) return <>{groupFailure}<WidgetState kind="loading" label="Loading the construction log…" /></>;
  if (match.reason) return <>{groupFailure}<WidgetState kind="unavailable" message={match.reason} /></>;
  if (!manifest.recorded) return <>{groupFailure}<WidgetState kind="unavailable" message="Construction was not recorded." /></>;
  if (!inBranch) return <>{groupFailure}<WidgetState kind="outside" /></>;
  if (!log || hour == null) return <>{groupFailure}<WidgetState kind="loading" label="Loading the construction log…" /></>;

  const chosen = contentSet(types);
  const toggle = (token: string) => { const next = toggleContentSet(types, token, Number.MAX_SAFE_INTEGER); if (next != null) setOption?.('types', next); };
  const unheld = manifest.missing.filter(span => span.firstHour <= hour);
  const renderRow = (row: LogRow<ProjectAt>) => {
    const at = row.item;
    const type = projectType(log, at.project);
    const label = country(at);
    const factories = rowFactories(at);
    const tone = statusTone(at);
    const named = (placeholder: string): ReactNode => label.known === undefined
      ? <span className={`skeleton ${placeholder}`} aria-hidden="true" /> : <span className="construction-ellipsis">{label.name}</span>;
    const flag = <CountryFlagUrl className="log-flag" url={label.flag} detail={label.flagDetail} reason={label.flagReason} pending={label.flagPending} />;
    return <>
      <span className="log-art" aria-hidden="true">
        {type?.icon ? <img src={type.icon} alt="" /> : <ArtFallback icon="factory" />}
        {(at.repairing || (at.status !== 'building' && at.status !== 'unread' && at.kind === 'repair')) && manifest.repairIcon && <img className="log-art-overlay" src={manifest.repairIcon} alt="" />}
      </span>
      <span className="construction-title">
        <span className="construction-building"><span className="construction-name">{typeName(type)}</span>
          {at.project.source !== 'outputAlone' && <span className="construction-levels"><LiveText live={painted} at={liveKey(at.project.id, 'levels')} /></span>}
          {at.project.source === 'outputAlone' && <span className="construction-levels">Known from output</span>}</span>
        <span className="construction-place">
          {!oneCountry && <span className="construction-place-country">{flag}{named('construction-name-pending is-short')}<span aria-hidden="true">·</span></span>}
          <span className="construction-ellipsis">{placeTitle(at.project, log.states)}</span>
        </span>
      </span>
      {!oneCountry && <span className="construction-country">{flag}{named('construction-name-pending')}</span>}
      <span className="construction-status">
        <span className={`pill${tone === 'neutral' ? '' : ` ${tone}`}`}>{live && at.status === 'building' && <i className="live-dot" aria-hidden="true" />}<span>{statusLabel(at)}</span></span>
      </span>
      {factories == null ? <span /> : <LiveFactories live={painted} at={liveKey(at.project.id, 'factories')} />}
    </>;
  };
  const menu = (row: LogRow<ProjectAt>) => jumpHours(row.item, hour).map(at => jumpItem(runtime, track.id, at));
  const empty = !logComplete(log, from) ? 'Loading earlier construction…'
    : chosen.length || projects !== 'all' || queue || from != null
    ? 'No construction matches these settings yet.' : widget.countries.kind === 'all' ? 'No construction yet.'
      : oneCountry ? 'No construction by this country yet.' : 'No construction by these countries yet.';
  return <div className={`log-widget construction-log${oneCountry ? ' is-one-country' : ''}`}>{groupFailure}
    {settings.chips && <div className="log-chips construction-filters">
      <ChipSet label="Projects" chips={PROJECT_CHOICES} selected={[projects]} max={1} full={false} onToggle={value => setOption?.('projects', value)} />
      <ChipSet label="Queue" chips={QUEUE_CHIP} selected={queue ? ['queue'] : []} max={1} full={false} onToggle={() => setOption?.('queue', !queue)} />
      {choices.length > 0 && <ContentSetPicker label="Building types" automatic={EVERY_TYPE} choices={choices} selected={chosen}
        onReset={() => { if (chosen.length) setOption?.('types', ''); }} onToggle={toggle} />}
    </div>}
    {unheld.length > 0 && <p className="construction-note">Construction is not recorded for {unheld.length} stretch{unheld.length === 1 ? '' : 'es'} of this campaign.</p>}
    <LogList rows={rows} label="Construction projects" density={settings.density} months={settings.months}
      animate={settings.animate && live} playhead={live ? hour : null} reveal={reveal} states renderRow={renderRow}
      describe={row => describeRow(log, row, country(row.item).name)}
      tooltip={row => {
        const at = paintedHour.current();
        return projectTooltip(log, projectAt(row.item.project, at) ?? row.item, at, country(row.item).name).map(form => ({ ...form, ...countryNameDetails({ nameDetail: form.detail }, country(row.item)) }));
      }}
      menu={menu} empty={empty} onVisible={onVisible} />
  </div>;
}
