import { useCallback, useRef, useState } from 'react';
import { contentTitle } from '../../dashboard/contentTitles.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { resolveWindow } from '../../dashboard/model';
import type { FamilyViewProps, FamilyViewRenderer } from '../../dashboard/definitions';
import { useProduct } from '../../dashboard/products';
import { useGate } from '../../dashboard/regionGate';
import { SPEC, type BoundHour, type BoundWindow, type GatedHour } from '../../dashboard/regionTime';
import { productRequest } from '../../dashboard/request.ts';
import { DetailWindowHost } from '../../families/host/DetailWindowHost.tsx';
import { FamilyWidget } from '../../families/host/registry.tsx';
import type { InstalledAction } from '../../families/plan.ts';
import { formatDateOnly, formatScalar } from '../../families/valueFormat.ts';
import type { ProducedAtReply, ProducedCount, ProducedMeasure } from '../../generated/dashboard';
import { UNKNOWN_WORDS, type EquipmentProducedRowMeta } from './equipmentProducedRaceProjection.ts';

function validateProducedAt(data: unknown): ProducedAtReply {
  const reply = data as Partial<ProducedAtReply> | null;
  if (!reply || typeof reply !== 'object' || typeof reply.tag !== 'string' || !Number.isInteger(reply.hour) || !Array.isArray(reply.models)) {
    throw new Error('The equipment produced reply at an hour is malformed.');
  }
  return reply as ProducedAtReply;
}

function ProducedDetails({ widget, runtime, active, country, hour, count, measure, refits, fromHour }: Pick<FamilyViewProps, 'widget' | 'runtime' | 'active'> & {
  readonly country: string;
  readonly hour: GatedHour;
  readonly count: ProducedCount;
  readonly measure: ProducedMeasure;
  readonly refits: boolean;
  readonly fromHour: BoundHour | null;
}) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track ? productRequest({
    sourceId: track.source.id,
    revision: track.source.revision,
    product: 'equipment-produced-at',
    params: { branchId: track.branch.id, country: countryFromKey(country), hour, measure, refits, ...(fromHour !== null ? { fromHour } : {}) },
  }) : null;
  const reply = useProduct<ProducedAtReply>(runtime.client, request, active, validateProducedAt, track?.source);
  if (reply.error) {
    return <div className="detail-empty">Could not read production detail: {reply.error.message}</div>;
  }
  if (!reply.data) {
    return <div className="detail-empty">Loading production detail…</div>;
  }
  const window = reply.data.icTotal.reason === 'windowIncomplete' || reply.data.icTotal.reason === 'observationRestarted' ? reply.data.icTotal.reason : null;
  if (!reply.data.models.length) {
    return <div className="detail-empty">{window ? UNKNOWN_WORDS[window] : `Nothing delivered at ${formatDateOnly(hour)}.`}</div>;
  }
  const decimals = count === 'ic' ? 5 : 0;
  const fmt = count === 'ic' ? { decimals: 0, maxDecimals: 2 } : { decimals: 0 };
  return (
    <div className="detail-list">
      {reply.data.models.map(model => {
        const amount = count === 'ic' ? model.ic : model.units;
        const typeText = model.typeRecorded ? contentTitle(model.typeName, model.typeKey, 'Equipment type') : 'Type not recorded';
        const text = amount.value === null
          ? 'Not known'
          : formatScalar(decimals === 0 ? amount.value : amount.value / 10 ** decimals, fmt);
        return (
          <div key={`${model.rocket ? 'r' : 'd'}:${model.variant}`} className="detail-row">
            <span>{model.name ?? `Design ${model.variant}`} · {typeText}</span>
            <span>{text}</span>
          </div>
        );
      })}
    </div>
  );
}

function rangeStart(widget: FamilyViewProps['widget'], runtime: FamilyViewProps['runtime'], span: (first: number, last: number) => BoundWindow | null, hour: GatedHour): BoundHour | null {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const first = track ? resolveWindow(widget, track, runtime.tracks, hour)?.firstHour : undefined;
  return first === undefined ? null : span(first, hour)?.firstHour ?? null;
}

function EquipmentProducedFamilyWidget(props: FamilyViewProps) {
  const { widget, runtime, active, manifestDeclaration, options } = props;
  const count: ProducedCount = options.count === 'ic' ? 'ic' : 'units';
  const measure: ProducedMeasure = options.measure === 'total' || options.measure === 'month' || options.measure === 'week'
    ? options.measure : 'day';
  const refits = options.refits === true;
  const anchorRef = useRef<HTMLDivElement>(null);
  const [detail, setDetail] = useState<{ readonly tag: string; readonly key: string; readonly hour: GatedHour } | null>(null);
  const gate = useGate(runtime, widget, SPEC, active);

  const onActivate = useCallback((action: InstalledAction) => {
    const gated = gate.recorded(Math.floor(action.hour));
    if (gated === null) return;
    const meta = action.meta as EquipmentProducedRowMeta | null | undefined;
    setDetail({ tag: meta?.tag ?? action.subject, key: meta?.key ?? action.subject, hour: gated });
  }, [gate]);

  if (!manifestDeclaration) return null;

  return (
    <div ref={anchorRef} className="fh-frame">
      <FamilyWidget
        {...props}
        family={manifestDeclaration.family}
        view={props.view ?? 'race'}
        declaration={manifestDeclaration}
        selectedKey={detail?.key ?? null}
        onActivate={onActivate}
      />
      <DetailWindowHost
        open={detail !== null}
        subject={detail?.tag ?? null}
        hour={detail?.hour ?? null}
        title={detail ? `${detail.tag} equipment produced` : 'Equipment produced'}
        anchor={anchorRef}
        returnFocus=".fh-body"
        onClose={() => setDetail(null)}
      >
        {detail ? (
          <ProducedDetails
            widget={widget}
            runtime={runtime}
            active={active}
            country={detail.key}
            hour={detail.hour}
            count={count}
            measure={measure}
            refits={refits}
            fromHour={measure === 'total' && widget.time.mode === 'range' ? rangeStart(widget, runtime, gate.span, detail.hour) : null}
          />
        ) : null}
      </DetailWindowHost>
    </div>
  );
}

export default { render: EquipmentProducedFamilyWidget } satisfies FamilyViewRenderer;
