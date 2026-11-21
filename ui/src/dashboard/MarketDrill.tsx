import { countryFromKey } from './countryKey.ts';
import { useEffect } from 'react';
import WidgetState from '../components/WidgetState';
import type { WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import { useHeadProduct } from './headProduct';
import { MARKET, validateMarketReply } from './market.ts';
import MarketOccurrences from './MarketOccurrences';
import { DELIVERY_KINDS } from './marketLedger.ts';
import TradeMarketView from '../widgets/trade-flows/TradeMarketView';
import '../widgets/trade-flows/tradeFlows.css';
import type { MarketLendLeaseReply } from '../generated/dashboard';

export default function MarketDrill({ runtime, widget, hour, following, tags, selected, active, onAnswered }: {
  runtime: WidgetRuntime; widget: WidgetSpec; hour: number; following: boolean; tags: readonly string[]; selected: string | null; active: boolean;
  onAnswered?: (hour: number | undefined) => void;
}) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const head = useHeadProduct<MarketLendLeaseReply>(runtime, widget, MARKET, { countries: [...tags].sort().map(countryFromKey) },
    { active, validate: validateMarketReply, hoverHour: following ? null : hour });
  useEffect(() => {
    onAnswered?.(head.at ?? undefined);
    return () => onAnswered?.(undefined);
  }, [onAnswered, head.at]);
  if (head.error && !head.data) return <WidgetState kind="error" failure={head.error} onRetry={head.retry} />;
  if (!head.data || !track) return <WidgetState kind="loading" label="Loading the market…" />;
  const data = { ...head.data, hour: head.at ?? hour };
  const follows = following && widget.time.mode === 'follow';
  return <div ref={head.root} style={{ display: 'contents' }}><TradeMarketView data={data} selected={selected} values={head.live.read()}
    deliveries={<MarketOccurrences runtime={runtime} track={track} kinds={DELIVERY_KINDS} tags={tags} firstHour={track.branch.firstHour}
      cut={follows ? { follow: true, offset: track.axisOffset } : { follow: false, hour: data.hour }} active={active} label="Deliveries"
      density="compact" months={false} animate={false} />} /></div>;
}
