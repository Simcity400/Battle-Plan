import { useCallback, useMemo, useState } from 'react';
import { CountryFlagUrl } from '../../components/CountryFlag';
import { ChipSet } from '../../components/ChipSet';
import { Icon } from '../../components/Icons';
import ArtFallback from '../../components/ArtFallback';
import WidgetState from '../../components/WidgetState';
import LogList from '../../charts/log/LogList';
import type { LogRow } from '../../charts/log/logData';
import { logCountryRequests } from '../../charts/log/logData';
import { logSettings } from '../../charts/log/logOptions';
import { jumpItem } from '../../charts/jump';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { selectedTags } from '../../dashboard/countrySelection';
import { stateNameRequests } from '../../dashboard/construction';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { contentSet, resolveOptions, storableToken, toggleContentSet, UNSTORABLE } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useStateNamesAt } from '../../dashboard/useStateNamesAt';
import { OPTIONS } from './definition';
import { buildingChoices, describeRow, factoriesShown, jumpHours, levelsLabel, logComplete, logRows, placeLabel, projectTooltip, projectType, statusLabel, typeName } from './constructionData';
import type { ProjectAt } from './constructionData';
import { useConstructionLog } from './useConstructionLog';
import './constructionLog.css';

const NO_ROWS: readonly LogRow<ProjectAt>[] = [];
/** The cells after a row's date: artwork, the sentence, its status and its factories. */
const COLUMNS = '24px minmax(0, 1fr) auto 3.4em';

/** Every construction project of the chosen countries as a log:
 * newest first, each with the game's building art, where it is, its levels, when it
 * started and finished and the factories on it. With Live on it follows playback, and
 * a project enters at the top as its reads show it start, build a level or end. */
