import { useId, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent, MouseEvent } from 'react';
import ArtFallback from '../../components/ArtFallback';
import { Icon } from '../../components/Icons';
import ListSkeleton from '../../components/Skeleton';
import WidgetState from '../../components/WidgetState';
import ProgressMeter from '../../charts/meter/ProgressMeter';
import { useOpenTooltip } from '../../charts/openTooltip.ts';
import type { TooltipContent } from '../../charts/tooltip';
import { contentTitle } from '../../dashboard/contentTitles';
import { openedOn } from '../../dashboard/gameWindows';
import type { WidgetProps } from '../../dashboard/definitions';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { researchArt, researchArtwork, researchLatestProgress, researchReadName, researchRecordedTemplates } from '../research/researchData';
import { researchDate } from '../research/researchLayout';
import { slotNumbers, slotsStructure } from '../research/researchSlots';
import { historyTokens, useResearchReply, useResearchSlots } from '../research/useResearch';
import { useTechnologyArtwork } from '../research/useTechnologyArtwork';
import { activeSlots, isOpenedResearch, openedResearch } from './activeResearchData';
import type { ActiveSlot, OpenedResearch } from './activeResearchData';
import ResearchWindow from './ResearchWindow';
import './activeResearch.css';

const NO_WARNINGS: string[] = [];
const NO_TOKENS: readonly string[] = [];

