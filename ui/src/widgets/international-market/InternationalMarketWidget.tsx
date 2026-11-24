import { useCallback, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { FamilyViewProps } from '../../dashboard/definitions';
import { MARKET, partyTags, validateMarketReply } from '../../dashboard/market.ts';
import { useProduct } from '../../dashboard/products';
import { useGate } from '../../dashboard/regionGate';
import { SPEC, type GatedHour } from '../../dashboard/regionTime';
import { productRequest } from '../../dashboard/request.ts';
import type { InstalledAction } from '../../families/contract.ts';
import { DetailWindowHost } from '../../families/host/DetailWindowHost.tsx';
import { FamilyWidget } from '../../families/host/registry';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { MarketLendLeaseReply } from '../../generated/dashboard';
import { DETAILS } from './definition';
import { marketDetail, type MarketDetailLine } from './marketData.ts';
import '../../families/table/gameWindowTable.css';
import './marketDetail.css';

interface Opened {
  readonly title: string;
  readonly tags: readonly string[];
  readonly selected: string;
  readonly hour: GatedHour;
}

function Lines({ lines }: { readonly lines: readonly MarketDetailLine[] }) {
  return (
    <dl>
      {lines.map((line, index) => (
        <div key={index} className={line.marked ? 'is-marked' : undefined}>
          <dt>{line.label}</dt>
          <dd>{line.value}</dd>
        </div>
      ))}
    </dl>
  );
}

function MarketRowDetail({ runtime, trackId, opened, active }: {
  readonly runtime: FamilyViewProps['runtime'];
  readonly trackId: string | undefined;
  readonly opened: Opened;
  readonly active: boolean;
}) {
  const track = runtime.tracks.find(item => item.id === trackId);
  const only = opened.selected.startsWith('offer:') ? 'offers' : 'contracts';
  const request = track ? productRequest({
    sourceId: track.source.id,
    revision: track.source.revision,
    product: MARKET,
    params: { branchId: track.branch.id, countries: opened.tags.map(countryFromKey), only, hour: opened.hour },
  }) : null;
  const { data, error, retry } = useProduct<MarketLendLeaseReply>(runtime.client, request, active, validateMarketReply, track?.source);
  if (!track) return null;
  if (error) return <WidgetState kind="error" what="the market details" failure={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading the market…" />;
  const detail = marketDetail(data, opened.selected);
  if (detail.missing !== null) return <WidgetState kind="unavailable" message={detail.missing} />;
  return (
    <div className="market-detail">
      {detail.sections.map(section => (
        <section key={section.title} aria-label={section.title}>
          <h4>{section.title}</h4>
          {section.lines.length ? <Lines lines={section.lines} /> : <p>{section.empty}</p>}
        </section>
      ))}
    </div>
  );
}

export default function InternationalMarketWidget(props: FamilyViewProps) {
  const { widget, runtime, active, manifestDeclaration } = props;
  const anchor = useRef<HTMLDivElement>(null);
  const gate = useGate(runtime, widget, SPEC, active);
  const [opened, setOpened] = useState<Opened | null>(null);

  const onActivate = useCallback(({ meta, hour }: InstalledAction) => {
    const info = meta as { title?: string; tags?: (string | null)[]; selected?: string } | undefined;
    const tags = info?.tags ? partyTags(...info.tags) : null;
    const at = gate.recorded(Math.floor(hour));
    if (!tags || !info?.selected || at === null) return;
    setOpened({ title: info.title ?? DETAILS.label, tags, selected: info.selected, hour: at });
  }, [gate]);

  return (
    <div ref={anchor} className="game-window-table">
      {manifestDeclaration ? <FamilyWidget {...props} family="table" declaration={manifestDeclaration} onActivate={onActivate} /> : null}
      <DetailWindowHost
        open={opened !== null}
        subject={opened?.selected ?? null}
        hour={opened?.hour ?? null}
        title={opened?.title ?? DETAILS.label}
        day={opened ? formatDateOnly(opened.hour) : ''}
        anchor={anchor}
        onClose={() => setOpened(null)}
      >
        {opened ? <MarketRowDetail runtime={runtime} trackId={widget.trackIds[0]} opened={opened} active={active} /> : null}
      </DetailWindowHost>
    </div>
  );
}
