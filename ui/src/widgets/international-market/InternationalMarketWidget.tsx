import { useCallback, useMemo, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import { MARKET, partyTags, validateMarketReply } from '../../dashboard/market.ts';
import { resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { useGate } from '../../dashboard/regionGate';
import { SPEC, type GatedHour } from '../../dashboard/regionTime';
import { productRequest } from '../../dashboard/request.ts';
import type { InstalledAction } from '../../families/contract.ts';
import { DetailWindowHost } from '../../families/host/DetailWindowHost.tsx';
import { countryTagKey, useIdentity, useIdentityScope } from '../../families/identity/react.ts';
import { FamilyWidget } from '../../families/host/registry';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { MarketLendLeaseReply } from '../../generated/dashboard';
import { DETAILS, OPTIONS } from './definition';
import { MARKET_DECLARATIONS } from './internationalMarketProjection.ts';
import { marketDetail, type MarketDetailLine } from './marketData.ts';
import '../../families/table/gameWindowTable.css';
import './marketDetail.css';

interface Opened {
  readonly title: string;
  readonly tags: readonly string[];
  readonly selected: string;
  readonly hour: GatedHour;
}

interface Naming {
  readonly runtime: WidgetProps['runtime'];
  readonly source: NonNullable<WidgetProps['runtime']['tracks'][number]>['source'];
  readonly scope: ReturnType<typeof useIdentityScope>;
  readonly hour: number;
  readonly active: boolean;
}

function CountryName({ naming, tag }: { readonly naming: Naming; readonly tag: string }) {
  const key = useMemo(() => countryTagKey(tag), [tag]);
  const { result } = useIdentity(naming.runtime.client, naming.source, naming.scope, key, naming.hour, { active: naming.active });
  return <>{result?.kind === 'known' && result.name.kind === 'known' ? result.name.value : tag}</>;
}

function Lines({ lines, naming }: { readonly lines: readonly MarketDetailLine[]; readonly naming: Naming }) {
  return (
    <dl>
      {lines.map((line, index) => (
        <div key={index} className={line.marked ? 'is-marked' : undefined}>
          <dt>{line.label}</dt>
          <dd>{line.country === undefined ? line.value : <CountryName naming={naming} tag={line.country} />}</dd>
        </div>
      ))}
    </dl>
  );
}

function MarketRowDetail({ runtime, trackId, opened, active }: {
  readonly runtime: WidgetProps['runtime'];
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
  const scope = useIdentityScope(runtime.client, track?.source, track?.branch.id ?? null, active);
  if (!track) return null;
  const naming: Naming = { runtime, source: track.source, scope, hour: opened.hour, active };
  if (error) return <WidgetState kind="error" what="the market details" failure={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading the market…" />;
  const detail = marketDetail(data, opened.selected);
  if (detail.missing !== null) return <WidgetState kind="unavailable" message={detail.missing} />;
  return (
    <div className="market-detail">
      {detail.sections.map(section => (
        <section key={section.title} aria-label={section.title}>
          <h4>{section.title}</h4>
          {section.lines.length ? <Lines lines={section.lines} naming={naming} /> : <p>{section.empty}</p>}
        </section>
      ))}
    </div>
  );
}

export default function InternationalMarketWidget(props: WidgetProps) {
  const { widget, runtime, active } = props;
  const view = resolveOptions(OPTIONS, widget.options).view;
  const tab = view === 'requests' || view === 'inventory' ? view : 'contracts';
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
      <FamilyWidget family="table" view={tab} declaration={MARKET_DECLARATIONS[tab]} {...props} onActivate={onActivate} />
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