export default function ActiveResearchWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const owner = useId();
  const history = useResearchReply(widget, runtime, active);
  const { track, tag, data } = history;
  const root = useRef<HTMLElement | null>(null);
  const now = useResearchSlots(widget, runtime, active && !!data, slotsStructure, root);
  const { latest } = now;
  const reply = now.data;
  const rows = useMemo(() => data && reply ? activeSlots(data.history, reply) : [], [data, reply]);
  const tokens = useMemo(() => data ? [...new Set(historyTokens(data.history).map(use => typeof use === 'string' ? use : use[0]))] : NO_TOKENS, [data]);
  const recorded = useMemo(() => researchRecordedTemplates(data?.history), [data]);
  const images = useTechnologyArtwork(runtime.client, track, tokens, active && !!data, recorded);
  const diagnostics = useMemo(() => [...new Set([...(data?.warnings ?? NO_WARNINGS), ...(reply?.warnings ?? NO_WARNINGS), ...images.warnings])], [data, reply, images.warnings]);
  useDiagnostics(onDiagnostics, diagnostics);
  const artwork = useMemo(() => researchArtwork(data, images.artwork), [data, images.artwork]);
  const tips = useOpenTooltip(owner, now.live.subscribe);
  const untip = tips.hide;
  const [opened, setOpened] = useState<{ on: string; research: OpenedResearch; x: number; y: number } | null>(null);
  if (!track || !tag) return <div className="dash-empty" role="status"><span className="empty-icon"><Icon name="flask" /></span><p>Load a campaign and choose a country to see its research.</p></div>;
  if (now.read === 'off') return <WidgetState kind="outside" />;
  const failure = history.error ?? now.error;
  if (failure && (!data || !reply)) return <WidgetState kind="error" failure={failure} onRetry={history.error ? history.retry : now.retry} />;
  if (!data || !reply) return <ListSkeleton label="Loading research..." />;
  const describe = (research: OpenedResearch) => ({ icon: artwork.get(research.token)?.icon ?? undefined,
    name: contentTitle(research.firstObservedHour === null ? research.definition?.name : researchArt(artwork, research.token, research.databaseIndex)?.name,
      research.token, 'Technology name unavailable') });
  const open = (item: Extract<ActiveSlot, { kind: 'researching' }>, x: number, y: number) => { untip(); setOpened({ on: openedOn(track, tag), research: openedResearch(item), x, y }); };
  const researching = rows.filter((item): item is Extract<ActiveSlot, { kind: 'researching' }> => item.kind === 'researching');
  const row = (item: ActiveSlot) => {
    const key = `slot:${item.slot.index}`;
    if (item.kind === 'unavailable') return <div key={key} className="active-row is-quiet" role="status" title={item.reason}>
      <span className="active-row-art is-icon"><Icon name="clock" /></span><span className="active-row-name is-muted">Not recorded</span></div>;
    const numbers = slotNumbers(item.read, reply.hour);
    if (item.kind === 'idle') return <div key={key} className="active-row is-quiet" role="status" data-research-slot={item.slot.index}>
      <span className="active-row-art is-icon"><Icon name="pause" /></span><span className="active-row-name is-idle">Idle</span>
      <span className="active-row-saved" data-paint="saved">{numbers.saved}</span></div>;
    const { token, slot } = item;
    const name = contentTitle(researchReadName(artwork, item.read), token, 'Technology name unavailable'), icon = artwork.get(token)?.icon ?? undefined;
    const click = (event: MouseEvent<HTMLElement>) => open(item, event.clientX, event.clientY);
    const content = (): TooltipContent | null => {
      const at = latest.current ?? reply, hour = at.hour, current = at.slots.find(read => read.index === slot.index);
      const shown = latest.inspect(slot.index);
      if ((shown ? shown.token : current?.state.token) !== token || !current) return null;
      const full = data.history.slots.find(held => held.index === slot.index) ?? slot;
      const progress = shown ? shown.fraction === null ? null : { fraction: shown.fraction, hour: shown.hour }
        : researchLatestProgress({ ...full, states: [current.state], progress: current.progress ? [current.progress] : [] }, current.state, hour);
      const left = shown ? shown.left : slotNumbers(current, hour).left;
      return { title: name, icon, rows: [
        { label: 'Progress', value: progress ? `${Math.round(progress.fraction * 100)}% on ${researchDate(progress.hour)}` : 'Not recorded' },
        { label: 'Days left', value: left === null ? 'Not recorded' : `${left}` },
      ], note: left === null ? undefined : 'Days left is the game’s estimate at this hour’s research speed.' };
    };
    return <div key={key} className="active-row" data-research-slot={slot.index} tabIndex={0} aria-label={`${name}, researching`}
      onPointerMove={event => tips.show(content, key, event.clientX, event.clientY)} onPointerLeave={untip}
      onFocus={event => { const box = event.currentTarget.getBoundingClientRect(); tips.show(content, key, box.right, box.top); }} onBlur={untip}
      onKeyDown={(event: KeyboardEvent<HTMLElement>) => {
        if (event.key !== 'Enter') return;
        event.preventDefault(); const box = event.currentTarget.getBoundingClientRect(); open(item, box.left + 40, box.bottom);
      }}>
      <span className="active-row-art is-open" onClick={click}>{icon ? <img src={icon} alt="" /> : <ArtFallback icon="flask" />}</span>
      <span className="active-row-name is-open" onClick={click}>{name}</span>
      <ProgressMeter value={numbers.fraction} state="live" label={`${name} progress`} />
      <span className="active-row-percent" data-paint="percent">{numbers.percent}</span>
      <span className="active-row-days" data-paint="days">{numbers.days}</span>
    </div>;
  };
  const shown = opened?.on === openedOn(track, tag) ? opened : null;
  const popup = shown && <ResearchWindow key={JSON.stringify(shown.research)} runtime={runtime} widget={widget} track={track} tag={tag} data={data} reply={reply}
    opened={shown.research} item={researching.find(item => isOpenedResearch(item, shown.research)) ?? null}
    shown={describe(shown.research)} feed={now.feed} installed={now.inspected} subscribe={now.live.subscribe} active={active} anchor={shown} onClose={() => setOpened(null)} />;
  const idle = rows.filter(item => item.kind === 'idle').length;
  return <section className="active-research" aria-label="Active research" ref={now.root}>
    {failure && <WidgetState kind="error" failure={failure} onRetry={history.error ? history.retry : now.retry} />}
    <div className="active-rows">
      {rows.length > 0 && <p className="active-section">Researching<span>{researching.length}</span>{idle > 0 && <span className="is-idle">{idle} idle</span>}</p>}
      {rows.map(row)}
    </div>
    {!rows.length && <p>{reply.missing ?? <>No research slots recorded at <span data-paint="date">{researchDate(reply.hour)}</span>.</>}</p>}
    {popup}
  </section>;
}
