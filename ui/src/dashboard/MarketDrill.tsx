import { countryFromKey } from './countryKey.ts';
import { useEffect } from 'react';
import WidgetState from '../components/WidgetState';
import type { WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import { useHeadProduct } from './headProduct';
import { useGate } from './regionGate';
import type { RegionAction } from './regionTime.ts';
import { MARKET, validateMarketReply } from './market.ts';
import MarketOccurrences from './MarketOccurrences';
import { DELIVERY_KINDS } from './marketLedger.ts';
import TradeMarketView from '../widgets/trade-flows/TradeMarketView';
import '../widgets/trade-flows/tradeFlows.css';

export default function MarketDrill({ runtime, widget, time, tags, selected, active, onAnswered }: {
  runtime: WidgetRuntime; widget: WidgetSpec; time: RegionAction; tags: readonly string[]; selected: string | null; active: boolean;
  onAnswered?: (hour: number | undefined) => void;
}) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const gate = useGate(runtime, widget, time);
  const head = useHeadProduct(runtime, gate, MARKET, { countries: [...tags].sort().map(countryFromKey) },
    { active, validate: validateMarketReply });
  useEffect(() => {
    onAnswered?.(head.at ?? undefined);
    return () => onAnswered?.(undefined);
  }, [onAnswered, head.at]);
  if (head.outside) return <WidgetState kind="outside" read={head} />;
  if (head.error && !head.data) return <WidgetState kind="error" failure={head.error} onRetry={head.retry} />;
  if (!head.data || !track || head.at === null) return <WidgetState kind="loading" label="Loading the market…" read={head} />;
  const data = { ...head.data, hour: head.at };
  const following = time.kind === 'follow' || time.kind === 'spec' && widget.time.mode === 'follow';
  return <div ref={head.root} style={{ display: 'contents' }}><TradeMarketView data={data} selected={selected} values={head.live.read()}
    deliveries={<MarketOccurrences runtime={runtime} track={track} gate={gate} kinds={DELIVERY_KINDS} tags={tags} span={gate.span(track.branch.firstHour, track.branch.lastHour)}
      cut={following ? { follow: true, offset: track.axisOffset } : { follow: false, hour: data.hour }} active={active} label="Deliveries"
      density="compact" months={false} />} /></div>;
}
