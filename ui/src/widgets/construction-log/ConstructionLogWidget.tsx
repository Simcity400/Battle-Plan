import { useCallback, useEffect, useMemo, useState } from 'react';
import type { ReactNode } from 'react';
import { CountryFlagUrl } from '../../components/CountryFlag';
import { ChipSet } from '../../components/ChipSet';
import ContentSetPicker from '../../components/ContentSetPicker';
import { Icon } from '../../components/Icons';
import ArtFallback from '../../components/ArtFallback';
import WidgetState from '../../components/WidgetState';
import LogList from '../../charts/log/LogList';
import type { LogRow } from '../../charts/log/logData';
import { logSettings } from '../../charts/log/logOptions';
import { jumpItem } from '../../charts/jump';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { selectedTags } from '../../dashboard/countrySelection';
import { stateNameRequests } from '../../dashboard/construction';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { contentSet, resolveOptions, toggleContentSet } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useStateNamesAt } from '../../dashboard/useStateNamesAt';
import { useParityPreparation } from '../../dashboard/parityPreparation';
import type { LogListProps } from '../../charts/log/LogList';
import { OPTIONS, PROJECT_CHOICES } from './definition';
import { buildingChoices, describeRow, jumpHours, levelsLabel, logComplete, logRows, placeTitle, projectTooltip, projectType, rowCountryRequests, rowFactories, statusLabel, statusTone, typeName } from './constructionData';
import type { ProjectAt } from './constructionData';
import { useConstructionLog } from './useConstructionLog';
import './constructionLog.css';

const NO_ROWS: readonly LogRow<ProjectAt>[] = [];
const QUEUE_CHIP = [{ value: 'queue', label: 'In the queue', hint: 'Only projects still in the queue at the moment shown' }];
const TYPES_FIELD = OPTIONS.find(field => field.key === 'types');
const EVERY_TYPE = TYPES_FIELD?.kind === 'content-set' ? TYPES_FIELD.automatic : 'Every type';

/** Every construction project of the chosen countries as a log: newest first, each
 * row the game's building art and name with its levels over where it is, the
 * country, a status pill and, while it is in the queue, the factories on it. With
 * Live on it follows playback, and a project enters at the top as its reads show it
 * start, build a level or end. */
