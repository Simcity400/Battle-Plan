import { useCallback, useMemo } from 'react';
import type { WidgetSpec } from '../../dashboard/model';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { PlaybackController } from '../../dashboard/playback';
import type { OptionValue } from '../../dashboard/options';
import type { Live } from '../../charts/live.ts';
import FamilyTable, { type FamilyColumn } from '../../draw/tables/FamilyTable.tsx';
import type { TableReading, TableRowModel } from '../../draw/tables/surface.ts';
import type { TableBuild } from '../../draw/tables/builders';
import { encodeTableSettings, tableSettingsOf, TABLE_SETTINGS_KEY } from '../../draw/tables/settings.ts';
import type { ExactRows } from '../../draw/tables/FamilyTable.tsx';
import { buildTable } from '../../draw/tables/builders.ts';
import { repliedCells } from '../../dashboard/follow/liveCells.ts';
import type { TableSettings } from '../../draw/tables/logic.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import { OCCUPATION_TOOLTIP_COLUMNS, type OccupiedStatesReply } from './occupiedStatesData.ts';
import { FORMAT, type OccupiedTableContext } from './occupiedTable.ts';
import { COLUMNS, OPTIONS, TABLE_BASE } from './definition.ts';

interface Props {
  widget: WidgetSpec;
  runtime: WidgetRuntime;
  clock: PlaybackController;
  data: OccupiedStatesReply;
  context: Omit<OccupiedTableContext, 'lawAs'>;
  follow?: number;
  shownAt: number;
  live: Live<{ get(key: string): number | undefined; missing(key: string): string }>;
  countryColours?: ReadonlyMap<string, string>;
  onHover: (key: string | null, at?: { x: number; y: number }, drawn?: TableReading) => void;
  setOption?: (key: string, value: OptionValue) => void;
  exactReply?: (hour: number) => Promise<OccupiedStatesReply>;
  fixed?: number;
  headerSlot: HTMLElement | null;
}

const NO_ROWS: readonly TableRowModel[] = [];
const PRIORITIES: Readonly<Record<string, number>> = { state: 0, resistance: 1, occupier: 2, compliance: 3, garrison: 4, law: 5, owner: 6 };
const TEXT = new Set(['state', 'owner', 'occupier', 'law']);
const SPECS: readonly FamilyColumn[] = COLUMNS.map(column => ({ ...column, priority: PRIORITIES[column.key] ?? 9, numeric: column.key in FORMAT,
  format: FORMAT[column.key], text: TEXT.has(column.key), widest: FORMAT[column.key] === 'fixed5Percent' ? ['100%'] : undefined }));

export default function OccupiedTableSurface({ widget, runtime, clock, data, context, follow, shownAt, live, countryColours, onHover, setOption, exactReply, fixed, headerSlot }: Props) {
  const settings = useMemo(() => tableSettingsOf(resolveOptions(OPTIONS, widget.options).tableSettings, COLUMNS, TABLE_BASE), [widget.options]);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const onSettings = useCallback((next: TableSettings) => setOption?.(TABLE_SETTINGS_KEY, encodeTableSettings(next)), [setOption]);
  const lawAs = settings.columns.law?.as ?? 'dot';
  const build = useMemo<TableBuild>(() => ({ kind: 'occupied-states', context: { ...context, lawAs } }), [context, lawAs]);
  const exactRows = useMemo<ExactRows | undefined>(() => exactReply && (async at => {
    const reply = await exactReply(at), table = buildTable(build, reply, at);
    return table && { rows: table.rows, values: repliedCells(reply).read(), ...table.empty !== null ? { empty: table.empty } : {} };
  }), [exactReply, build]);
  return <FamilyTable label="Occupied states" className="occupied-table-surface" clock={clock} offset={track?.axisOffset ?? 0}
    columns={SPECS} rows={NO_ROWS} reply={follow === undefined ? data : undefined} build={build} fixed={fixed} settings={settings} onSettings={setOption ? onSettings : undefined}
    live={live} follow={follow} shownAt={shownAt} countryColours={countryColours} headerSlot={headerSlot} exactRows={exactRows}
    tooltipColumns={OCCUPATION_TOOLTIP_COLUMNS} onHover={(key, at, drawn) => { if (key !== null && at && drawn) onHover(key, at, drawn); else onHover(null); }} />;
}
