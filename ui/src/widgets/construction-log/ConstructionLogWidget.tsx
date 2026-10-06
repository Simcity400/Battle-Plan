import { useCallback, useEffect, useMemo, useState } from 'react';
import { ChipSet } from '../../components/ChipSet';
import { Icon } from '../../components/Icons';
import WidgetState from '../../components/WidgetState';
import LogList from '../../charts/log/LogList';
import type { LogRow } from '../../charts/log/logData';
import { logCountryRequests } from '../../charts/log/logData';
import { logSettings } from '../../charts/log/logOptions';
import { jumpItem } from '../../charts/jump';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { selectedTags } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { contentSet, resolveOptions, storableToken, toggleContentSet, UNSTORABLE } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { OPTIONS } from './definition';
import { buildingChoices, describeRow, factoriesShown, jumpHours, levelsLabel, logRows, placeLabel, projectTooltip, projectType, statusLabel, typeName } from './constructionData';
import type { ProjectAt } from './constructionData';
import { useConstructionLog } from './useConstructionLog';
import './constructionLog.css';

const NO_WARNINGS: string[] = [];
const NO_ROWS: readonly LogRow<ProjectAt>[] = [];
/** The cells after a row's date: artwork, the sentence, its status and its factories. */
const COLUMNS = '24px minmax(0, 1fr) auto 3.4em';

/** Every construction project of the chosen countries as a log (mock-up style-5):
 * newest first, each with the game's building art, where it is, its levels, when it
 * started and finished and the factories on it. With Live on it follows playback, and
 * a project enters at the top as its reads show it start, build a level or end. */
export default function ConstructionLogWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const log = logSettings(options);
  const { track, data, error, retry } = useConstructionLog(widget, runtime, active);
  const grouped = widget.countries.kind === 'group';
  const groups = useCountryGroups(runtime, track, active && grouped);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = window?.lastHour;
  const from = widget.time.mode === 'range' ? window?.firstHour ?? null : null;
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
  const rows = useMemo(() => data && hour != null && match.ready
    ? logRows(data, hour, { types: contentSet(types), projects, country: match.matches, from }, order) : NO_ROWS,
  [data, hour, match, types, projects, from, order]);
  // Countries are named and flagged as recorded when each drawn row was first read.
  const [visible, setVisible] = useState<readonly LogRow<ProjectAt>[]>(NO_ROWS);
  const wanted = useMemo(() => logCountryRequests(visible, row => ({ hour: row.item.project.firstRead, tags: [row.item.project.tag] })), [visible]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const country = useCallback((at: ProjectAt) => labelAt(at.project.firstRead)(at.project.tag), [labelAt]);
  const chips = useMemo(() => {
    return buildingChoices(data).map(choice => ({ ...choice, unavailable: storableToken(choice.value) ? undefined : UNSTORABLE }));
  }, [data]);
  useEffect(() => {
    onDiagnostics(data?.warnings.length ? data.warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, onDiagnostics]);

  if (!track) return <WidgetState kind="empty" icon="hammer" message="Load a campaign to see its construction." />;
  if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (!data || (grouped && !groups.data && !groups.error)) return <WidgetState kind="loading" label="Loading the construction log…" />;
  if (grouped && groups.error) return <WidgetState kind="error" message={`Groups unavailable: ${groups.error}`} onRetry={groups.retry} />;
  if (match.reason) return <WidgetState kind="unavailable" message={match.reason} />;
  if (!data.reads.length && !data.railwayReads.length) return <WidgetState kind="unavailable" message="The construction queue was not recorded in this campaign branch." />;
  if (hour == null || hour < track.branch.firstHour || hour > track.branch.lastHour) return <WidgetState kind="unavailable" message="This time is outside the campaign’s recorded branch." />;

  const chosen = contentSet(types);
  const toggle = (token: string) => { const next = toggleContentSet(types, token, Number.MAX_SAFE_INTEGER); if (next != null) setOption?.('types', next); };
  const renderRow = (row: LogRow<ProjectAt>) => {
    const at = row.item;
    const type = projectType(data, at.project);
    const label = country(at);
    const levels = levelsLabel(at);
    const factories = factoriesShown(at);
    return <>
      <span className={`log-art${type?.icon ? '' : ' is-missing'}`} aria-hidden="true">
        {type?.icon ? <img src={type.icon} alt="" /> : '?'}
        {(at.repairing || (at.status !== 'building' && at.status !== 'unread' && at.kind === 'repair')) && data.repairIcon && <img className="log-art-overlay" src={data.repairIcon} alt="" />}
      </span>
      <span className="log-text">
        {label.flag && <img className="log-flag" src={label.flag} alt="" />}<b>{label.name}</b>
        <span className="construction-muted"> · </span>{typeName(type)}{levels && <span className="construction-levels"> {levels}</span>}
        <span className="construction-muted"> in </span>{placeLabel(data, at.project)}
      </span>
      <span className={`log-tag construction-status is-${at.status}`}><i />{statusLabel(at)}</span>
      <span className={`log-value${factories == null ? ' is-muted' : ''}`} data-tooltip={factories == null ? 'Not read since a gap in the recording' : 'Factories on it'}>
        <Icon name="hammer" className="construction-factory-icon" />{factories ?? '—'}
      </span>
    </>;
  };
  const menu = (row: LogRow<ProjectAt>) => jumpHours(row.item, hour).map(at => jumpItem(runtime, track.id, at));
  const empty = chosen.length || projects !== 'all' || from != null
    ? 'No construction matches these settings yet.' : widget.countries.kind === 'all' ? 'No construction yet.' : 'No construction by these countries yet.';
  return <div className="log-widget construction-log">
    {log.chips && chips.length > 1 && <div className="log-chips">
      <ChipSet label="Building types" max={Number.MAX_SAFE_INTEGER} selected={chosen.length ? chosen : ['']}
        chips={[{ value: '', label: 'All types' }, ...chips]}
        onToggle={value => { if (value === '') { if (chosen.length) setOption?.('types', ''); } else toggle(value); }} />
    </div>}
    <LogList rows={rows} label="Construction projects" density={log.density} months={log.months}
      animate={log.animate && widget.time.mode === 'follow'} columns={COLUMNS} renderRow={renderRow}
      describe={row => describeRow(data, row, country(row.item).name)}
      tooltip={row => projectTooltip(data, row.item, hour, country(row.item).name)}
      menu={menu} empty={empty} onVisible={setVisible} />
  </div>;
}
