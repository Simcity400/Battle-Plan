import { useCallback, useEffect, useMemo, useRef } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import Sunburst from '../../charts/sunburst/Sunburst';
import type { SunburstItem, SunburstText, SunburstValues } from '../../charts/sunburst/Sunburst';
import { levelsShown, sunburstSettings } from '../../charts/sunburst/sunburstOptions';
import type { Live, LiveValues } from '../../charts/live';
import { jumpItem } from '../../charts/jump';
import { usNumber } from '../../charts/numberFormat';
import { motionOf } from '../../charts/easing.ts';
import { useCountryLooks } from '../../dashboard/countryLooks';
import { usePushedLooksHour } from '../../dashboard/pushedLooks.ts';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import { countryKey, isSlotKey } from '../../dashboard/countryKey.ts';
import { useSlotTags } from '../../dashboard/countrySlots.ts';
import { useFactionColours } from '../../dashboard/FactionColours';
import { factionNameLabel } from '../../dashboard/factionNames';
import { resolveCountries } from '../../dashboard/groups';
import type { WidgetProps } from '../../dashboard/definitions';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useCampaignGate, useGate } from '../../dashboard/regionGate';
import { repliedCells } from '../../dashboard/follow/liveCells.ts';
import { resolveOptions } from '../../dashboard/options';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { AREA_UNIT, measureOf, territoryNote } from '../../dashboard/territoryMeasure.ts';
import { affiliation, affiliationEpochs, breakdownItems, epochOf, validateBreakdownReply, withFinals } from './breakdownData.ts';
import { useProduct } from '../../dashboard/products';
import { resolveWindow } from '../../dashboard/model';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import type { RegionalRule } from './breakdownData';
import { LEVELS, OPTIONS } from './definition';
import type { TerritoryBreakdownReply } from './types';
import { productRequest } from '../../dashboard/request.ts';

const NO_WARNINGS: string[] = [];