export default function ConstructionLogWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const live = widget.time.mode === 'follow';
  usePlaybackHour(runtime.clock, active && live);
  const options = resolveOptions(OPTIONS, widget.options);
  const settings = logSettings(options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const filtered = widget.countries.kind !== 'all';
  const groups = useCountryGroups(runtime, track, active && filtered);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const target = window?.lastHour ?? null;
  const from = widget.time.mode === 'range' ? window?.firstHour ?? null : null;
  const inBranch = target != null && track != null && target >= track.branch.firstHour && target <= track.branch.lastHour;
  // The hour shown is the target's once its window is held.
  const { manifest, log, hour, error, retry, onDrawn } = useConstructionLog(runtime, track, active, inBranch ? target : null, live);
  // Tags stay an array end to end: a mod's tag may hold any character, spaces included.
  const tags = selectedTags(widget.countries, track);
  const tagsKey = JSON.stringify(tags);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const fallback = useMemo(() => tags, [tagsKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const types = typeof options.types === 'string' ? options.types : '';
  const projects = options.projects === 'new' || options.projects === 'repair' ? options.projects : 'all';
  const order = options.order === 'start' ? 'start' : 'change';
  const rows = useMemo(() => log && hour != null && match.ready
    ? logRows(log, hour, { types: contentSet(types), projects, country: match.matches, from }, order) : NO_ROWS,
  [log, hour, match, types, projects, from, order]);
  // Countries are named and flagged as recorded when each drawn row was first read.
  const [visible, setVisible] = useState<readonly LogRow<ProjectAt>[]>(NO_ROWS);
  const onVisible = useCallback((drawn: readonly LogRow<ProjectAt>[]) => { setVisible(drawn); onDrawn(rows, drawn, from); }, [rows, onDrawn, from]);
  const namingActive = active && inBranch && log != null && match.ready;
  const wanted = useMemo(() => namingActive
    ? logCountryRequests(visible, row => ({ hour: row.item.project.firstRead, tags: [row.item.project.tag] })) : [], [visible, namingActive]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, namingActive);
  const country = useCallback((at: ProjectAt) => labelAt(at.project.firstRead)(at.project.tag), [labelAt]);
  const wantedStates = useMemo(() => stateNameRequests(visible.map(row => row.item.project), namingActive ? hour : null), [visible, hour, namingActive]);
  const stateAt = useStateNamesAt(runtime.client, track, wantedStates, namingActive);
  const chips = useMemo(() => {
    return buildingChoices(manifest).map(choice => ({ ...choice, unavailable: storableToken(choice.value) ? undefined : UNSTORABLE }));
  }, [manifest]);
  const warnings = useMemo(() => [...new Set([...manifest?.warnings ?? [], ...[...log?.windows.values() ?? []].flatMap(reply => reply.warnings)])],
    [manifest, log]);
  useDiagnostics(onDiagnostics, warnings);

  if (!track) return <WidgetState kind="empty" icon="hammer" message="Load a campaign to see its construction." />;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (filtered && groups.error) return <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} />;
  if (!manifest || !match.ready) return <WidgetState kind="loading" label="Loading the construction log…" />;
  if (match.reason) return <WidgetState kind="unavailable" message={match.reason} />;
  if (!manifest.recorded) return <WidgetState kind="unavailable" message="Construction was not recorded." />;
  if (!inBranch) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (!log || hour == null) return <WidgetState kind="loading" label="Loading the construction log…" />;

  const chosen = contentSet(types);
  const toggle = (token: string) => { const next = toggleContentSet(types, token, Number.MAX_SAFE_INTEGER); if (next != null) setOption?.('types', next); };
  const renderRow = (row: LogRow<ProjectAt>) => {
    const at = row.item;
    const type = projectType(log, at.project);
    const label = country(at);
    const levels = levelsLabel(at);
    const factories = factoriesShown(at);
    return <>
      <span className="log-art" aria-hidden="true">
        {type?.icon ? <img src={type.icon} alt="" /> : <ArtFallback icon="factory" />}
        {(at.repairing || (at.status !== 'building' && at.status !== 'unread' && at.kind === 'repair')) && manifest.repairIcon && <img className="log-art-overlay" src={manifest.repairIcon} alt="" />}
      </span>
      <span className="log-text">
        <CountryFlagUrl className="log-flag" url={label.flag} reason={label.flagReason} pending={label.flagPending} /><b>{label.name}</b>
        <span className="construction-muted"> · </span>{typeName(type)}{levels && <span className="construction-levels"> {levels}</span>}
        <span className="construction-muted"> in </span>{placeLabel(at.project, hour, stateAt)}
      </span>
      <span className={`log-tag construction-status is-${at.status}`}><i />{statusLabel(at)}</span>
      <span className={`log-value${factories == null ? ' is-muted' : ''}`} data-tooltip={factories == null ? 'Factories unknown' : 'Factories on it'}>
        <Icon name="hammer" className="construction-factory-icon" />{factories ?? '—'}
      </span>
    </>;
  };
  const menu = (row: LogRow<ProjectAt>) => jumpHours(row.item, hour).map(at => jumpItem(runtime, track.id, at));
  // Older rows may still be on their way while earlier windows load.
  const empty = !logComplete(log, from) ? 'Loading earlier construction…' : chosen.length || projects !== 'all' || from != null
    ? 'No construction matches these settings yet.' : widget.countries.kind === 'all' ? 'No construction yet.' : 'No construction by these countries yet.';
  return <div className="log-widget construction-log">
    {settings.chips && chips.length > 1 && <div className="log-chips">
      <ChipSet label="Building types" max={Number.MAX_SAFE_INTEGER} selected={chosen.length ? chosen : ['']}
        chips={[{ value: '', label: 'All types' }, ...chips]}
        onToggle={value => { if (value === '') { if (chosen.length) setOption?.('types', ''); } else toggle(value); }} />
    </div>}
    <LogList rows={rows} label="Construction projects" density={settings.density} months={settings.months}
      animate={settings.animate && live} columns={COLUMNS} renderRow={renderRow}
      describe={row => describeRow(log, row, country(row.item).name, hour, stateAt)}
      tooltip={row => projectTooltip(log, row.item, hour, country(row.item).name, stateAt)}
      menu={menu} empty={empty} onVisible={onVisible} />
  </div>;
}
