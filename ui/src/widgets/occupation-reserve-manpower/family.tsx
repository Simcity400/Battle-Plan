import { useCallback, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { FamilyViewProps, FamilyViewRenderer } from '../../dashboard/definitions';
import { useProduct } from '../../dashboard/products';
import { useGate } from '../../dashboard/regionGate';
import { SPEC, type GatedHour } from '../../dashboard/regionTime';
import { productRequest } from '../../dashboard/request.ts';
import { DetailWindowHost } from '../../families/host/index.ts';
import { FamilyWidget } from '../../families/host/registry.tsx';
import type { InstalledAction } from '../../families/plan.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { OccupationReserveReply } from '../../generated/dashboard';
import { persons, validateOccupationReserve } from './occupationReserveData';
import type { OccupationReserveRowMeta } from './occupationReserveRaceProjection.ts';
import './occupationReserve.css';

function HolderSourcesDetail({ runtime, trackId, tag, hour, active }: {
  readonly runtime: FamilyViewProps['runtime'];
  readonly trackId: string | undefined;
  readonly tag: string;
  readonly hour: GatedHour;
  readonly active: boolean;
}) {
  const track = runtime.tracks.find(item => item.id === trackId);
  const request = track ? productRequest({
    sourceId: track.source.id,
    revision: track.source.revision,
    product: 'occupation-reserve-manpower',
    params: { branchId: track.branch.id, countries: [countryFromKey(tag)], hour },
  }) : null;
  const { data, error, retry } = useProduct<OccupationReserveReply>(runtime.client, request, active, validateOccupationReserve, track?.source);
  if (!track) return null;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading holder reserve…" />;
  if (data.missing !== null) return <WidgetState kind="unavailable" message={data.missing} />;
  const holder = data.holders[0];
  if (!holder || !holder.entries.length) {
    return <WidgetState kind="empty" icon="flag" message="No reserve manpower held at this date." />;
  }
  const positive = holder.entries.filter(entry => entry.amount >= 0);
  const negative = holder.entries.filter(entry => entry.amount < 0);
  return (
    <div className="occupation-holder">
      <p>Reserve manpower held on {formatDateOnly(data.readHour ?? hour)}: {persons(holder.total)}</p>
      <ul>
        {positive.map((entry, index) => (
          <li key={index}>{entry.source.tag ?? 'Source country unavailable'}: {persons(entry.amount)}</li>
        ))}
      </ul>
      {negative.length > 0 ? (
        <dl className="occupation-negative" aria-label="Negative entries">
          <dt>Negative entries</dt>
          {negative.map((entry, index) => (
            <dd key={index}><span>{entry.source.tag ?? 'Source country unavailable'}</span><span>{persons(entry.amount)}</span></dd>
          ))}
        </dl>
      ) : null}
    </div>
  );
}

function OccupationReserveFamilyWidget(props: FamilyViewProps) {
  const { widget, runtime, active, manifestDeclaration } = props;
  const isBreakdown = manifestDeclaration?.family === 'time-chart';
  const anchorRef = useRef<HTMLDivElement>(null);
  const [detail, setDetail] = useState<{ readonly tag: string; readonly key: string; readonly hour: GatedHour } | null>(null);
  const gate = useGate(runtime, widget, SPEC, active);

  const onActivate = useCallback((action: InstalledAction) => {
    const gated = gate.recorded(Math.floor(action.hour));
    if (gated === null) return;
    const meta = action.meta as OccupationReserveRowMeta | null | undefined;
    setDetail({ tag: meta?.tag ?? action.subject, key: meta?.key ?? action.subject, hour: gated });
  }, [gate]);

  if (!manifestDeclaration) return null;
  if (isBreakdown) {
    return (
      <div className="fh-frame">
        <FamilyWidget {...props} family={manifestDeclaration.family} declaration={manifestDeclaration} />
      </div>
    );
  }

  return (
    <div ref={anchorRef} className="fh-frame">
      <FamilyWidget
        {...props}
        family={manifestDeclaration.family}
        declaration={manifestDeclaration}
        selectedKey={detail?.key ?? null}
        onActivate={onActivate}
      />
      <DetailWindowHost
        open={detail !== null}
        subject={detail?.tag ?? null}
        hour={detail?.hour ?? null}
        title={detail ? `${detail.tag} occupation reserve` : 'Holder breakdown'}
        anchor={anchorRef}
        returnFocus=".fh-body"
        onClose={() => setDetail(null)}
      >
        {detail ? (
          <HolderSourcesDetail
            runtime={runtime}
            trackId={widget.trackIds[0]}
            tag={detail.tag}
            hour={detail.hour}
            active={active}
          />
        ) : null}
      </DetailWindowHost>
    </div>
  );
}

export default { render: OccupationReserveFamilyWidget } satisfies FamilyViewRenderer;
