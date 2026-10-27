import { useCallback, useMemo, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { useParityRows } from '../../dashboard/parityRows';
import { useContextMenu } from '../../components/ContextMenu';
import { PreparedKey } from '../../charts/PreparedKey';
import { pendingAt, preparedPrefix } from '../../dashboard/preparedPrefix';
import Timeline from '../../charts/timeline/Timeline';
import { playbackDomain } from '../../charts/view';
import { useTableHour } from '../../charts/table/useTableHour';
import { jumpItem } from '../../charts/jump';
import { fitTooltip } from '../../charts/tooltip';
import { useViewportHeight } from '../../charts/useChartTokens';
import { formatDateTime } from '../../dashboard/calendar';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackSpeed } from '../../dashboard/playback';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { careerCountryFacts, careerLanes, leaderAt, leaderName, careerEmptyEntry, recordedSince } from '../generals/generalsData';
import { LeaderIdentity } from '../generals/LeaderIdentity';
import { useLeaderPortraits } from '../generals/portraits';
import { useLeaders } from '../generals/useLeaders';
import { OPTIONS } from './definition';
import '../generals/generals.css';
import { experienceText, experienceFactorText } from '../generals/experience';

/** One view for the leaderboard drill and the standalone career widget. */
export default function CareerView({ reference, hour, onPick, caption, widget, runtime, active, onDiagnostics }: WidgetProps & {
  reference: string; hour: number; onPick?: (hour: number) => void; caption?: string;
}) {
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const playback = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const hoursPerSecond = usePlaybackSpeed(runtime.clock);
  const selection = useMemo(() => ({ kind: 'leader' as const, reference }), [reference]);
  const { data, error, retry } = useLeaders(runtime, widget.trackIds[0], selection, 'career', active);
  const prefix = useMemo(() => preparedPrefix(data), [data]);
  const preparing = pendingAt(prefix, hour);
  const leader = data?.leaders.find(l => l.reference === reference);
  const [hoveredHour, setHoveredHour] = useState<number | null>(null);
  const row = useMemo(() => data && leader && !preparing ? leaderAt(data, leader, hour) : null, [data, leader, hour, preparing]);
  const portraitRows = useMemo(() => row ? [row] : [], [row]);
  const portraits = useLeaderPortraits(runtime, track, portraitRows, 118, active);
  const window = track && resolveWindow(widget, track, runtime.tracks, playback);
  const options = resolveOptions(OPTIONS, widget.options);
  const grow = options.growWithPlayback === true;
  const first = track?.branch.firstHour ?? 0;
  const last = Math.min(hour, window?.lastHour ?? hour);
  const lanes = useMemo(() => data && leader ? careerLanes(data, leader, first, last, hour) : [], [data, leader, first, last, hour]);
  const wanted = useMemo(() => data && leader ? careerCountryFacts(data, leader, hour, hoveredHour) : [], [data, leader, hour, hoveredHour]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const viewportHeight = useViewportHeight();
  const menu = useContextMenu('Leader career actions');
  const diagnostics = useMemo(() => [...new Set([...(data?.warnings ?? []), ...portraits.warnings])], [data, portraits.warnings]);
  useDiagnostics(onDiagnostics, diagnostics);
  const tooltip = useCallback<NonNullable<Parameters<typeof Timeline>[0]['tooltip']>>((lane, span, h) => {
    if (!data || !leader) return null;
    if (pendingAt(prefix, h)) return { title: 'Leader career', subtitle: formatDateTime(h),
      rows: [{ label: lane.label, value: 'Still being prepared' }] };
    const hovered = leaderAt(data, leader, h);
    const owner = hovered?.value.owner;
    const country = owner ? labelAt(h)(owner) : { name: 'Owner not recorded' };
    const command = lane.id === 'commands' && span ? leader.commands.find(c => `command:${c.formation}:${c.firstHour}` === span.id) : undefined;
    const unknownName = command && (command.nameReadHour == null || h < command.nameReadHour || command.nameUnknown.some(r => r.firstHour <= h && h <= r.lastHour));
    const entry = unknownName ? `Unnamed ${command.kind === 'fleet' ? 'fleet' : 'formation'}` : span?.label;
    const content = { title: hovered ? leaderName(hovered) : 'Leader not recorded', subtitle: `${country.name} · ${formatDateTime(h)}`,
      rows: [{ label: lane.label, value: entry ?? careerEmptyEntry(data, leader, lane.id, h) }],
      note: lane.id === 'battles' && !leader.battleWon && !span ? 'Battle results are not in this recording'
        : lane.id === 'commands' && hovered?.value.kind === 2 ? data.navyCommandsMissing ? 'Fleet command history is incomplete.' : undefined : undefined };
    return fitTooltip([content], viewportHeight);
  }, [data, leader, labelAt, viewportHeight, prefix]);
  const parityRoot = useRef<HTMLDivElement>(null);
  useParityRows(parityRoot, () => ({ kind: 'chart', label: 'General career values', complete: !!data,
    total: lanes.length, rows: [...lanes], hour, leader: leader ?? null, current: row ?? null }),
    { ready: () => !!data, error: () => error ?? null });
  if (!track) return <WidgetState kind="empty" message="Load a campaign to see a leader career." />;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading leader career…" />;
  if (!leader && preparing) return <WidgetState kind="loading" label="This range is still being prepared." />;
  if (!leader) return <WidgetState kind="unavailable" message="Career unavailable for this leader." />;
  const country = row?.value.owner ? labelAt(hour)(row.value.owner) : { name: 'Owner not recorded' };
  return <div className="general-career" ref={parityRoot}>
    {caption && <p className="career-subject">{caption}</p>}
    {row ? <div className="career-top"><LeaderIdentity art={portraits.at(row)} row={row} country={country} showCountry />
      <div className="career-stats">
        <span><strong>{row.preparing ? 'Still being prepared' : row.fought}</strong>fought</span>
        {row.won == null ? null
          : <><span><strong>{row.won}</strong>won</span><span><strong>{row.lost}</strong>lost</span><span><strong>{row.winRate == null ? '—' : `${Math.round(row.winRate)}%`}</strong>win rate</span></>}
        <span><strong>{row.preparing ? 'Still being prepared' : row.unresolved}</strong>unresolved</span>
        <span>{recordedSince(row.career, data.firstHour)}</span>
        <span><strong>{row.value.progress ? experienceText(row.value.progress.experienceRaw, row.value.progress.nextRequiredRaw) : 'Unavailable'}</strong>commander XP</span>
      </div></div> : <p className="career-note">{preparing ? 'Leader details are still being prepared at this time.' : 'Leader details unavailable at this time.'}</p>}
    {row?.value.progress && <div className="career-training">
      <strong>Trait training</strong>
      {row.value.progress.training === null ? <p>Unavailable</p> : row.value.progress.training.length === 0 ? <p>No traits in training</p>
        : <ul>{row.value.progress.training.map((trait, index) => <li key={index}>{trait.name ?? trait.token ?? 'Unknown trait'}: {experienceText(trait.experienceRaw, trait.requiredRaw)}</li>)}</ul>}
    </div>}
    {row?.value.progress && <div className="career-training">
      <strong>Trait training modifiers</strong>
      {row.value.progress.factors === null ? <p>Unavailable</p> : row.value.progress.factors.length === 0 ? <p>No individual training modifiers</p>
        : <ul>{row.value.progress.factors.map((trait, index) => <li key={index}>{trait.name ?? trait.token ?? 'Unknown trait'}: {experienceFactorText(trait.factorRaw)}</li>)}</ul>}
    </div>}
    <Timeline label="General career" extent={playbackDomain(grow ? 'draw-in' : 'cursor',
      [first, Math.max(first + 1, track.branch.lastHour + 1)], last + 1, 0, 1)} playhead={last + 1} labelHour={last}
      preserveZoomOnExtentChange={grow}
      pending={prefix?.pending} unobserved={prefix?.pending}
      lanes={lanes} showIcons showNames emptyLabel="Not recorded" active={active} revision={[data, labelAt]} tooltip={tooltip}
      description={`${row ? leaderName(row) : 'Leader'}: skill, traits, commands and battles.`}
      hoursPerSecond={hoursPerSecond}
      dismissPointerTooltipOnEscape
      onHover={setHoveredHour}
      onContextMenu={(h, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, h), ...(onPick ? [{ label: 'Show this hour', run: () => onPick(h) }] : [])])} />
    <PreparedKey pending={prefix?.pending} />
    {row && !row.preparing && row.won == null && <p className="career-note">Result not recorded</p>}
    {row?.foughtPartial && <p className="career-note">Battle counts are incomplete.</p>}
    {row?.value.kind === 2 && data.navyCommandsMissing && <p className="career-note">Fleet command history is incomplete.</p>}
    {menu.menu}
  </div>;
}
