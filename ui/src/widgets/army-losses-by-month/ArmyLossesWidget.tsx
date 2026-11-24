import { useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import { useGate } from '../../dashboard/regionGate';
import { useHeadProduct } from '../../dashboard/headProduct';
import { at as pickAt, SPEC } from '../../dashboard/regionTime.ts';
import { resolveOptions } from '../../dashboard/options';
import { liveKey, type LiveValues } from '../../charts/live.ts';
import { DeclaredFamilyWidget, DetailWindowHost } from '../../families/host/index.ts';
import { timeChartHostAdapter } from '../../families/time-chart/hostAdapter.ts';
import type { ArmyLossesReply, ArmyLossMonthArmy } from '../../generated/dashboard';
import { OPTIONS } from './definition.ts';
import {
  armyLossesDeclaration,
  formatLossRaw,
  formatMonthTitle,
  LEDGERS,
  LEDGER_LABELS,
  monthOfHour,
  resolveArmyLossesOptions,
  validateArmyLosses,
} from './armyLossesProjection.ts';

function resolveManpower(row: ArmyLossMonthArmy, live: LiveValues<number>): readonly [number, number, number] | null {
  if (!row.manpower) return null;
  if (!row.manpowerLiveRow) return row.manpower;
  const g = live.get(liveKey(row.manpowerLiveRow, 'ground'));
  const a = live.get(liveKey(row.manpowerLiveRow, 'air'));
  const n = live.get(liveKey(row.manpowerLiveRow, 'navy'));
  if (g === undefined || a === undefined || n === undefined) return null;
  return [Math.round(g), Math.round(a), Math.round(n)];
}

function resolveEquipmentTotal(
  row: ArmyLossMonthArmy,
  wantedLedgers: readonly number[],
  live: LiveValues<number>,
): { readonly text: string; readonly missing?: string } {
  const matched = row.ledgers.filter(l => wantedLedgers.includes(l.ledger));
  if (!matched.length) return { text: 'Not recorded', missing: 'No equipment ledger row kept for this army in this month.' };
  let total = 0n;
  for (const ledger of matched) {
    if (!ledger.kept || ledger.missing !== null || ledger.totalRaw === null) {
      return { text: 'Not recorded', missing: ledger.missing ?? 'Not recorded' };
    }
    if (ledger.liveRow) {
      const key = liveKey(ledger.liveRow, 'amount');
      const liveVal = live.get(key);
      if (liveVal === undefined) {
        return { text: 'Not recorded', missing: live.missing(key) };
      }
      total += BigInt(Math.round(liveVal));
      continue;
    }
    total += BigInt(ledger.totalRaw);
  }
  return { text: formatLossRaw(total, 'equipment') };
}

export default function ArmyLossesWidget(props: WidgetProps) {
  const { widget, runtime, active, setOption } = props;
  const anchor = useRef<HTMLDivElement>(null);
  const [picked, setPicked] = useState<{ readonly month: number; readonly hour: number } | null>(null);
  const options = resolveOptions(OPTIONS, widget.options);
  const { measure, ledger } = resolveArmyLossesOptions(options);
  const drillGate = useGate(
    runtime,
    widget,
    picked ? pickAt(picked.hour) : SPEC,
    active && picked !== null,
    { country: { stored: widget.options?.country, setOption } },
  );
  const tag = drillGate.open ? drillGate.country?.tag ?? null : null;
  const drillState = useHeadProduct<ArmyLossesReply, 'army-losses'>(
    runtime,
    drillGate,
    'army-losses',
    tag && picked !== null ? { country: countryFromKey(tag), view: { kind: 'month', month: picked.month } } : null,
    { active: active && picked !== null, validate: validateArmyLosses },
  );
  const detail = drillState.data?.monthDetail ?? null;
  const live = drillState.live.read();
  const wantedLedgers = LEDGERS[ledger];
  const caption = measure === 'manpower' ? 'Manpower lost' : LEDGER_LABELS[ledger];
  const title = picked ? `${caption} \u00b7 ${formatMonthTitle(picked.month)}` : 'Month detail';

  return (
    <div ref={anchor} style={{ height: '100%', minHeight: 0 }}>
      <DeclaredFamilyWidget
        {...props}
        adapter={timeChartHostAdapter}
        declaration={armyLossesDeclaration}
        onActivate={action => {
          if (action.subject.startsWith('month:') && Number.isFinite(action.hour)) {
            setPicked({ month: monthOfHour(action.hour), hour: action.hour });
          }
        }}
      />
      <DetailWindowHost
        open={picked !== null}
        subject={picked ? String(picked.month) : null}
        hour={picked?.hour ?? null}
        title={title}
        day={picked ? formatMonthTitle(picked.month) : ''}
        anchor={anchor}
        onClose={() => setPicked(null)}
      >
        {drillState.error ? (
          <WidgetState kind="error" failure={drillState.error} onRetry={drillState.retry} />
        ) : !detail ? (
          <WidgetState kind="loading" label="Loading month breakdown\u2026" />
        ) : !detail.armies.length ? (
          <WidgetState kind="empty" message="No army kept an entry for this month." />
        ) : (
          <table className="data-table" style={{ width: '100%', borderCollapse: 'collapse' }}>
            <thead>
              <tr>
                <th style={{ textAlign: 'left', padding: '4px 8px' }}>Army</th>
                {measure === 'manpower' ? (
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
              {detail.armies.map(row => {
                const mp = resolveManpower(row, live);
                const eq = measure === 'equipment' ? resolveEquipmentTotal(row, wantedLedgers, live) : null;
                return (
                  <tr key={row.army.reference}>
                    <td style={{ padding: '4px 8px' }}>{row.army.name ?? 'Army name unavailable'}</td>
                    {measure === 'manpower' ? (
                      mp ? (
                        <>
                          <td style={{ textAlign: 'right', padding: '4px 8px' }}>{formatLossRaw(BigInt(mp[0]), 'manpower')}</td>
                          <td style={{ textAlign: 'right', padding: '4px 8px' }}>{formatLossRaw(BigInt(mp[1]), 'manpower')}</td>
                          <td style={{ textAlign: 'right', padding: '4px 8px' }}>{formatLossRaw(BigInt(mp[2]), 'manpower')}</td>
                        </>
                      ) : (
                        <td colSpan={3} style={{ textAlign: 'right', padding: '4px 8px', opacity: 0.6 }}>Not recorded</td>
                      )
                    ) : (
                      <td style={{ textAlign: 'right', padding: '4px 8px', opacity: eq?.missing ? 0.6 : 1 }} title={eq?.missing}>
                        {eq?.text}
                      </td>
                    )}
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </DetailWindowHost>
    </div>
  );
}