export default function ConstructionLogWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const live = widget.time.mode === 'follow';
  usePlaybackHour(runtime.clock, active && live);
  const options = resolveOptions(OPTIONS, widget.options);
  const settings = logSettings(options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const filtered = widget.countries.kind !== 'all';
  // One country is named by the header chip, so its rows leave the country out.
  const oneCountry = widget.countries.kind === 'selected' || (widget.countries.kind === 'tags' && widget.countries.tags.length === 1);
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
  const queue = options.queue === true;
  const order = options.order === 'start' ? 'start' : 'change';
  const rows = useMemo(() => log && hour != null && match.ready
    ? logRows(log, hour, { types: contentSet(types), projects, queue, country: match.matches, from }, order) : NO_ROWS,
  [log, hour, match, types, projects, queue, from, order]);
  const parity = import.meta.env.MODE === 'parity' ? useParityPreparation() : null;
  // Countries are named and flagged as recorded when each drawn row was first read.
  const [visible, setVisible] = useState<readonly LogRow<ProjectAt>[]>(NO_ROWS);
  const onVisible = useCallback((drawn: readonly LogRow<ProjectAt>[]) => { setVisible(drawn); if (!parity?.requested) onDrawn(rows, drawn, from); }, [rows, onDrawn, from, parity?.requested]);
  const namingActive = active && inBranch && log != null && match.ready;
  // Each drawn row names its country at its own first read: every one of those hours is asked for.
  const names = useMemo(() => rowCountryRequests(namingActive ? visible : NO_ROWS), [visible, namingActive]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, names.wanted, namingActive, names.maxHours);
  const country = useCallback((at: ProjectAt) => labelAt(at.project.firstRead)(at.project.tag), [labelAt]);
  const wantedStates = useMemo(() => stateNameRequests(visible.map(row => row.item.project), namingActive ? hour : null), [visible, hour, namingActive]);
  const stateAt = useStateNamesAt(runtime.client, track, wantedStates, namingActive);
  let paritySemantic: LogListProps<ProjectAt>['paritySemantic'];
  if (import.meta.env.MODE === 'parity') {
    const requested = parity!.requested;
    const historyComplete = log != null && logComplete(log, from);
    useEffect(() => { if (requested) onDrawn(rows, rows, from); }, [requested, onDrawn, rows, from]);
    const complete = requested && historyComplete;
    paritySemantic = { complete, preparation: { prepare: parity!.prepare, ready: () => complete, error: () => error },
      content: row => { const type = log && projectType(log, row.item.project); return type ? { id: type.id, token: type.token, name: type.name } : null; },
      reads: { current: log?.current, held: log?.held, from,
        windows: [...log?.windows ?? []].sort(([a], [b]) => a - b).map(([index, reply]) => ({ index, interval: reply.interval,
          outputReads: reply.outputReads, outputUnavailable: reply.outputUnavailable })) } };
  }
  const choices = useMemo(() => buildingChoices(manifest), [manifest]);
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
    const factories = rowFactories(at);
    const tone = statusTone(at);
    // A name still being read shows a placeholder; a missing or failed one says so.
    const named = (placeholder: string): ReactNode => label.known === undefined
      ? <span className={`skeleton ${placeholder}`} aria-hidden="true" /> : <span className="construction-ellipsis">{label.name}</span>;
    const flag = <CountryFlagUrl className="log-flag" url={label.flag} reason={label.flagReason} pending={label.flagPending} />;
    return <>
      <span className="log-art" aria-hidden="true">
        {type?.icon ? <img src={type.icon} alt="" /> : <ArtFallback icon="factory" />}
        {(at.repairing || (at.status !== 'building' && at.status !== 'unread' && at.kind === 'repair')) && manifest.repairIcon && <img className="log-art-overlay" src={manifest.repairIcon} alt="" />}
      </span>
      <span className="construction-title">
        <span className="construction-building"><span className="construction-name">{typeName(type)}</span>
          {levels && <span className="construction-levels">{levels}</span>}</span>
        <span className="construction-place">
          {!oneCountry && <span className="construction-place-country">{flag}{named('construction-name-pending is-short')}<span aria-hidden="true">·</span></span>}
          <span className="construction-ellipsis">{placeTitle(at.project, hour, stateAt)}</span>
        </span>
      </span>
      {!oneCountry && <span className="construction-country">{flag}{named('construction-name-pending')}</span>}
      <span className="construction-status">
        <span className={`pill${tone === 'neutral' ? '' : ` ${tone}`}`}>{live && at.status === 'building' && <i className="live-dot" aria-hidden="true" />}<span>{statusLabel(at)}</span></span>
      </span>
      {factories == null ? <span /> : <span className={`log-value${factories.factories ? '' : ' is-muted'}`}
        data-tooltip={factories.factories == null ? 'Factories not recorded' : 'Factories on it'}>
        {factories.factories == null ? '—' : <><Icon name="hammer" className="construction-factory-icon" />{factories.factories}</>}
      </span>}
    </>;
  };
  const menu = (row: LogRow<ProjectAt>) => jumpHours(row.item, hour).map(at => jumpItem(runtime, track.id, at));
  // Older rows may still be on their way while earlier windows load.
  const empty = !logComplete(log, from) ? 'Loading earlier construction…' : chosen.length || projects !== 'all' || queue || from != null
    ? 'No construction matches these settings yet.' : widget.countries.kind === 'all' ? 'No construction yet.'
      : oneCountry ? 'No construction by this country yet.' : 'No construction by these countries yet.';
  return <div className={`log-widget construction-log${oneCountry ? ' is-one-country' : ''}`}>
    {settings.chips && <div className="log-chips construction-filters">
      <ChipSet label="Projects" chips={PROJECT_CHOICES} selected={[projects]} max={1} full={false} onToggle={value => setOption?.('projects', value)} />
      <ChipSet label="Queue" chips={QUEUE_CHIP} selected={queue ? ['queue'] : []} max={1} full={false} onToggle={() => setOption?.('queue', !queue)} />
      {choices.length > 0 && <ContentSetPicker label="Building types" automatic={EVERY_TYPE} choices={choices} selected={chosen}
        onReset={() => { if (chosen.length) setOption?.('types', ''); }} onToggle={toggle} />}
    </div>}
    <LogList rows={rows} label="Construction projects" density={settings.density} months={settings.months}
      {...import.meta.env.MODE === 'parity' ? { parityHistoryComplete: logComplete(log, from), paritySemantic } : {}}
      animate={settings.animate && live} renderRow={renderRow}
      describe={row => describeRow(log, row, country(row.item).name, hour, stateAt)}
      tooltip={row => projectTooltip(log, row.item, hour, country(row.item).name, stateAt)}
      menu={menu} empty={empty} onVisible={onVisible} />
  </div>;
}
