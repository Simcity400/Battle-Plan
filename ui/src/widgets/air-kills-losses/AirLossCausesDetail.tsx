import WidgetState from '../../components/WidgetState';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { useProduct } from '../../dashboard/products';
import type { BoundHour, GatedHour } from '../../dashboard/regionTime';
import { productRequest } from '../../dashboard/request.ts';
import { formatBarsValueField } from '../../families/bars/model.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { MilitaryAirAtReply } from '../../generated/dashboard';
import { AIR_LOSSES_AT, causeLabel, militaryAirAmount, validateMilitaryAirAt, type MilitaryAirRowMeta } from '../bombing-damage/militaryAirAt.ts';

export interface AirLossCausesDetailProps {
  readonly runtime: WidgetRuntime;
  readonly trackId: string | undefined;
  readonly row: MilitaryAirRowMeta;
  readonly hour: GatedHour;
  readonly from: BoundHour | null;
  readonly active: boolean;
}

export function AirLossCausesDetail({ runtime, trackId, row, hour, from, active }: AirLossCausesDetailProps) {
  const track = runtime.tracks.find(item => item.id === trackId);
  const slot = Number(row.identity.split(':')[0]);
  const request = track && Number.isSafeInteger(slot) ? productRequest({
    sourceId: track.source.id,
    revision: track.source.revision,
    product: 'military-air-at',
    params: {
      branchId: track.branch.id, family: 'airLosses', regions: [...row.filters.regions], causes: [...row.filters.causes],
      ...(from !== null ? { fromHour: from } : {}),
      hour, metric: 'taken', countries: [slot],
    },
  }) : null;
  const { data, error, retry } = useProduct<MilitaryAirAtReply>(runtime.client, request, active, validateMilitaryAirAt, track?.source);
  if (!track) return null;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading aircraft loss causes…" />;
  if (data.missing !== null) return <WidgetState kind="unavailable" message={data.missing} />;
  const held = data.rows.find(item => item.identity === row.identity);
  if (!held) return <WidgetState kind="unavailable" message="This country is not present at this date." />;
  const text = (raw: string) => formatBarsValueField(militaryAirAmount(raw, held.partial, AIR_LOSSES_AT)).exact;
  return (
    <div className="navy-management-content">
      <p>Own aircraft lost by cause on {formatDateOnly(hour)}: {text(held.taken)} in all.</p>
      {held.partial ? <p>{held.partial}</p> : null}
      {held.causes.length ? (
        <ul>
          {held.causes.map(cause => <li key={cause.cause}>{causeLabel(cause)}: {text(cause.taken)}{cause.missing ? ` (${cause.missing})` : ''}</li>)}
        </ul>
      ) : held.partial ? null : <p>No aircraft lost yet.</p>}
    </div>
  );
}
