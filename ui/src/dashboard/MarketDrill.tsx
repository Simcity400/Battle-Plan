// Trade flows' market drill: one offer, request, contract or agreement's recorded
// terms and the hour's deliveries, read at the drill's hour.
import WidgetState from '../components/WidgetState';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack } from './model';
import { useProduct } from './products';
import type { MarketLendLeaseReply } from '../generated/dashboard';
import TradeMarketView from '../widgets/trade-flows/TradeMarketView';
import { marketRequest, validateMarketReply } from '../widgets/trade-flows/marketData';
import '../widgets/trade-flows/tradeFlows.css';

/** `selected` is Trade flows' key: `offer:<country>`, `contract:<request>:<id>` or `agreement:<id>`. */
export default function MarketDrill({ runtime, track, hour, tags, selected, active }: {
  runtime: WidgetRuntime; track: BoundTrack; hour: number; tags: readonly string[]; selected: string | null; active: boolean;
}) {
  const { data, error, retry } = useProduct<MarketLendLeaseReply>(runtime.client, marketRequest(track, hour, tags), active, validateMarketReply);
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading the market…" />;
  return <TradeMarketView data={data} selected={selected} />;
}