export default function TerritoryBreakdownWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const measure = measureOf(options.measure);
  const rule: RegionalRule = options.rule === 'owned' ? 'owned' : 'controlled';
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const params = useMemo(() => ({ measure, rule }), [measure, rule]);
  const gate = useGate(runtime, widget);
  const campaign = useCampaignGate(runtime, widget, active);
  const breakdown = useHeadProduct(runtime, gate, 'territory-breakdown', track ? params : null,
    { active, validate: validateBreakdownReply });
  const hour = breakdown.at ?? breakdown.hour ?? (track ? Math.floor(runtime.clock.getHour()) + track.axisOffset : 0);
  const shownAt = useRef(breakdown.shownAt);
  shownAt.current = breakdown.shownAt;
  useShownEachFrame(runtime.clock, () => breakdown.data ? breakdown.shownAt() : null, breakdown.live);
  const fill = options.growth === 'fill';
  const end = track ? (widget.time.mode === 'range' ? resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour())?.lastHour ?? track.branch.lastHour : track.branch.lastHour) : 0;
  const endAt = widget.time.mode === 'range' ? gate.window?.lastHour ?? null : campaign.hour;
  const ending = useProduct<TerritoryBreakdownReply>(runtime.client, track && fill && endAt !== null ? productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'territory-breakdown', params: { branchId: track.branch.id, hour: endAt, measure, rule } }) : null, active, validateBreakdownReply, track?.source);
  const groups = useCountryGroups(runtime, track, active);
  const colours = useFactionColours(runtime.client, track?.source);
  const fallback = defaultCountryKey(track);
  const lastHour = track?.branch.lastHour ?? 0;
  const resolved = useMemo(() => widget.countries.kind === 'all' ? null
    : resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data, lastHour, Number.MAX_SAFE_INTEGER), [widget.countries, fallback, groups.data, lastHour]);
  const slots = useSlotTags(runtime.client, track, active && !!resolved?.tags.some(isSlotKey));
  const tags = useMemo(() => [...new Set([...(breakdown.data?.rows ?? []), ...(fill ? ending.data?.rows ?? [] : [])].flatMap(row => row.tag === null ? [] : [row.tag]))], [breakdown.data, fill, ending.data]);
  const named = usePushedLooksHour(runtime, widget, track, tags, active && breakdown.read === 'followed', hour);
  const looks = useCountryLooks(runtime.client, track, tags, named.hour, active, undefined, undefined, named.exact);
  const selectedSlot = useCallback((tag: string, at: number) => {
    const slot = slots?.slotOf(tag, at);
    return slot != null && !!resolved?.tags.includes(countryKey(slot));
  }, [slots, resolved]);
  const itemsAt = useCallback((reply: TerritoryBreakdownReply, values: LiveValues<number>, at: number, nameAt = at) => groups.data
    ? breakdownItems(reply, values, measure, affiliation(groups.data, at), {
      country: tag => ({ name: looks.name(tag, nameAt), flag: looks.flag(tag, nameAt), flagPending: looks.flagPending(tag, nameAt), flagReason: looks.flagReason(tag, nameAt) }),
      faction: faction => factionNameLabel(faction, at, colours.displayNames.get(faction.id)),
      colour: id => colours.colours.get(id),
      selected: tag => resolved === null || (tag !== null && (resolved.tags.includes(tag) || selectedSlot(tag, at))),
    }) : [], [groups.data, measure, looks, colours.colours, colours.displayNames, resolved, selectedSlot]);
  const endingItems = useMemo(() => fill && ending.data ? itemsAt(ending.data, repliedCells(ending.data).read(), end) : null, [fill, ending.data, itemsAt, end]);
  const epochs = useMemo(() => groups.data ? affiliationEpochs(groups.data) : [], [groups.data]);
  const live = useMemo<Live<SunburstValues> | undefined>(() => {
    const reply = breakdown.data;
    if (!reply) return undefined;
    const cells = breakdown.live;
    let made: { values: LiveValues<number>; epoch: number; out: SunburstValues } | null = null;
    return {
      read: () => {
        const values = cells.read();
        const at = Math.floor(shownAt.current() ?? hour);
        const epoch = epochOf(epochs, at);
        if (made?.values !== values || made.epoch !== epoch) {
          const now = itemsAt(reply, values, at, named.hour);
          made = { values, epoch, out: { items: endingItems ? withFinals(now, endingItems) : now } };
        }
        return made.out;
      },
      subscribe: changed => cells.subscribe(changed),
    };
  }, [breakdown.data, breakdown.live, hour, epochs, itemsAt, named.hour, endingItems]);
  const items: readonly SunburstItem[] = useMemo(() => live?.read().items ?? [], [live]);
  const text = useMemo<SunburstText>(() => ({ format: (value, exact, denominator = 1n) => {
    if (exact == null) return usNumber.format(value);
    const whole = exact / denominator, remainder = exact % denominator;
    return remainder === 0n ? usNumber.format(whole) : `${whole === 0n ? '' : `${usNumber.format(whole)} `}${remainder}/${denominator}`;
  }, unit: measure === 'area' ? AREA_UNIT : measure === 'victoryPoints' ? 'victory points' : measure,
  valueLabel: rule === 'owned' ? 'Owned' : 'Controlled', missingLabel: 'Not recorded', emptyLabel: 'No territory at this hour.' }), [measure, rule]);
  const menu = useContextMenu('Territory breakdown actions');
  const diagnosticKey = JSON.stringify([...(breakdown.data?.warnings ?? []), ...(groups.data?.warnings ?? []), ...colours.warnings,
    ...(breakdown.reason ? [`Territory breakdown is asked by the fallback clock: ${breakdown.reason}`] : [])]);
  useEffect(() => {
    const warnings = JSON.parse(diagnosticKey) as string[];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [diagnosticKey, onDiagnostics]);
  const state = !track ? <WidgetState kind="empty" icon="sunburst" message="Load a campaign to see its territory breakdown." />
    : breakdown.error ? <WidgetState kind="error" failure={breakdown.error} onRetry={breakdown.retry} />
      : fill && ending.error ? <WidgetState kind="error" what="the territory at the view's end" failure={ending.error} onRetry={ending.retry} />
      : groups.error && !groups.data ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />
        : colours.error ? <WidgetState kind="error" what="faction colours" failure={colours.error} />
          : !breakdown.data || !groups.data || colours.loading || (fill && !ending.data) ? <WidgetState kind="loading" label="Loading territory breakdown..." />
            : breakdown.data.missing ? <WidgetState kind="unavailable" message={breakdown.data.missing} />
              : measure === 'area' && breakdown.data.areaReason ? <WidgetState kind="unavailable" message={breakdown.data.areaReason} />
              : resolved?.reason ? <WidgetState kind="unavailable" message={resolved.reason} /> : null;
  return <div ref={breakdown.root}>{state ?? <Sunburst key={`${track!.source.id}:${track!.source.revision}:${track!.branch.id}`} label="Territory breakdown"
    items={items} live={live} root={{ id: 'territory', label: widget.countries.kind === 'all' ? 'World territory' : 'Selected territory' }}
    levels={levelsShown(options, LEVELS)} text={text} settings={sunburstSettings(options)} active={active}
    hoursPerSecond={motionOf(runtime.clock.getSnapshot())} footnote={territoryNote(measure)}
    onContextMenu={(_node, x, y) => menu.open(x, y, [jumpItem(runtime, track!.id, Math.floor(shownAt.current() ?? hour))])} />}{menu.menu}</div>;
}
