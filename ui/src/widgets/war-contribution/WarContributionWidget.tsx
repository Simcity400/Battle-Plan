import { countryFromKey } from '../../dashboard/countryKey.ts';
import { countryNameDetails } from '../../charts/tooltip.ts';
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import WidgetState from '../../components/WidgetState';
import { Segmented } from '../../components/Segmented';
import { ButterflyChart } from '../../charts/butterfly';
import type { ButterflyPartHit } from '../../charts/butterfly';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { LiveCell } from '../../charts/live';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { useChartTokens } from '../../charts/useChartTokens';
import { useHolderLooks } from '../../dashboard/holderLooks';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { useProduct } from '../../dashboard/products';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { countryIdentityIndex } from '../../dashboard/countryIdentity';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useShownHour } from '../../dashboard/widgetPerf';
import { useHeadHour } from '../army-size/headHour';
import type { WarContributionsReply, WarsReply } from '../../generated/dashboard';
import { joinedAt, observationAt, validateWarContributions, validateWarsReply, warContributions } from '../wars/warsData';
import { OPTIONS, SHOW_CHOICES } from './definition';
import { chosenWar, contributionItems, contributionRows, contributionStructure, formatContribution, readHourFor, shareText, sideAmount, warName } from './warContributionData';
import type { ContributionItem, Looks, Show, SideTotal } from './warContributionData';
import './warContribution.css';
import { productRequest } from '../../dashboard/request.ts';

const VALUES_WIDTH = 420;
const NO_ITEMS: readonly ContributionItem[] = [];

