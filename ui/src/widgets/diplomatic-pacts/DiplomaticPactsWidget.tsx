import { countryFromKey, isSlotKey } from '../../dashboard/countryKey.ts';
import { keyNameAt, useSlotTags } from '../../dashboard/countrySlots.ts';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import Timeline from '../../charts/timeline/Timeline';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { useChartTokens } from '../../charts/useChartTokens';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions } from '../../dashboard/options';
import { useHeadHour } from '../army-size/headHour';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { changesBy } from '../../charts/timeline/runState';
import { useProduct } from '../../dashboard/products';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { OPTIONS } from './definition';
import { aboutElsewhere, agreementLanes, agreementsOf, changeHours, validatePacts } from './pactsData';
import './diplomaticPacts.css';
import { motionOf } from '../../charts/easing.ts';
import type { DiplomacyPactsReply } from '../../generated/dashboard';
import { useChangedState } from '../../dashboard/changedState.ts';
import { recordedHour } from '../../dashboard/regionTime.ts';
import { productRequest } from '../../dashboard/request.ts';

const NO_WARNINGS: string[] = [];

export default function DiplomaticPactsWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const clock = runtime.clock;
  const followsClock = widget.time.mode === 'follow';
  const structure = useRef<(hour: number) => string>(() => '');
  const playbackHour = useHeadHour(clock, active && followsClock, hour => structure.current(hour));
  const options = resolveOptions(OPTIONS, widget.options);
  const tokens = useChartTokens();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const first = widget.time.mode === 'range' && window ? window.firstHour : track?.branch.firstHour ?? 0;
  const last = widget.time.mode === 'range' && window ? window.lastHour : track?.branch.lastHour ?? 0;
  const at = Math.floor(playhead ?? last);
  const extent: [number, number] = [first, Math.max(first + 1, last + 1)];
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] ?? null : defaultCountryKey(track);
  const reply = useProduct<DiplomacyPactsReply>(runtime.client, track && tag ? productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'diplomatic-pacts', params: { branchId: track.branch.id, country: countryFromKey(tag) } }) : null, active, validatePacts, track?.source);
  const data = reply.data;
  const axisOffset = track?.axisOffset ?? 0;
  const changes = useMemo(() => data ? changeHours(data) : [], [data]);
  structure.current = hour => `${changesBy(changes, Math.floor(hour + axisOffset))}:${clock.getSnapshot().playing ? '' : hour}`;
  const liveHead = useMemo(() => followsClock && track ? { read: () => clock.getSnapshot().position + axisOffset,
    subscribe: (changed: () => void) => clock.subscribeFrame(changed) } : undefined, [followsClock, track, clock, axisOffset]);
  useShownEachFrame(clock, () => data && track ? liveHead ? recordedHour(track, liveHead.read()) : at : null);
  const kinds = useMemo(() => { const chosen = contentSet(options.kinds).map(contentKey); return chosen.length ? new Set(chosen) : null; }, [options.kinds]);
  const bySlot = tag !== null && isSlotKey(tag);
  const slots = useSlotTags(runtime.client, track, active && bySlot);
  const pacts = useMemo(() => !data || !tag || !bySlot ? data?.pacts ?? [] : data.pacts.map(pact => {
    const own = keyNameAt(slots, tag, pact.lastSeen);
    return own === null ? pact : { ...pact, first: pact.first === own ? tag : pact.first, second: pact.second === own ? tag : pact.second };
  }), [data, tag, bySlot, slots]);
  const agreements = useMemo(() => data && tag ? agreementsOf(pacts, tag, at, kinds) : [], [data, pacts, tag, at, kinds]);
  const [inView, setInView] = useChangedState<[number, number]>([0, 40]);
  const [shown, setShown] = useState('[]');
  const wanted = useMemo(() => [...(tag ? [{ hour: at, tag }] : []),
    ...(JSON.parse(shown) as [string, number][]).map(([partner, lastSeen]) => ({ hour: Math.min(at, lastSeen), tag: partner }))], [shown, tag, at]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const country = useCallback((partner: string | null, hour: number) => {
    if (!partner) return { name: 'Country not recorded', flagReason: 'Country not recorded' };
    const named = labelAt(Math.min(at, hour))(partner);
    return named.known === undefined ? { ...named, name: 'Loading name…' } : named;
  }, [labelAt, at]);
  const kindNames = useMemo(() => new Map(data?.kinds.map(kind => [kind.kind, kind.name]) ?? []), [data]);
  const { lanes, info } = useMemo(() => agreementLanes(agreements, options.ended !== 'hidden', track?.branch.lastHour ?? at, {
    name: tag ? country(tag, at).name : '', country, colors: tokens.categories,
    kindName: kind => kindNames.get(kind) ?? 'Relation name unavailable',
  }), [agreements, options.ended, track?.branch.lastHour, at, tag, country, tokens.categories, kindNames]);
  const visible = lanes.slice(Math.max(0, inView[0] - 8), inView[1] + 9).flatMap(lane => info.get(lane.id)?.agreement ?? []);
  const visibleKey = JSON.stringify(visible.flatMap(item => item.partner ? [[item.partner, item.lastSeen]] : []));
  useEffect(() => { setShown(visibleKey); }, [visibleKey]);
  const looksKey = visible.map(item => { const look = country(item.partner, item.lastSeen); return `${look.name}:${look.flag ? 1 : 0}`; }).join('|');
  const onLanesInView = useCallback((from: number, to: number) => setInView(previous => previous[0] === from && previous[1] === to ? previous : [from, to]), []);
  const diagnostics = useMemo(() => (data?.warnings ?? []).filter(warning => !aboutElsewhere(warning)), [data]);
  useDiagnostics(onDiagnostics, diagnostics.length ? diagnostics : NO_WARNINGS);
  const menu = useContextMenu('Timeline actions');

  if (!track) return <WidgetState kind="empty" icon="link" message="Load a campaign to see diplomacy." />;
  if (!window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (!tag) return <WidgetState kind="empty" icon="link" message="Choose a country in widget settings." />;
  if (reply.error) return <WidgetState kind="error" failure={reply.error} onRetry={reply.retry} />;
  if (!data || bySlot && !slots) return <WidgetState kind="loading" label="Loading diplomacy…" />;
  if (!lanes.length) return <WidgetState kind="empty" icon="link" message={`${country(tag, at).name} has no ${agreements.length ? 'agreements in force' : 'recorded agreements'} at this time.`} />;
  const tooltip = (laneId: string): TooltipContent | null => {
    const about = info.get(laneId);
    if (!about) return null;
    const item = about.agreement;
    const partner = country(item.partner, item.lastSeen);
    const state = item.state;
    const rows: TooltipRow[] = [
      { label: about.mutual ? 'With' : item.outgoing ? 'To' : 'From', value: partner.name, countryFlag: item.partner ? partner : undefined },
      { label: 'Since', value: `${item.sinceBound ? 'by ' : ''}${dayLabel(item.since)}` },
      ...(item.expires != null ? [{ label: 'Expires', value: dayLabel(item.expires) }] : []),
      state.state === 'present' ? { label: 'Status', value: 'In force' }
        : state.state === 'gone' ? { label: 'Ended', value: `by ${dayLabel(state.by)}` }
          : { label: 'Last recorded', value: dayLabel(state.since), tone: 'muted' as const },
    ];
    return { title: about.title, subtitle: dayLabel(at), rows };
  };
  const description = `${dayLabel(at)}: ${lanes.filter(lane => lane.heading && !lane.depth).map(lane => lane.identity?.label).join('. ')}.`;
  return <div className="diplomatic-pacts">
    <Timeline key={`${track.source.id}:${track.source.revision}:${track.branch.id}:${tag}`} label="Diplomatic agreements" extent={extent} playhead={playhead}
      liveHead={liveHead} labelHour={liveHead ? undefined : at} lanes={lanes} showIcons={false} showNames={false} description={description} unobserved={data.unobserved}
      hoursPerSecond={motionOf(runtime.clock.getSnapshot())} active={active} revision={looksKey}
      onLanesInView={onLanesInView} tooltip={lane => tooltip(lane.id)}
      onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />
    {menu.menu}
  </div>;
}
