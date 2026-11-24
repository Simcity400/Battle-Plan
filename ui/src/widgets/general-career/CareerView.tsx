import type { ReactNode } from 'react';
import { useCallback, useMemo, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { useContextMenu } from '../../components/ContextMenu';
import Timeline from '../../charts/timeline/Timeline';
import { playbackDomain } from '../../charts/view';
import { jumpItem } from '../../charts/jump';
import { liveKey } from '../../charts/live.ts';
import { countryNameDetails, fitTooltip } from '../../charts/tooltip.ts';
import { useViewportHeight } from '../../charts/useChartTokens';
import { formatDateTime } from '../../dashboard/calendar';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { WidgetProps } from '../../dashboard/definitions';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { useAtHead } from '../../dashboard/atHead';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { careerEmptyEntry, careerLanes, leaderName, recordedSince, runAt, traitDateText, validateCareer, validateLeaderHour } from '../generals/generalsData';
import { LeaderIdentity, leaderIdentityProps, leaderPortraitProps } from '../generals/LeaderIdentity.tsx';
import { useLeaderPortraits } from '../generals/portraits';
import type { PortraitAt } from '../generals/portraits';
import { experienceFactorText, experienceText } from '../generals/experience';
import { OPTIONS } from './definition';
import { LiveText } from './LiveText';
import '../generals/generals.css';
import type { LeaderCareerReply } from '../../generated/dashboard';
import { axisAt, regionWindow, SPEC, type RegionAction } from '../../dashboard/regionTime.ts';
import { useRegion, useRegionHour } from '../../dashboard/widgetTime.ts';
import { productRequest } from '../../dashboard/request.ts';

const NO_PORTRAIT: readonly PortraitAt[] = [];
const BRANCH_WIDE = { outside: false };

export default function CareerView({ reference, onPick, caption, widget, runtime, active, onDiagnostics, time = SPEC }: WidgetProps & {
  reference: string; onPick?: (hour: number) => void; caption?: ReactNode; time?: RegionAction;
}) {
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const career = useProduct<LeaderCareerReply>(runtime.client, track ? productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'unit-leader-career', params: { branchId: track.branch.id, reference } }) : null, active, validateCareer, track?.source);
  const params = useMemo(() => ({ reference }), [reference]);
  const region = useRegion(widget, time);
  const head = useHeadProduct(runtime, useGate(runtime, widget, time), 'unit-leader-hour', params, { active, validate: validateLeaderHour });
  const regionAt = useRegionHour(runtime, region, active);
  const data = career.data;
  const now = head.data;
  const first = track?.branch.firstHour ?? 0;
  const after = useAtHead(runtime.clock, active && head.outside && !!track,
    at => !!track && (regionWindow(region, track, runtime.tracks, axisAt(at))?.lastHour ?? -Infinity) > track.branch.lastHour);
  const shownAt = head.outside ? after ? track?.branch.lastHour ?? null : null : head.at ?? regionAt;
  const hour = shownAt ?? first - 1;
  const shown = useShownHour();
  const options = resolveOptions(OPTIONS, widget.options);
  const grow = options.growWithPlayback === true;
  const [hoveredHour, setHoveredHour] = useState<number | null>(null);
  const portraitAt = useMemo<readonly PortraitAt[]>(() => data?.portraitHour != null ? [{ reference, hour: data.portraitHour }] : NO_PORTRAIT, [data?.portraitHour, reference]);
  const portraits = useLeaderPortraits(runtime, track, portraitAt, 118, active);
  const lanes = useMemo(() => data ? careerLanes(data, first, hour, hour) : [], [data, first, hour]);
  const wanted = useMemo(() => {
    const at = [hour, ...hoveredHour === null ? [] : [hoveredHour]];
    return data ? at.flatMap(h => { const owner = runAt(data, h)?.role.owner; return owner ? [{ hour: h, tag: owner }] : []; }) : [];
  }, [data, hour, hoveredHour]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const viewportHeight = useViewportHeight();
  const menu = useContextMenu('Leader career actions');
  const diagnostics = useMemo(() => [...new Set([...(data?.warnings ?? []), ...(now?.warnings ?? []), ...portraits.warnings])], [data, now, portraits.warnings]);
  useDiagnostics(onDiagnostics, diagnostics);
  const tooltip = useCallback<NonNullable<Parameters<typeof Timeline>[0]['tooltip']>>((lane, span, h) => {
    if (!data) return null;
    const run = runAt(data, h);
    const owner = run?.role.owner;
    const country = owner ? labelAt(h)(owner) : { name: 'Owner not recorded' };
    const content = { ...countryNameDetails(country), title: run ? leaderName(data) : 'Leader not recorded', subtitle: `${country.name} · ${formatDateTime(h)}`,
      rows: [{ label: lane.label, value: span?.label ?? careerEmptyEntry(data, lane.id, h) }],
      note: lane.id === 'battles' && !span && data.battlesPartialFrom !== null && data.battlesPartialFrom <= h ? 'Battle data is incomplete here.' : undefined };
    return fitTooltip([content], viewportHeight);
  }, [data, labelAt, viewportHeight]);
  if (!track) return <WidgetState kind="empty" message="Load a campaign to see a leader career." />;
  if (career.error) return <WidgetState kind="error" failure={career.error} onRetry={career.retry} />;
  if (head.error) return <WidgetState kind="error" failure={head.error} onRetry={head.retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading leader career…" read={BRANCH_WIDE} />;
  if (data.missing) return <WidgetState kind="unavailable" message="Career unavailable for this leader." detail={data.missing} />;
  const role = now?.role ?? null;
  const country = role?.owner ? labelAt(hour)(role.owner) : { name: 'Owner not recorded' };
  const decided = now ? now.won + now.lost : 0;
  const partial = now?.battlesPartialFrom != null && now.battlesPartialFrom <= hour;
  const progress = now?.progress ?? null;
  const report = () => shown(head.shownAt());
  return <div className="general-career">
    {caption && <p className="career-subject">{caption}</p>}
    {now && role && !now.missing ? <div className="career-top"><LeaderIdentity {...leaderIdentityProps({ name: leaderName(now), role, skill: now.skill }, country)}
      {...leaderPortraitProps(portraitAt[0] ? portraits.at(portraitAt[0]) : undefined)} showCountry />
      <div className="career-stats">
        <span title={partial ? 'Battle counts are incomplete: some battles were not recorded.' : undefined}><strong>{now.fought}</strong>fought</span>
        <span><strong>{now.won}</strong>won</span><span><strong>{now.lost}</strong>lost</span>
        <span><strong>{decided ? `${Math.round(now.won * 100 / decided)}%` : '—'}</strong>win rate</span>
        <span><strong>{now.fought - decided}</strong>unresolved</span>
        <span>{recordedSince(data.runs[0]?.firstHour ?? null, first)}</span>
        <span><strong><LiveText live={head.live} cell={liveKey(reference, 'experience')} onPaint={report}
          format={(value, missing) => value === undefined ? missing : experienceText(String(value), progress?.nextRequiredRaw ?? null)} /></strong>commander XP</span>
      </div></div> : head.outside ? <WidgetState kind="outside" read={head} /> : !now ? <WidgetState kind="loading" label="Loading leader details…" read={head} />
      : <p className="career-note">{now.missing ?? 'Leader details unavailable at this time.'}</p>}
    {progress && <div className="career-training">
      <strong>Trait training</strong>
      {progress.training === null ? <p>Unavailable</p> : progress.training.length === 0 ? <p>No traits in training</p>
        : <ul>{progress.training.map(trait => <li key={trait.key}>{trait.name ?? trait.token ?? 'Unknown trait'}: <LiveText live={head.live} cell={liveKey(trait.key, 'experience')}
          format={(value, missing) => value === undefined ? missing : experienceText(String(value), trait.requiredRaw)} /></li>)}</ul>}
    </div>}
    {progress && <div className="career-training">
      <strong>Trait training modifiers</strong>
      {progress.factors === null ? <p>Unavailable</p> : progress.factors.length === 0 ? <p>No individual training modifiers</p>
        : <ul>{progress.factors.map((trait, index) => <li key={index}>{trait.name ?? trait.token ?? 'Unknown trait'}: {experienceFactorText(trait.factorRaw)}</li>)}</ul>}
    </div>}
    {now && now.traits && now.traits.length > 0 && <div className="career-training">
      <strong>Traits</strong>
      <ul>{now.traits.map((at, i) => { const trait = now.traitCatalogue[at];
        return <li key={trait.token}>{trait.name ?? trait.token}: {traitDateText(now.traitDates[i] ?? null)}</li>; })}</ul>
    </div>}
    <Timeline label="General career" extent={playbackDomain(grow ? 'draw-in' : 'cursor',
      [first, Math.max(first + 1, track.branch.lastHour + 1)], hour + 1, 0, 1)} playhead={head.outside || shownAt === null ? null : hour + 1} labelHour={head.outside || shownAt === null ? undefined : hour}
      preserveZoomOnExtentChange={grow}
      lanes={lanes} showIcons showNames emptyLabel="Not recorded" active={active} revision={[data, labelAt]} tooltip={tooltip}
      description={`${leaderName(data)}: skill, traits, commands and battles.`}
      hoursPerSecond={hoursPerSecond}
      dismissPointerTooltipOnEscape
      onHover={setHoveredHour}
      onContextMenu={(h, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, h), ...(onPick ? [{ label: 'Show this hour', run: () => onPick(h) }] : [])])} />
    {partial && <p className="career-note">Battle counts are incomplete.</p>}
    {role?.kind === 2 && <p className="career-note">Fleet commands cover fleets the recording names this leader for.</p>}
    {menu.menu}
  </div>;
}
