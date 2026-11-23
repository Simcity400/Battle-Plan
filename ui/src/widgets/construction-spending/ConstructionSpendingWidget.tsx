import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useEffect, useMemo, useRef } from 'react';
import WidgetState from '../../components/WidgetState';
import Sunburst from '../../charts/sunburst/Sunburst';
import type { SunburstText, SunburstValues } from '../../charts/sunburst/Sunburst';
import { levelsShown, sunburstSettings } from '../../charts/sunburst/sunburstOptions';
import { liveAt } from '../../charts/live';
import { motionOf } from '../../charts/easing.ts';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { icDecimals } from '../../dashboard/icDecimals';
import { useShownHour } from '../../dashboard/widgetPerf';
import { useHeadHour } from '../army-size/headHour';
import { OPTIONS } from './definition';
import { LEVELS, footnote, formatSpent, hasConstruction, presentKey, scaleDigits, spendingItems, validateSpendingReply } from './spendingData';
import type { SpendingWindow } from './spendingData';
import { productRequest } from '../../dashboard/request.ts';

const NO_WARNINGS: string[] = [];
const ROOT = { id: 'all', label: 'All construction' };

export default function ConstructionSpendingWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const settings = sunburstSettings(options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryKey(track) ?? undefined;
  const request = track && tag ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'construction-spending',
    params: { branchId: track.branch.id, country: countryFromKey(tag) } }) : null;
  const { data, error, retry } = useProduct(runtime.client, request, active, validateSpendingReply, track?.source);
  const following = active && widget.time.mode === 'follow' && !!track && !!data;
  const range = widget.time.mode === 'range';
  const windowAt = (position: number): SpendingWindow | null => {
    const window = track ? resolveWindow(widget, track, runtime.tracks, position) : null;
    if (!track || !window) return null;
    return { from: range ? window.firstHour : null, hour: window.lastHour, end: range ? window.lastHour : track.branch.lastHour };
  };
  const windowRef = useRef(windowAt);
  windowRef.current = windowAt;
  const position = useHeadHour(runtime.clock, following, at => {
    const shown = windowRef.current(at);
    return data && shown ? presentKey(spendingItems(data, shown)) : '';
  });
  const shownWindow = windowAt(position);
  const hour = shownWindow?.hour;
  const inBranch = track != null && hour != null && hour >= track.branch.firstHour && hour <= track.branch.lastHour;
  const items = useMemo(() => data && shownWindow ? spendingItems(data, shownWindow) : [],
    [data, shownWindow?.from, shownWindow?.hour, shownWindow?.end]);
  const live = useMemo(() => following ? liveAt(runtime.clock, at => windowRef.current(at)?.hour ?? hour ?? 0, () => {
    const shown = windowRef.current(runtime.clock.getSnapshot().position);
    return { items: data && shown ? spendingItems(data, shown) : [] } satisfies SunburstValues;
  }) : undefined,
  [following, runtime.clock, data]);
  const report = useShownHour();
  useEffect(() => {
    if (!live) { report(data && inBranch ? hour ?? null : null); return; }
    report(live.hour());
    return live.subscribe(() => report(live.hour()));
  }, [live, report, data, inBranch, hour]);
  const levelKey = LEVELS.map(level => options[level] !== false).join();
  const levels = useMemo(() => levelsShown(options, LEVELS), [levelKey]);
  const scale = data?.costScale ?? 1;
  const digits = icDecimals(options, scaleDigits(scale));
  const text = useMemo<SunburstText>(() => ({
    format: (value, exact) => exact == null ? 'Not recorded' : formatSpent(value, exact, scale, digits), unit: 'IC', valueLabel: 'Spent', countLabel: 'Projects',
  }), [scale, digits]);
  useEffect(() => {
    onDiagnostics(data?.warnings.length ? data.warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, onDiagnostics]);

  if (!track || !tag) return <WidgetState kind="empty" icon="sunburst" message="Load a campaign and choose a country to see its construction spending." />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading construction spending…" />;
  if (!data.recorded) return <WidgetState kind="unavailable" message="Construction was not recorded." />;
  if (!shownWindow || !inBranch) return <WidgetState kind="outside" />;
  const note = footnote(data, items, shownWindow);
  if (!hasConstruction(items)) return <WidgetState kind="empty" icon="sunburst" message="No construction recorded" detail={note} />;
  return <div style={{ height: '100%' }}><Sunburst label="Construction spending" items={items} root={ROOT} levels={levels} text={text} settings={settings}
    live={live} hoursPerSecond={motionOf(runtime.clock.getSnapshot())} active={active} footnote={note} /></div>;
}
