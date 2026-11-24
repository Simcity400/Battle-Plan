import { useCallback, useMemo, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import { contentSet } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { useGate } from '../../dashboard/regionGate';
import { SPEC, type GatedHour } from '../../dashboard/regionTime';
import { productRequest } from '../../dashboard/request.ts';
import { DetailWindowHost } from '../../families/host/DetailWindowHost.tsx';
import { FamilyWidget } from '../../families/host/registry.tsx';
import { parseChartInspect } from '../../families/time-chart/hitTest.ts';
import type { InstalledAction } from '../../families/plan.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { FactoryPartsReply } from '../../generated/dashboard';
import { countMode, partRows, validateFactoryParts } from './factoriesData.ts';
import type { FactoriesRowMeta } from './factoriesProjection.ts';

function FactoryDetails({ widget, runtime, active, tag, hour, types, mode }: Pick<WidgetProps, 'widget' | 'runtime' | 'active'> & {
  tag: string; hour: GatedHour; types: readonly string[]; mode: ReturnType<typeof countMode>;
}) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track ? productRequest({
    sourceId: track.source.id,
    revision: track.source.revision,
    product: 'factory-parts',
    params: { branchId: track.branch.id, country: countryFromKey(tag), hour },
  }) : null;
  const parts = useProduct<FactoryPartsReply>(runtime.client, request, active, validateFactoryParts, track?.source);
  if (parts.error) return <WidgetState kind="error" failure={parts.error} onRetry={parts.retry} />;
  if (!parts.data) return <WidgetState kind="loading" label="Loading factory breakdown…" />;
  const rows = partRows(parts.data, types, mode);
  return (
    <div className="detail-list">
      <p>{tag}, {formatDateOnly(hour)}</p>
      {rows.map((row, index) => (
        <div key={`${row.label}:${index}`} className="detail-row">
          <span>{row.strong ? <strong>{row.label}</strong> : row.label}</span>
          <span>{row.strong ? <strong>{row.value ?? ''}</strong> : (row.value ?? '')}</span>
        </div>
      ))}
    </div>
  );
}

export default function FactoriesWidget(props: WidgetProps) {
  const { widget, runtime, active, manifestDeclaration } = props;
  const rawView = String(props.view ?? widget.options?.view ?? 'race');
  const view = rawView === 'lines' ? 'lines' : rawView === 'mix' ? 'mix' : 'race';
  const mode = countMode(widget.options?.count);
  const types = useMemo(() => contentSet(widget.options?.types ?? ''), [widget.options?.types]);
  const isLines = manifestDeclaration?.family === 'time-chart' || view === 'lines';

  const anchorRef = useRef<HTMLDivElement>(null);
  const [detail, setDetail] = useState<{ readonly tag: string; readonly key: string; readonly hour: GatedHour } | null>(null);
  const gate = useGate(runtime, widget, SPEC, active);

  const onActivate = useCallback((action: InstalledAction) => {
    if (isLines && parseChartInspect(action.subject) !== null) return;
    const gated = gate.recorded(Math.floor(action.hour));
    if (gated === null) return;
    const rowMeta = action.meta as FactoriesRowMeta | null | undefined;
    setDetail({ tag: rowMeta?.tag ?? action.subject, key: rowMeta?.key ?? action.subject, hour: gated });
  }, [gate, isLines]);

  if (!manifestDeclaration) return null;

  return (
    <div ref={anchorRef} className="fh-frame">
      <FamilyWidget
        {...props}
        family={manifestDeclaration.family}
        view={view}
        declaration={manifestDeclaration}
        {...(isLines ? {} : { selectedKey: detail?.key ?? null })}
        onActivate={onActivate}
      />
      <DetailWindowHost
        open={detail !== null}
        subject={detail?.tag ?? null}
        hour={detail?.hour ?? null}
        title={detail ? `${detail.tag} factories` : 'Factories'}
        anchor={anchorRef}
        returnFocus=".fh-body"
        onClose={() => setDetail(null)}
      >
        {detail ? (
          <FactoryDetails
            widget={widget}
            runtime={runtime}
            active={active}
            tag={detail.tag}
            hour={detail.hour}
            types={types}
            mode={mode}
          />
        ) : null}
      </DetailWindowHost>
    </div>
  );
}
