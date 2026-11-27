import { repliedCells } from '../dashboard/follow/liveCells.ts';
import { buildRowSourceFromHead, validateGenericTailReply, type DatedLogRow, type PageFetchAsk, type RowWindow, type SnapshotPage, type TableRowSource, type TailValidator } from './table/rowSource.ts';

export type ExactEntry = {
  readonly name: string;
  readonly hour: number;
} & ({
  readonly kind: 'panel' | 'live' | 'hover';
} | {
  readonly kind: 'rows';
  readonly rowIdentity: string;
  readonly pages: readonly (PageFetchAsk | undefined)[];
  readonly refusedPages?: Readonly<Record<number, SnapshotPage>>;
});

export interface ExactPart {
  readonly name: string;
  readonly hour: number;
  readonly panel?: unknown;
  readonly live?: ReturnType<ReturnType<typeof repliedCells>['read']>;
  readonly hover?: unknown;
  readonly rows?: TableRowSource<DatedLogRow>;
}

export function exactPart(entry: ExactEntry, recorded: number, raws: readonly unknown[], validateTail?: TailValidator): ExactPart {
  const { name, hour, kind } = entry;
  const panel = raws[0];
  if (kind === 'hover') return { name, hour, hover: panel };
  if (kind === 'live') return { name, hour, panel, live: repliedCells(panel).read() };
  if (kind !== 'rows') return { name, hour, panel };
  const validate = validateTail ?? validateGenericTailReply;
  const pages = entry.refusedPages && { ...entry.refusedPages };
  entry.pages.forEach((page, i) => {
    if (page) pages![page.offset] = { ...page, raw: raws[i], reply: validate(raws[i], page.hour) };
  });
  const rows = buildRowSourceFromHead(entry.rowIdentity, entry.pages[0] ? undefined : panel, undefined, recorded, pages, validateTail);
  return { name, hour, panel, ...(rows ? { rows } : {}) };
}

export function exactStatics(recorded: number, parts: readonly (ExactPart | null)[], allowEmpty: boolean, rowWindow?: RowWindow | null) {
  if (parts.length ? !parts[0] : !allowEmpty) return null;
  const panels = new Map<string, unknown>();
  const live = new Map<string, NonNullable<ExactPart['live']>>();
  const hovers = new Map<string, { hour: number; data: unknown }>();
  const rows = new Map<string, TableRowSource<DatedLogRow>>();
  let hour = recorded;
  for (const part of parts) {
    if (!part) continue;
    hour = Math.min(hour, part.hour);
    if (part.panel !== undefined) panels.set(part.name, part.panel);
    if (part.live) live.set(part.name, part.live);
    if (part.hover !== undefined) hovers.set(part.name, { hour: part.hour, data: part.hover });
    if (part.rows !== undefined) rows.set(part.name, part.rows);
  }
  return { hour,
    ...(panels.size ? { panels } : {}),
    ...(live.size ? { live } : {}),
    ...(hovers.size ? { hovers } : {}),
    ...(rows.size ? { rows } : {}),
    ...(rows.size && rowWindow ? { rowWindow } : {}),
  };
}

export type ExactStatics = NonNullable<ReturnType<typeof exactStatics>>;
