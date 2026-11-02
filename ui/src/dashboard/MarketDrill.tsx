// Trade flows' market drill: one offer, request, contract or agreement's recorded
// terms and the hour's deliveries, read at the drill's hour.
import { useEffect } from 'react';
import WidgetState from '../components/WidgetState';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack } from './model';
import { useFollowedProduct } from './products';
import type { MarketLendLeaseReply } from '../generated/dashboard';
import TradeMarketView from '../widgets/trade-flows/TradeMarketView';
import { marketRequest, validateMarketReply } from '../widgets/trade-flows/marketData';
import '../widgets/trade-flows/tradeFlows.css';

/** `selected` is Trade flows' key: `offer:<country>`, `contract:<request>:<id>` or `agreement:<id>`. */
export default function MarketDrill({ runtime, track, hour, tags, selected, active, onAnswered }: {
  runtime: WidgetRuntime; track: BoundTrack; hour: number; tags: readonly string[]; selected: string | null; active: boolean;
  /** The hour the shown reply answers, which a reply kept while the next hour loads holds. */
  onAnswered?: (hour: number | undefined) => void;
}) {
  // A drill following playback keeps its last hour shown while the newest is read.
  const { data, error, retry } = useFollowedProduct<MarketLendLeaseReply>(runtime.client, marketRequest(track, hour, tags), ['hour'], active, validateMarketReply);
  useEffect(() => {
    onAnswered?.(data?.hour);
    return () => onAnswered?.(undefined);
  }, [onAnswered, data?.hour]);
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading the market…" />;
  return <TradeMarketView data={data} selected={selected} />;
}