export default function WarContributionWidget({ widget, runtime, active, headerSlot, setOption, onDiagnostics }: WidgetProps) {
  const clock = runtime.clock;
  const hoursPerSecond = usePlaybackMotion(clock);
  const options = resolveOptions(OPTIONS, widget.options);
  const show: Show = options.show === 'totals' ? 'totals' : 'share';
  const tokens = useChartTokens();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const following = widget.time.mode === 'follow';
  const viewpointTag = selectedCountryKeys(widget.countries, track)[0] ?? null;
  const structural = useProduct<WarsReply>(runtime.client, track ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'wars',
    params: { branchId: track.branch.id, viewpoint: viewpointTag === null ? null : countryFromKey(viewpointTag) } }) : null, active, validateWarsReply, track?.source);
  const saved = typeof options.war === 'string' && options.war ? contentKey(options.war) : null;
  const trackHour = useCallback((clockHour: number) => {
    const window = track && resolveWindow(widget, track, runtime.tracks, clockHour);
    return window ? Math.floor(window.lastHour) : null;
  }, [track, widget, runtime.tracks]);
  const clockHour = useHeadHour(clock, active && following, hour => {
    const at = trackHour(hour);
    const war = at === null || !structural.data ? null : chosenWar(structural.data, saved, at);
    if (at === null || !war) return String(at === null ? '' : 'none');
    return `${war.id}:${war.observations.findIndex(item => item.firstHour <= at && at <= item.lastHour)}:${at > war.lastHour}`;
  });
  const window = track && resolveWindow(widget, track, runtime.tracks, clockHour);
  const at = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const war = structural.data ? chosenWar(structural.data, saved, at) : null;
  const readHour = war ? readHourFor(war, at) : null;
  const going = war !== null && readHour === at;
  const looksRef = useRef<{ looks: Looks; show: Show; muted: string }>(null!);
  const live = useMemo(() => new LiveCell<readonly ContributionItem[]>(NO_ITEMS), []);
  const latest = useRef<WarContributionsReply | undefined>(undefined);
  const shown = useShownHour();
  const warRef = useRef(war);
  warRef.current = war;
  const groupOf = useCallback((reply: WarContributionsReply) => {
    const held = warRef.current;
    return held ? warContributions(reply.contributions, observationAt(held, reply.hour)) : null;
  }, []);
  const paint = useCallback((reply: WarContributionsReply) => {
    latest.current = reply;
    const group = groupOf(reply);
    const { looks, show: shownAs, muted } = looksRef.current;
    live.set(group && !group.reason ? contributionItems(contributionRows(group), shownAs, looks, muted) : NO_ITEMS);
    shown(reply.hour);
  }, [groupOf, live, shown]);
  const gate = useGate(runtime, widget);
  const head = useHeadProduct(runtime, gate, 'war-contributions', going ? { viewpoint: viewpointTag === null ? null : countryFromKey(viewpointTag) } : null, {
    active, validate: validateWarContributions, paint, structure: reply => contributionStructure(groupOf(reply), reply.available),
  });
  const endedAt = gate.recorded(readHour);
  const ended = useProduct<WarContributionsReply>(runtime.client, track && war && !going && endedAt !== null ? productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'war-contributions', params: { branchId: track.branch.id, viewpoint: viewpointTag === null ? null : countryFromKey(viewpointTag), hour: endedAt } }) : null, active, validateWarContributions, track?.source);
  const reply = going ? head.data : ended.data;
  const error = going ? head.error : ended.error;
  const current = reply && war && war.firstHour <= reply.hour && reply.hour <= war.lastHour ? reply : undefined;
  const observation = current && war ? observationAt(war, current.hour) : null;
  const group = current ? warContributions(current.contributions, observation) : null;
  const rows = useMemo(() => group && !group.reason ? contributionRows(group) : [], [group]);
  const tags = useMemo(() => [...new Set(rows.flatMap(row => [...row.left.members, ...row.right.members].map(member => member.tag)))], [rows]);
  const holders = useMemo(() => current ? tags.map(tag => ({ key: tag, tag, hour: current.hour })) : [], [tags, current?.hour]);
  const looks = useHolderLooks(runtime.client, track, holders, active, tokens);
  looksRef.current = { looks, show, muted: tokens.muted };
  const items = useMemo(() => contributionItems(rows, show, looks, tokens.muted), [rows, show, looks, tokens.muted]);
  useLayoutEffect(() => {
    if (current) latest.current = current;
    live.set(items);
    if (!going) shown(current ? current.hour : null);
  }, [items, live, current, going, shown]);
  const groups = useCountryGroups(runtime, track, active);
  const identities = useMemo(() => groups.data ? countryIdentityIndex(groups.data.countryIdentities) : null, [groups.data]);
  const [width, setWidth] = useState(0);
  const root = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const element = root.current;
    if (!element) return;
    const observer = new ResizeObserver(() => setWidth(element.clientWidth));
    observer.observe(element); setWidth(element.clientWidth);
    return () => observer.disconnect();
  }, [rows.length > 0]);
  useEffect(() => { head.root(root.current); });
  const values = width >= VALUES_WIDTH;
  const diagnostics = useMemo(() => [...new Set([...(structural.data?.warnings ?? []), ...(current?.warnings ?? []),
    ...(group?.reason ? [group.reason] : [])])], [structural.data, current, group]);
  useDiagnostics(onDiagnostics, diagnostics);
  const tooltip = useCallback((item: ContributionItem, part?: ButterflyPartHit | null): TooltipContent | null => {
    const held = latest.current;
    if (!held) return null;
    const participation = item.row.key === 'participation';
    const format = (raw: bigint) => formatContribution(raw, participation);
    const sideRows = (label: string, side: SideTotal): TooltipRow[] => [
      { label, value: side.members.length ? format(side.total) : 'Not recorded', strong: true, divider: true },
      ...(side.unknown != null && side.unknown !== 0n ? [{ label: 'Unknown casualties contribution', value: formatContribution(side.unknown, false), indent: true, tone: 'muted' as const }] : []),
    ];
    if (part) {
      const side = part.side === 'left' ? item.row.left : item.row.right;
      const found = partAt(side, part.index);
      if (found) {
        const value = found.unknown ? found.member.unknown ?? 0n : found.member.value;
        return { ...countryNameDetails({ nameFailure: looks.nameFailure?.(found.member.tag) }), title: looks.name(found.member.tag), subtitle: `${item.label} · ${dayLabel(held.hour)}`, rows: [
          { label: found.unknown ? 'Unknown casualties contribution' : item.label, value: format(value) },
          { label: 'Share of side', value: shareText(value, sideAmount(side)) },
        ] };
      }
    }
    return { title: item.label, subtitle: `${warName(observation)} · ${dayLabel(held.hour)}`,
      rows: [...sideRows('Viewpoint’s side', item.row.left), ...sideRows('Opposing side', item.row.right)] };
  }, [observation, looks]);
  const menu = useCallback((item: ContributionItem) => {
    const held = latest.current;
    if (!war || !track || !held) return [];
    const joins = identities ? [...new Set([...item.row.left.members, ...item.row.right.members].map(member => member.tag))].flatMap(tag => {
      const joined = joinedAt(war, tag, held.hour, identities);
      return joined && joined.firstHour !== war.firstHour ? [{ ...jumpItem(runtime, track.id, joined.firstHour),
        label: `Jump to ${looks.name(tag)} ${joined.joinedBy ? 'joined by' : 'joining'} ${dayLabel(joined.firstHour)}` }] : [];
    }) : [];
    return [{ ...jumpItem(runtime, track.id, war.firstHour), label: `Jump to war start ${dayLabel(war.firstHour)}` }, ...joins];
  }, [war, track, runtime, looks, identities]);
  const header = headerSlot && war && createPortal(<span className="pill" title={warName(observation)}>{warName(observation ?? war.observations.at(-1) ?? null)}</span>, headerSlot);
  if (!track) return <WidgetState kind="empty" icon="swords" message="Load a campaign to see war contributions." />;
  if (!window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (structural.error) return <WidgetState kind="error" failure={structural.error} onRetry={structural.retry} />;
  if (error && !current) return <WidgetState kind="error" failure={error} onRetry={going ? head.retry : ended.retry} />;
  if (!structural.data) return <WidgetState kind="loading" label="Loading war contributions…" />;
  if (!structural.data.available) return <WidgetState kind="unavailable" message="Wars are not in this recording." />;
  if (!war) return <WidgetState kind="empty" icon="swords" message={following ? 'This country has fought no war yet.' : 'This country has fought no war in this campaign.'} />;
  if (!current) return <>{header}<WidgetState kind="loading" label="Loading war contributions…" /></>;
  if (!observation) return <>{header}<WidgetState kind="unavailable" message="This war was not recorded at this time." /></>;
  if (!group || group.reason) return <>{header}<WidgetState kind="unavailable" message="This war’s contributions were not recorded at this time." /></>;
  return <div ref={root} className="war-contribution">
    {header}
    {setOption && <div className="war-contribution-caption"><Segmented value={show} options={SHOW_CHOICES} label="Show" onChange={value => setOption('show', value)} /></div>}
    <ButterflyChart key={JSON.stringify([track.source.id, track.source.revision, track.branch.id, war.id])} label={`Contributions to ${warName(observation)}`}
      items={items} liveItems={live} leftLabel="◀ Viewpoint’s side" rightLabel="Opposing side ▶" ratioLabel="" showRatio={false} showFlags={false}
      showValues={values} axis={show === 'share'} formatTick={value => `${Math.round(value * 100)}%`}
      active={active} hoursPerSecond={hoursPerSecond} tooltip={tooltip} contextItems={menu} />
  </div>;
}

function partAt(side: SideTotal, index: number) {
  let at = 0;
  for (const member of side.members) {
    if (at === index) return { member, unknown: false };
    at++;
    if (member.unknown != null && member.unknown !== 0n) { if (at === index) return { member, unknown: true }; at++; }
  }
  return null;
}
