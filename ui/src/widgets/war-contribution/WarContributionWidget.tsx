import { useCallback, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import WidgetState from '../../components/WidgetState';
import { Segmented } from '../../components/Segmented';
import { ButterflyChart } from '../../charts/butterfly';
import type { ButterflyItem, ButterflyPartHit } from '../../charts/butterfly';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { useChartTokens } from '../../charts/useChartTokens';
import { formatAmount } from '../../charts/valueAxis';
import { useHolderLooks } from '../../dashboard/holderLooks';
import { selectedTags } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, resolveOptions } from '../../dashboard/options';
import { usePlaybackHour, usePlaybackMotion } from '../../dashboard/playback';
import { useFollowedProduct, useProduct } from '../../dashboard/products';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { countryIdentityIndex } from '../../dashboard/countryIdentity';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { currentWarsAt, joinedAt, observationAt, validateWarsReply, warContributions } from '../wars/warsData';
import { OPTIONS, SHOW_CHOICES } from './definition';
import { SCALE, chosenWar, contributionRows, readHourFor, rowLengths, shareText, sideAmount, sideMagnitude, toNumber, warName } from './warContributionData';
import type { ContributionRow, SideTotal } from './warContributionData';
import './warContribution.css';

type Show = 'share' | 'totals';
type Item = ButterflyItem & { row: ContributionRow };
/** Below this width the exact values at the bar tips move into the tooltip. */
const VALUES_WIDTH = 420;

const amount = (raw: bigint, digits = 2) => formatAmount(raw, SCALE, digits);
const sideText = (side: SideTotal, participation: boolean) => !side.members.length ? 'Not recorded'
  : participation ? `${amount(side.total * 100n)}%` : sideMagnitude(side) === 0n ? 'None yet' : amount(sideAmount(side));

