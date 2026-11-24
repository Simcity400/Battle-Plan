import { useCallback, useMemo, useRef } from 'react';
import WidgetState from '../../components/WidgetState';
import type { FamilyViewProps } from '../../dashboard/definitions';
import { MARKET, validateMarketReply } from '../../dashboard/market.ts';
import { useProduct } from '../../dashboard/products';
import { useGate } from '../../dashboard/regionGate';
import { SPEC } from '../../dashboard/regionTime';
import type { InstalledAction } from '../../families/contract.ts';
import { DetailWindowHost } from '../../families/host/DetailWindowHost.tsx';
import { countryTagKey, useIdentity, useIdentityScope } from '../../families/identity/react.ts';
import { FamilyWidget } from '../../families/host/registry';
import { rowRequest, rowScope, rowSource, selectionTrack, useOpenedRow, type RowSource } from '../../families/table/rowSelection.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { MarketLendLeaseReply } from '../../generated/dashboard';
import { DETAILS } from './definition';
import { marketDetail, type MarketDetailLine, type MarketRowMeta } from './marketData.ts';
import '../../families/table/gameWindowTable.css';
import './marketDetail.css';

type MarketSelection = RowSource & MarketRowMeta;

interface Naming {
  readonly runtime: FamilyViewProps['runtime'];
  readonly source: NonNullable<FamilyViewProps['runtime']['tracks'][number]>['source'];
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

function MarketRowDetail({ runtime, selection, active }: {
  readonly runtime: FamilyViewProps['runtime'];
  readonly selection: MarketSelection;
  readonly active: boolean;
}) {
  const track = selectionTrack(runtime.tracks, selection);
  const request = track ? rowRequest(selection, MARKET, { countries: [], only: selection.section, row: selection.row }) : null;
  const { data, error, retry } = useProduct<MarketLendLeaseReply>(runtime.client, request, active, validateMarketReply, track?.source);
  const scope = useIdentityScope(runtime.client, track?.source, track ? selection.branchId : null, active);
  if (!track) return null;
  const naming: Naming = { runtime, source: track.source, scope, hour: selection.hour, active };
  if (error) return <WidgetState kind="error" what="the market details" failure={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading the market…" />;
  const detail = marketDetail(data, selection);
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

export default function InternationalMarketWidget(props: FamilyViewProps) {
  const { widget, runtime, active, manifestDeclaration } = props;
  const anchor = useRef<HTMLDivElement>(null);
  const gate = useGate(runtime, widget, SPEC, active);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const [opened, open, close] = useOpenedRow<MarketSelection>(rowScope(track, widget));

  const onActivate = useCallback(({ subject, meta, hour }: InstalledAction) => {
    const row = meta as MarketRowMeta | undefined;
    const at = gate.recorded(Math.floor(hour));
    if (!track || !row || at === null) return;
    open(subject, { ...rowSource(track, at), title: row.title, section: row.section, row: row.row, variant: row.variant });
  }, [gate, track, open]);

  const selection = opened?.selection ?? null;
  return (
    <div ref={anchor} className="game-window-table">
      {manifestDeclaration ? <FamilyWidget {...props} family="table" declaration={manifestDeclaration} onActivate={onActivate} /> : null}
      <DetailWindowHost
        open={selection !== null}
        subject={opened?.key ?? null}
        hour={selection?.hour ?? null}
        title={selection?.title ?? DETAILS.label}
        day={selection ? formatDateOnly(selection.hour) : ''}
        anchor={anchor}
        onClose={close}
      >
        {selection ? <MarketRowDetail runtime={runtime} selection={selection} active={active} /> : null}
      </DetailWindowHost>
    </div>
  );
}
