import { useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { FamilyViewProps } from '../../dashboard/definitions';
import { repliedCells } from '../../dashboard/follow/liveCells.ts';
import { useProduct } from '../../dashboard/products';
import { useGate } from '../../dashboard/regionGate';
import { productRequest } from '../../dashboard/request.ts';
import { DetailWindowHost } from '../../families/host/index.ts';
import { FamilyWidget } from '../../families/host/registry.tsx';
import type { ArmyLossesReply } from '../../generated/dashboard';
import {
  formatMonthTitle,
  LEDGERS,
  LEDGER_LABELS,
  type LossLedger,
  type LossMeasure,
  monthOfHour,
  resolveArmyLossesOptions,
  resolveArmyMonthDetail,
  validateArmyLosses,
} from './armyLossesProjection.ts';

interface PickedMonth {
  readonly month: number;
  readonly frameHour: number;
  readonly country: string;
  readonly measure: LossMeasure;
  readonly ledger: LossLedger;
}

export default function ArmyLossesWidget(props: FamilyViewProps) {
  const { widget, runtime, active, options } = props;
  const anchor = useRef<HTMLDivElement>(null);
  const [picked, setPicked] = useState<PickedMonth | null>(null);
  const { measure, ledger } = resolveArmyLossesOptions(options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = selectedCountryKeys(widget.countries, track)[0] ?? null;
  const gate = useGate(runtime, widget);
  const recordedHour = picked ? gate.recorded(picked.frameHour) : null;
  const monthReq = track && picked && recordedHour !== null
    ? productRequest({
        sourceId: track.source.id,
        revision: track.source.revision,
        product: 'army-losses',
        params: {
          branchId: track.branch.id,
          country: countryFromKey(picked.country),
          view: picked.measure === 'equipment'
            ? { kind: 'month', month: picked.month, ledgers: [...LEDGERS[picked.ledger]] }
            : { kind: 'month', month: picked.month },
          hour: recordedHour,
        },
      })
    : null;
  const drillState = useProduct<ArmyLossesReply>(runtime.client, monthReq, active && picked !== null, validateArmyLosses, track?.source);
  const live = repliedCells(drillState.data).read();
  const detailMeasure = picked?.measure ?? measure;
  const detailLedger = picked?.ledger ?? ledger;
  const resolved = drillState.data ? resolveArmyMonthDetail(drillState.data, detailMeasure, detailLedger, live) : null;
  const caption = detailMeasure === 'manpower' ? 'Manpower lost' : LEDGER_LABELS[detailLedger];
  const title = picked ? `${caption} \u00b7 ${formatMonthTitle(picked.month)}` : 'Month detail';

  return (
    <div ref={anchor} style={{ height: '100%', minHeight: 0 }}>
      {props.manifestDeclaration ? <FamilyWidget
        {...props}
        family={props.manifestDeclaration.family} declaration={props.manifestDeclaration}
        onActivate={action => {
          if (!action.subject.startsWith('month:')) return;
          const meta = action.meta as { readonly frameHour?: number; readonly monthFirstHour?: number; readonly country?: string } | undefined;
          const frameHour = typeof meta?.frameHour === 'number' && Number.isFinite(meta.frameHour) ? meta.frameHour : action.hour;
          const monthHour = typeof meta?.monthFirstHour === 'number' && Number.isFinite(meta.monthFirstHour) ? meta.monthFirstHour : action.hour;
          const country = meta?.country ?? tag;
          if (!country || !Number.isFinite(frameHour) || !Number.isFinite(monthHour)) return;
          setPicked({ month: monthOfHour(monthHour), frameHour, country, measure, ledger });
        }}
      /> : null}
      <DetailWindowHost
        open={picked !== null}
        subject={picked ? String(picked.month) : null}
        hour={picked?.frameHour ?? null}
        title={title}
        day={picked ? formatMonthTitle(picked.month) : ''}
        anchor={anchor}
        onClose={() => setPicked(null)}
      >
        {drillState.error ? (
          <WidgetState kind="error" failure={drillState.error} onRetry={drillState.retry} />
        ) : !resolved ? (
          <WidgetState kind="loading" label="Loading month breakdown\u2026" />
        ) : resolved.kind === 'absent' ? (
          <WidgetState kind="empty" message="This country does not exist at this date." />
        ) : resolved.kind === 'unavailable' ? (
          <WidgetState kind="empty" message="The combat log wasn\u2019t recorded at this time." detail={resolved.reason} />
        ) : resolved.kind === 'empty' ? (
          <WidgetState kind="empty" message="No army kept an entry for this month." />
        ) : (
          <table className="data-table" style={{ width: '100%', borderCollapse: 'collapse' }}>
            <thead>
              <tr>
                <th style={{ textAlign: 'left', padding: '4px 8px' }}>Army</th>
                {detailMeasure === 'manpower' ? (
                  <>
                    <th style={{ textAlign: 'right', padding: '4px 8px' }}>Ground</th>
                    <th style={{ textAlign: 'right', padding: '4px 8px' }}>Air</th>
                    <th style={{ textAlign: 'right', padding: '4px 8px' }}>Navy</th>
                  </>
                ) : (
                  <th style={{ textAlign: 'right', padding: '4px 8px' }}>{caption}</th>
                )}
              </tr>
            </thead>
            <tbody>
              {resolved.rows.map(row => (
                <tr key={row.reference}>
                  <td style={{ padding: '4px 8px' }}>{row.name}</td>
                  {detailMeasure === 'manpower' ? (
                    row.manpower ? (
                      <>
                        <td style={{ textAlign: 'right', padding: '4px 8px' }}>{row.manpower[0]}</td>
                        <td style={{ textAlign: 'right', padding: '4px 8px' }}>{row.manpower[1]}</td>
                        <td style={{ textAlign: 'right', padding: '4px 8px' }}>{row.manpower[2]}</td>
                      </>
                    ) : (
                      <td colSpan={3} style={{ textAlign: 'right', padding: '4px 8px', opacity: 0.6 }} title={row.manpowerMissing}>Not recorded</td>
                    )
                  ) : (
                    <td style={{ textAlign: 'right', padding: '4px 8px', opacity: row.equipmentMissing ? 0.6 : 1 }} title={row.equipmentMissing}>
                      {row.equipmentText ?? 'Not recorded'}
                    </td>
                  )}
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </DetailWindowHost>
    </div>
  );
}