export default function WarContributionWidget({ widget, runtime, active, headerSlot, setOption, onDiagnostics }: WidgetProps) {
  const playbackHour = usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const options = resolveOptions(OPTIONS, widget.options);
  const show: Show = options.show === 'totals' ? 'totals' : 'share';
  const tokens = useChartTokens();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const at = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const viewpointTag = selectedTags(widget.countries, track)[0] ?? null;
  const structural = useProduct(runtime.client, track ? { sourceId: track.source.id, revision: track.source.revision, product: 'wars',
    params: { branchId: track.branch.id, viewpointTag } } : null, active, validateWarsReply, track?.source);
  const saved = typeof options.war === 'string' && options.war ? contentKey(options.war) : null;
  const war = structural.data ? chosenWar(structural.data, saved, at) : null;
  const readHour = war ? readHourFor(war, at) : null;
  // The requested-hour groups, read from that hour's frame alone as Wars reads them.
  // The last read hour's groups stay shown while playback's newest is read; both halves use its hour.
  const exact = useFollowedProduct(runtime.client, track && readHour != null ? { sourceId: track.source.id, revision: track.source.revision, product: 'wars',
    params: { branchId: track.branch.id, viewpointTag, reads: [{ hour: readHour }] } } : null, ['reads'], active, validateWarsReply, track?.source);
  const shownHour = readHour == null ? null : exact.moment ? (JSON.parse(exact.moment) as [{ hour: number }[]])[0][0]?.hour ?? readHour : readHour;
  // The war each read hour was asked for: a reply still shown presents its own war.
  const asked = useRef(new Map<number, NonNullable<typeof war>>());
  if (war && readHour != null) { asked.current.delete(readHour); asked.current.set(readHour, war); }
  const askingHour = exact.asking ? (JSON.parse(exact.asking) as [{ hour: number }[]])[0][0]?.hour : undefined;
  for (const hour of asked.current.keys()) {
    if (asked.current.size <= 64) break;
    if (hour !== shownHour && hour !== readHour && hour !== askingHour) asked.current.delete(hour);
  }
  // A reply whose war is no longer known waits for one that is, rather than borrowing the newer war.
  const shownWar = shownHour == null || shownHour === readHour ? war : asked.current.get(shownHour) ?? null;
  const current = shownHour == null ? null : currentWarsAt(exact.data, shownHour);
  const observation = shownWar && shownHour != null ? observationAt(shownWar, shownHour) : null;
  // A war holding several native groups shows the largest within it, as Wars does.
  const group = current ? warContributions(current.contributions, observation) : null;
  const rows = useMemo(() => group && !group.reason ? contributionRows(group) : [], [group]);
  const tags = useMemo(() => [...new Set(rows.flatMap(row => [...row.left.members, ...row.right.members].map(member => member.tag)))], [rows]);
  // Colours and names for every participant, read at the war's read hour itself: every
  // participant held its tag then, so no other holder's look is sampled or kept. No flag
  // artwork, since the bars draw none.
  const holders = useMemo(() => shownHour == null ? [] : tags.map(tag => ({ key: tag, tag, hour: shownHour })), [tags, shownHour]);
  const looks = useHolderLooks(runtime.client, track, holders, active, tokens);
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
  const values = width >= VALUES_WIDTH;
  const diagnostics = useMemo(() => [...new Set([...(structural.data?.warnings ?? []), ...(current?.warnings ?? []),
    ...(group?.reason ? [group.reason] : [])])], [structural.data, current, group]);
  useDiagnostics(onDiagnostics, diagnostics);
  const items = useMemo<Item[]>(() => rows.map(row => {
    const lengths = rowLengths(row, show), participation = row.key === 'participation';
    // A negative score, such as lend-lease received, keeps its length and is outlined as negative.
    const parts = (side: SideTotal) => side.members.flatMap(member => [
      { id: member.tag, value: Math.abs(toNumber(member.value)), color: looks.colour(member.tag), negative: member.value < 0n },
      ...(member.unknown != null && member.unknown !== 0n ? [{ id: `${member.tag}:unknown`, value: Math.abs(toNumber(member.unknown)), color: looks.colour(member.tag), hatched: true,
        negative: member.unknown < 0n }] : []),
    ]);
    return { id: row.key, label: row.label, color: tokens.muted, row, segment: 'war',
      left: lengths.left, right: lengths.right, leftParts: parts(row.left), rightParts: parts(row.right),
      leftText: sideText(row.left, participation), rightText: sideText(row.right, participation), ratioText: '' };
  }), [rows, show, looks, tokens.muted]);
  const partAt = useCallback((side: SideTotal, index: number) => {
    let at = 0;
    for (const member of side.members) {
      if (at === index) return { member, unknown: false };
      at++;
      if (member.unknown != null && member.unknown !== 0n) { if (at === index) return { member, unknown: true }; at++; }
    }
    return null;
  }, []);
  const tooltip = useCallback((item: Item, part?: ButterflyPartHit | null): TooltipContent | null => {
    if (shownHour == null) return null;
    const participation = item.row.key === 'participation';
    const format = (raw: bigint) => participation ? `${amount(raw * 100n, 5)}%` : amount(raw, 5);
    const sideRows = (label: string, side: SideTotal): TooltipRow[] => [
      { label, value: side.members.length ? format(side.total) : 'Not recorded', strong: true, divider: true },
      ...(side.unknown != null && side.unknown !== 0n ? [{ label: 'Unknown casualties contribution', value: amount(side.unknown, 5), indent: true, tone: 'muted' as const }] : []),
    ];
    if (part) {
      const side = part.side === 'left' ? item.row.left : item.row.right;
      const found = partAt(side, part.index);
      if (found) {
        const value = found.unknown ? found.member.unknown ?? 0n : found.member.value;
        return { title: looks.name(found.member.tag), subtitle: `${item.label} · ${dayLabel(shownHour)}`, rows: [
            { label: found.unknown ? 'Unknown casualties contribution' : item.label, value: format(value) },
            { label: 'Share of side', value: shareText(value, sideAmount(side)) },
          ] };
      }
    }
    return { title: item.label, subtitle: `${warName(observation)} · ${dayLabel(shownHour)}`,
      rows: [...sideRows('Viewpoint’s side', item.row.left), ...sideRows('Opposing side', item.row.right)] };
  }, [shownHour, observation, looks, partAt]);
  const menu = useCallback((item: Item) => {
    if (!shownWar || !track || shownHour == null) return [];
    // A member's joining is its native country's, continuous through its tag changes.
    const joins = identities ? [...new Set([...item.row.left.members, ...item.row.right.members].map(member => member.tag))].flatMap(tag => {
      const joined = joinedAt(shownWar, tag, shownHour, identities);
      return joined && joined.firstHour !== shownWar.firstHour ? [{ ...jumpItem(runtime, track.id, joined.firstHour),
        label: `Jump to ${looks.name(tag)} ${joined.joinedBy ? 'joined by' : 'joining'} ${dayLabel(joined.firstHour)}` }] : [];
    }) : [];
    return [{ ...jumpItem(runtime, track.id, shownWar.firstHour), label: `Jump to war start ${dayLabel(shownWar.firstHour)}` }, ...joins];
  }, [shownWar, track, shownHour, runtime, looks, identities]);
  const header = headerSlot && shownWar && createPortal(<span className="pill" title={warName(observation)}>{warName(observation ?? (readHour == null ? null : shownWar.observations.at(-1) ?? null))}</span>, headerSlot);
  if (!track) return <WidgetState kind="empty" icon="swords" message="Load a campaign to see war contributions." />;
  if (!window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (structural.error) return <WidgetState kind="error" detail={structural.error} onRetry={structural.retry} />;
  if (exact.error) return <WidgetState kind="error" detail={exact.error} onRetry={exact.retry} />;
  if (!structural.data) return <WidgetState kind="loading" label="Loading war contributions…" />;
  if (!structural.data.available) return <WidgetState kind="unavailable" message="Wars are not in this recording." />;
  if (!war) return <WidgetState kind="empty" icon="swords" message={widget.time.mode === 'follow' ? 'This country has fought no war yet.' : 'This country has fought no war in this campaign.'} />;
  if (!shownWar) return <WidgetState kind="loading" label="Loading war contributions…" />;
  if (!observation) return <>{header}<WidgetState kind="unavailable" message="This war was not recorded at this time." /></>;
  if (!current) return <>{header}<WidgetState kind="loading" label="Loading war contributions…" /></>;
  if (!group || group.reason) return <>{header}<WidgetState kind="unavailable" message="This war’s contributions were not recorded at this time." /></>;
  return <div ref={root} className="war-contribution">
    {header}
    {setOption && <div className="war-contribution-caption"><Segmented value={show} options={SHOW_CHOICES} label="Show" onChange={value => setOption('show', value)} /></div>}
    <ButterflyChart key={JSON.stringify([track.source.id, track.source.revision, track.branch.id, shownWar!.id])} label={`Contributions to ${warName(observation)}`}
      items={items} leftLabel="◀ Viewpoint’s side" rightLabel="Opposing side ▶" ratioLabel="" showRatio={false} showFlags={false}
      showValues={values} axis={show === 'share'} formatTick={value => `${Math.round(value * 100)}%`}
      active={active} hoursPerSecond={hoursPerSecond} tooltip={tooltip} contextItems={menu} />
  </div>;
}
