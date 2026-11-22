import { useMemo, useReducer, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { ChipSet } from '../../components/ChipSet';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, reduceDrill } from '../../charts/drill/drillState';
import { dayLabel } from '../../charts/draw';
import { logSettings } from '../../charts/log/logOptions';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatchFilter } from '../../dashboard/groups';
import { partyTags } from '../../dashboard/market.ts';
import MarketDrill from '../../dashboard/MarketDrill';
import { KINDS, ledgerSelection, ledgerTitle, shownKinds } from '../../dashboard/marketLedger.ts';
import type { KindKey, LedgerItem } from '../../dashboard/marketLedger';
import MarketOccurrences from '../../dashboard/MarketOccurrences';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { LogCut } from '../../dashboard/useLogPages';
import { trackHourOf } from '../../dashboard/settledHour';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { DETAILS, OPTIONS } from './definition';
import './marketLedger.css';
import { drillAction, recordedHour } from '../../dashboard/regionTime.ts';
import { useRegion, useRegionHour } from '../../dashboard/widgetTime.ts';

export default function MarketLedgerWidget({ widget, runtime, active, setOption }: WidgetProps) {
  const [drillShown, setDrillShown] = useState<number | undefined>();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  useShownEachFrame(runtime.clock, () => track ? trackHourOf(widget, track, runtime.tracks, runtime.clock.getHour()) : null);
  const options = resolveOptions(OPTIONS, widget.options);
  const log = logSettings(options);
  const follow = widget.time.mode === 'follow';
  const window = track && !follow ? resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour()) : null;
  const firstHour = widget.time.mode === 'range' && window && track ? Math.max(Math.floor(window.firstHour), track.branch.firstHour) : track?.branch.firstHour ?? 0;
  const cut: LogCut = follow ? { follow: true, offset: track?.axisOffset ?? 0 }
    : { follow: false, hour: widget.time.mode === 'campaign' || !window ? null : Math.floor(window.lastHour) };
  const needsGroups = widget.countries.kind === 'group' || widget.countries.kind === 'factions';
  const groups = useCountryGroups(runtime, track, active && needsGroups);
  const tags = useMemo<string[] | null | undefined>(() => {
    if (widget.countries.kind === 'all') return null;
    if (!needsGroups) return [...selectedCountryKeys(widget.countries, track)].sort();
    if (!groups.data) return undefined;
    const filter = countryMatchFilter(widget.countries, [], groups.data, track?.branch.lastHour ?? 0, undefined, track?.branch.firstHour);
    return [...new Set(filter.membership?.map(run => run.tag) ?? [])].sort();
  }, [widget.countries, needsGroups, track, groups.data]);
  const enabled = (key: KindKey) => options[key] !== false;
  const kinds = shownKinds(enabled);
  const kindsKey = kinds.join(',');
  const stableKinds = useMemo(() => kinds, [kindsKey]);
  const anchor = useRef<HTMLDivElement>(null);
  const [drill, dispatch] = useReducer(reduceDrill, CLOSED);
  const pickOwner = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id]) : null;
  const [pick, setPick] = useState<{ owner: string | null; item: LedgerItem } | null>(null);
  const picked = pick && pick.owner === pickOwner ? pick.item : null;
  const open = (item: LedgerItem, at: number) => {
    setPick({ owner: pickOwner, item });
    dispatch({ type: 'open', breakdown: DETAILS.id, hour: at, playing: runtime.clock.getSnapshot().playing,
      playhead: follow && track ? recordedHour(track, Math.floor(runtime.clock.getHour()) + track.axisOffset) : null });
  };
  const drillTime = drillAction(drill, track);
  const drillRegion = useRegion(widget, drillTime);
  const drillOpen = drill.breakdown !== null && !!picked;
  const drillAt = useRegionHour(runtime, drillRegion, active && drillOpen);
  const drillDay = drillShown ?? drillAt;
  const pickedTags = picked ? partyTags(picked.seller, picked.buyer) : null;
  const chips = KINDS.map(kind => ({ value: kind.key, label: kind.label }));

  if (!track) return <WidgetState kind="empty" icon="history" message="Load a campaign to see market activity." />;
  const parent = !follow && widget.time.mode !== 'campaign' && !window ? <WidgetState kind="unavailable" message="This time is outside the recording." />
    : needsGroups && groups.error && !groups.data ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />
    : tags !== null && tags !== undefined && !tags.length ? <WidgetState kind="unavailable" message={needsGroups ? 'This group has no members in this campaign.' : 'Choose a country in the widget settings.'} /> : null;
  if (parent && !drillOpen) return parent;
  const main = parent ?? <div className="log-widget market-ledger">
    {log.chips && <div className="log-chips"><ChipSet label="Market occurrences" chips={chips}
      selected={KINDS.filter(kind => enabled(kind.key)).map(kind => kind.key)} max={KINDS.length} onToggle={key => setOption?.(key, !enabled(key as KindKey))} /></div>}
    {tags === undefined ? <WidgetState kind="loading" label="Loading market activity…" />
      : !stableKinds.length ? <WidgetState kind="empty" icon="history" message="Choose the occurrences to show in the widget settings." />
        : <MarketOccurrences runtime={runtime} track={track} kinds={stableKinds} tags={tags} firstHour={firstHour} cut={cut} active={active} label="Market ledger"
          density={log.density} months={log.months} animate={log.animate} onOpen={open} />}
  </div>;
  return <div ref={anchor} className="market-ledger-host">
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drillOpen} anchor={anchor}
      title={picked ? ledgerTitle(picked, tag => tag ?? 'Country not recorded') : DETAILS.label} day={drillDay == null ? '' : dayLabel(drillDay)}
      following={drill.follow} canFollow={follow} onClose={() => dispatch({ type: 'close' })} onFollow={() => dispatch({ type: 'follow' })}
      main={main} time={{ runtime, region: drillRegion }}>
      {picked && (pickedTags ? <MarketDrill runtime={runtime} widget={widget} time={drillTime} active={active} onAnswered={setDrillShown}
        tags={pickedTags} selected={ledgerSelection(picked)} />
        : <WidgetState kind="unavailable" message="Neither party was recorded at this hour." />)}
    </DrillHost>
  </div>;
}
