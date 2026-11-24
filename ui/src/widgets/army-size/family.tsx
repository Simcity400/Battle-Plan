import { useCallback, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { OptionChoices, WidgetProps, WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import { useGate } from '../../dashboard/regionGate';
import { SPEC, type GatedHour } from '../../dashboard/regionTime';
import { productRequest } from '../../dashboard/request.ts';
import { forcePartLabel, validateForceAtReply } from '../../families/bars/forceAtRace.ts';
import { DetailWindowHost } from '../../families/host/index.ts';
import { FamilyWidget } from '../../families/host/registry.tsx';
import type { InstalledAction } from '../../families/plan.ts';
import { parseChartInspect } from '../../families/time-chart/hitTest.ts';
import { formatDateOnly, formatScalar } from '../../families/valueFormat.ts';
import type { ArmyAttribution, ForceAtReply } from '../../generated/dashboard';
import { armyMetadataParams, measureChoices, validateArmyMetadata, type ArmyMetadataReply } from './armySizeMeta.ts';

function useArmyChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track ? productRequest({
    sourceId: track.source.id,
    revision: track.source.revision,
    product: 'army-size',
    params: armyMetadataParams(track.branch.id),
  }) : null;
  const { data } = useProduct<ArmyMetadataReply>(runtime.client, request, true, validateArmyMetadata, track?.source);
  return { measure: measureChoices(data) };
}

function ArmyAtDetail({ runtime, trackId, tag, hour, countFor, active }: {
  readonly runtime: WidgetProps['runtime'];
  readonly trackId: string | undefined;
  readonly tag: string;
  readonly hour: GatedHour;
  readonly countFor: ArmyAttribution;
  readonly active: boolean;
}) {
  const track = runtime.tracks.find(item => item.id === trackId);
  const request = track ? productRequest({
    sourceId: track.source.id,
    revision: track.source.revision,
    product: 'army-at',
    params: { branchId: track.branch.id, countries: [countryFromKey(tag)], countFor, hour },
  }) : null;
  const { data, error, retry } = useProduct<ForceAtReply>(runtime.client, request, active, validateForceAtReply, track?.source);
  if (!track) return null;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading army breakdown…" />;
  if (data.missing !== null) return <WidgetState kind="unavailable" message={data.missing} />;
  const row = data.rows[0];
  if (!row) return <WidgetState kind="empty" icon="shield" message="No battalions recorded at this date." />;
  if (row.missing !== null) return <WidgetState kind="unavailable" message={row.missing} />;
  if (row.partsMissing !== null) return <WidgetState kind="unavailable" message={row.partsMissing} />;
  const items = data.parts
    .map((part, index) => ({ key: part.key, label: forcePartLabel(part), value: row.values[index] ?? null }))
    .filter(item => item.value === null || item.value > 0);
  return (
    <div className="navy-management-content">
      <p>Battalions by unit group on {formatDateOnly(hour)} ({row.total === null ? 'Not recorded' : formatScalar(row.total, { decimals: 0 })} total).</p>
      {items.length ? (
        <ul>
          {items.map(item => (
            <li key={item.key}>{item.label}: {item.value === null ? 'Not recorded' : formatScalar(item.value, { decimals: 0 })}</li>
          ))}
        </ul>
      ) : (
        <p>No battalions recorded at this date.</p>
      )}
    </div>
  );
}

function ArmySizeFamilyWidget(props: WidgetProps) {
  const { widget, runtime, active, view, manifestDeclaration } = props;
  const isLines = manifestDeclaration?.family === 'time-chart';
  const countFor = (widget.options?.countFor === 'host' ? 'host' : 'owner') as ArmyAttribution;
  const anchorRef = useRef<HTMLDivElement>(null);
  const [detail, setDetail] = useState<{ readonly tag: string; readonly key: string; readonly hour: GatedHour } | null>(null);
  const gate = useGate(runtime, widget, SPEC, active);

  const onActivate = useCallback((action: InstalledAction) => {
    if (isLines && parseChartInspect(action.subject) !== null) return;
    const gated = gate.recorded(Math.floor(action.hour));
    if (gated === null) return;
    const meta = action.meta as { readonly tag?: string; readonly key?: string } | null | undefined;
    setDetail({ tag: meta?.tag ?? action.subject, key: meta?.key ?? action.subject, hour: gated });
  }, [gate, isLines]);

  return (
    <div ref={anchorRef} className="fh-frame">
      {manifestDeclaration ? (
        <FamilyWidget
          {...props}
          family={manifestDeclaration.family}
          view={view}
          declaration={manifestDeclaration}
          {...(isLines ? {} : { selectedKey: detail?.key ?? null })}
          onActivate={onActivate}
        />
      ) : null}
      <DetailWindowHost
        open={detail !== null}
        subject={detail?.tag ?? null}
        hour={detail?.hour ?? null}
        title={detail ? `${detail.tag} army breakdown` : 'Army breakdown'}
        anchor={anchorRef}
        returnFocus=".fh-body"
        onClose={() => setDetail(null)}
      >
        {detail ? (
          <ArmyAtDetail
            runtime={runtime}
            trackId={widget.trackIds[0]}
            tag={detail.tag}
            hour={detail.hour}
            countFor={countFor}
            active={active}
          />
        ) : null}
      </DetailWindowHost>
    </div>
  );
}

export default { render: ArmySizeFamilyWidget, useOptionChoices: useArmyChoices } satisfies WidgetRenderer;
