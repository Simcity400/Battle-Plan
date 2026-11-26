import { extendLastRun, joinLanePieces, laneRun, missingCell, noRuns, LaneRunBuilder, solidCell, type LanePieces, type LaneRuns } from '../../families/heat/laneRuns.ts';
import { authoredHex, measureColour, surfaceGrey } from '../../charts/palette.ts';
import { airChoiceKey } from '../../dashboard/airOptions.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { contentKey, contentSet } from '../../dashboard/options.ts';
import type { BoundedReadView, SourcePlan } from '../../families/contract.ts';
import { closedDayTiles, dayHistoryNames, dayTileCount, dayTileEntries } from '../../families/heat/dayTiles.ts';
import type { HeatLanesWidgetDeclaration } from '../../families/heat/lanesDeclaration.ts';
import {
  spanIndexAt,
  type HeatLaneCell,
  type HeatLaneReadout,
  type HeatLaneRow,
  type HeatLanesModel,
  type HeatLanesProjection,
} from '../../families/heat/lanesModel.ts';
import { contestedGrey, countryColourAt, readableOn, regionLabel, type DatedColour } from '../../families/heat/scale.ts';
import type { FamilyTooltipPayload } from '../../families/host/tooltips.ts';
import type { FamilyPlanContext } from '../../families/plan.ts';
import { readRegionChoice, regionParams, type RegionChoice } from '../../families/regions/regionChoice.ts';
import { countrySubject, noRegionsBody, regionLanes } from '../../families/regions/regionRows.ts';
import { formatDateOnly, formatShare } from '../../families/valueFormat.ts';
import type { AirDayCell, AirReadout, AirSide, AirTimelineReply, AirTimelineRow } from '../../generated/dashboard';
import { LANE_READOUT_MAX_WIDTH } from '../../families/heat/lanesLayout.ts';
import { mix } from '../../shell/appearance/color.ts';

export const AIR_TOP_DEFAULT = 7;

export interface AirTimelineProjectionOptions {
  readonly viewpointCountry: string | null;
  readonly historyStart: number;
  readonly tiles: number;
  readonly surface: string;
}

interface AirMode {
  readonly mode: 'sides' | 'viewpoint';
  readonly viewpoint: string;
  readonly sides: readonly string[];
  readonly regions: RegionChoice;
  readonly theatres: readonly string[];
}

export function airTimelineMode(ctx: Pick<FamilyPlanContext, 'options' | 'track' | 'countryTag'>): AirMode {
  const options = ctx.options;
  const campaign = ctx.track.source.metadata.campaignKey ?? '';
  const mode = options.mode === 'viewpoint' ? 'viewpoint' as const : 'sides' as const;
  const chosen = options.viewpoint ? String(options.viewpoint) : '';
  const viewpoint = mode !== 'viewpoint'
    ? ''
    : chosen
      ? contentKey(chosen).startsWith('country:') ? contentKey(chosen) : airChoiceKey(campaign, chosen) ?? ''
      : ctx.countryTag ? `country:${ctx.countryTag}` : '';
  const regions = readRegionChoice(options.regions);
  return {
    mode,
    viewpoint,
    sides: contentSet(typeof options.sides === 'string' ? options.sides : '').flatMap(value => airChoiceKey(campaign, value) ?? []),
    regions,
    theatres: contentSet(typeof options.theatres === 'string' ? options.theatres : '').flatMap(value => airChoiceKey(campaign, value) ?? []),
  };
}

export function airTimelineParams(ctx: FamilyPlanContext, mode: AirMode): Record<string, unknown> {
  const top = typeof ctx.options.regionCount === 'number' ? ctx.options.regionCount : AIR_TOP_DEFAULT;
  return {
    branchId: ctx.branch.id,
    mode: mode.mode,
    ...(mode.viewpoint
      ? mode.viewpoint.startsWith('country:')
        ? { viewpoint: countryFromKey(mode.viewpoint.slice(8)), viewpointCountry: true }
        : { viewpoint: mode.viewpoint }
      : {}),
    ...(mode.sides.length ? { sides: mode.sides } : {}),
    ...(mode.theatres.length ? { theatres: mode.theatres } : {}),
    ...regionParams(mode.regions),
    top,
  };
}

const REASONS: Readonly<Record<string, string>> = {
  'theatres-not-recorded': 'Air theatres were not recorded',
  'side-not-present': 'This side was not present',
};

const palettes = new WeakMap<AirTimelineReply, { readonly surface: string; readonly colours: readonly (string | null)[] }>();

function sidePalette(reply: AirTimelineReply, surface: string): readonly (string | null)[] {
  const held = palettes.get(reply);
  if (held && held.surface === surface) return held.colours;
  const colours = reply.sides.map(({ color }) => (color ? readableOn(authoredHex(color), surface) : null));
  palettes.set(reply, { surface, colours });
  return colours;
}

function sideColour(reply: AirTimelineReply, side: number, surface: string, fallback: string): string {
  return sidePalette(reply, surface)[side] ?? fallback;
}

function sideName(side: AirSide | undefined): string {
  return side?.name ?? (side?.nameMissing ? 'Name not recorded' : 'Side not recorded');
}

function depth(track: string, color: string, strength: number): string {
  return mix(track, color, 0.28 + 0.72 * Math.max(0, Math.min(1, strength)));
}

interface Look {
  readonly track: string;
  readonly contested: string;
  readonly enemy: string;
  readonly unknownSide: string;
  readonly viewer: ((hour: number) => DatedColour) | null;
  readonly noViewer: string;
}

function viewerColour(side: number | null, reply: AirTimelineReply, look: Look, surface: string, hour: number): DatedColour {
  if (look.viewer) return look.viewer(hour);
  return { colour: side === null ? look.noViewer : sideColour(reply, side, surface, look.noViewer), through: Infinity };
}

const clear: HeatLaneCell = { kind: 'clear' };
const noDay: HeatLaneCell = { kind: 'missing', reason: 'Not recorded on this day' };
const unknownScore: HeatLaneCell = { kind: 'missing', reason: 'A side’s air score was not recorded' };

function cellOf(cell: AirDayCell | null, reply: AirTimelineReply, look: Look, surface: string, viewer: string, previous?: HeatLaneCell): HeatLaneCell {
  if (!cell) return noDay;
  if (cell.kind === 'reason') return cell.reason === 'outside-theatres' ? clear : missingCell(REASONS[cell.reason] ?? cell.reason, previous);
  if (cell.kind === 'view') {
    if (cell.share_bp === null) return clear;
    const share = cell.share_bp / 10_000;
    return solidCell(share > 0.5 ? depth(look.track, viewer, share)
      : share < 0.5 ? depth(look.track, look.enemy, 1 - share) : look.contested, previous);
  }
  const lead = cell.lead;
  if (lead.kind === 'unknown') return unknownScore;
  if (lead.kind === 'noActivity') return clear;
  return solidCell(lead.kind === 'contested' ? look.contested : depth(look.track, sideColour(reply, lead.side, surface, look.unknownSide), lead.share_bp / 10_000), previous);
}

function airTip(raw: AirDayCell | null, painted: HeatLaneCell, reply: AirTimelineReply, look: Look, surface: string): Omit<FamilyTooltipPayload, 'title' | 'hour'> {
  if (painted.kind === 'missing') return { missingReason: painted.reason };
  if (painted.kind !== 'solid') return { items: [{ label: raw?.kind === 'reason' ? 'Outside the chosen air theatres' : 'No activity' }] };
  if (raw?.kind === 'view') return { items: [{ label: 'Air superiority', value: formatShare(raw.share_bp! / 10_000), swatch: painted.color }] };
  const items = [];
  if (raw?.kind === 'sides') {
    if (raw.lead.kind === 'contested') items.push({ label: 'Contested' });
    let total = 0;
    for (const part of raw.parts) total += Math.max(0, Number(part.friendly));
    for (const part of raw.parts) {
      const score = Math.max(0, Number(part.friendly));
      if (score > 0) items.push({ label: sideName(reply.sides[part.side]), value: formatShare(score / total), swatch: sideColour(reply, part.side, surface, look.unknownSide) });
    }
  }
  return { items };
}

function readoutOf(readout: AirReadout | undefined, reply: AirTimelineReply, look: Look, surface: string): HeatLaneReadout | undefined {
  if (!readout) return undefined;
  switch (readout.kind) {
    case 'notRecorded':
      return { kind: 'missing', reason: REASONS[readout.reason] ?? readout.reason };
    case 'noActivity':
      return { kind: 'clear', label: 'No activity' };
    case 'contested':
      return { kind: 'value', form: 'value', primary: '', secondary: 'Contested', swatch: look.contested, audit: 'Contested' };
    case 'leading': {
      const side = reply.sides[readout.side];
      const pct = formatShare(readout.share_bp / 10_000);
      return { kind: 'value', form: 'value', primary: pct, secondary: sideName(side), swatch: sideColour(reply, readout.side, surface, look.unknownSide) };
    }
    case 'viewpoint': {
      const pct = formatShare(readout.share_bp / 10_000);
      const viewer = viewerColour(readout.side, reply, look, surface, readout.as_of_hour).colour;
      const swatch = readout.share_bp > 5000 ? viewer : readout.share_bp < 5000 ? look.enemy : look.contested;
      return { kind: 'value', form: 'value', primary: pct, secondary: 'Air superiority', swatch };
    }
  }
}

function lane(cells: AirTimelineRow['cells'], reply: AirTimelineReply, look: Look, surface: string, previous: LaneRuns): LaneRuns {
  const runs = new LaneRunBuilder(previous);
  let day = 0;
  for (const [repeat, raw] of cells) {
    const end = day + repeat - 1;
    while (day <= end) {
      const firstHour = reply.firstHour + day * reply.dayHours;
      if (firstHour > reply.lastHour) return runs.finish();
      let until = end;
      let viewer = look.noViewer;
      if (raw?.kind === 'view') {
        const dated = viewerColour(raw.side, reply, look, surface, reply.days[day] ?? firstHour);
        viewer = dated.colour;
        if (dated.through < reply.firstHour + (end + 1) * reply.dayHours - 1) {
          const boundary = Math.floor((dated.through - reply.firstHour) / reply.dayHours);
          const observed = reply.days[boundary] ?? reply.firstHour + boundary * reply.dayHours;
          until = Math.max(day, Math.min(end, observed > dated.through ? boundary - 1 : boundary));
        }
      }
      const old = runs.next;
      const cell = cellOf(raw, reply, look, surface, viewer, old?.cell);
      runs.add(laneRun(firstHour, Math.min(reply.firstHour + (until + 1) * reply.dayHours - 1, reply.lastHour), reply.days[until] ?? null, cell, old));
      day = until + 1;
    }
  }
  return runs.finish();
}

const emptyCells: AirTimelineRow['cells'] = [];
const indexes = new WeakMap<AirTimelineReply, ReadonlyMap<number, AirTimelineRow['cells']>>();
const heldTiles = new WeakMap<AirTimelineReply, { sig: string; runs: Map<AirTimelineRow['cells'], LaneRuns> }>();
const windows = new WeakMap<AirTimelineProjectionOptions, Map<number, LanePieces>>();

interface AirEdge {
  readonly firstHour: number;
  readonly dayHours: number;
  readonly days: AirTimelineReply['days'];
  readonly lastHour: number;
  readonly colours: readonly (string | null)[];
  readonly sig: string;
  readonly runs: LaneRuns;
}

const edges = new WeakMap<AirTimelineRow['cells'], AirEdge>();

function currentLane(cells: AirTimelineRow['cells'], reply: AirTimelineReply, look: Look, surface: string, sig: string, previous: LaneRuns): LaneRuns {
  const old = edges.get(cells);
  const colours = sidePalette(reply, surface);
  const day = Math.floor((reply.lastHour - reply.firstHour) / reply.dayHours);
  let runs: LaneRuns;
  if (!look.viewer && old && old.sig === sig && old.firstHour === reply.firstHour && old.dayHours === reply.dayHours
    && Math.floor((old.lastHour - reply.firstHour) / reply.dayHours) === day && old.runs.at(-1)?.lastHour === old.lastHour
    && reply.days.length === day + 1 && old.days.length === reply.days.length && (old.days === reply.days || reply.days.every((read, i) => i === day || read === old.days[i]))
    && old.colours.length === colours.length && (old.colours === colours || colours.every((colour, i) => colour === old.colours[i]))) {
    runs = extendLastRun(old.runs, reply.lastHour, reply.days[day] ?? null);
  } else {
    runs = lane(cells, reply, look, surface, previous);
  }
  edges.set(cells, { firstHour: reply.firstHour, dayHours: reply.dayHours, days: reply.days, lastHour: reply.lastHour, colours, sig, runs });
  return runs;
}

function tileLanes(reply: AirTimelineReply, look: Look, surface: string) {
  let cells = indexes.get(reply);
  if (!cells) indexes.set(reply, cells = new Map(reply.rows.map(row => [row.region, row.cells])));
  const sig = `${surface}|${look.contested}|${datedSignature(look, reply.firstHour, reply.lastHour)}`;
  let tile = heldTiles.get(reply);
  if (tile?.sig !== sig) heldTiles.set(reply, tile = { sig, runs: new Map() });
  return { reply, cells, sig, runs: tile.runs };
}

function inspectDay(reply: AirTimelineReply, cells: AirTimelineRow['cells'], runs: LaneRuns, label: string, hour: number, look: Look, surface: string): FamilyTooltipPayload | null {
  const day = Math.floor((hour - reply.firstHour) / reply.dayHours);
  if (day < 0 || !cells.length) return null;
  let end = 0;
  let raw: AirDayCell | null = null;
  for (const entry of cells) {
    raw = entry[1];
    end += entry[0];
    if (day < end) break;
  }
  const painted = runs[spanIndexAt(runs, hour, run => run.firstHour)]?.cell;
  if (!painted) return null;
  const tip = airTip(raw, painted, reply, look, surface);
  const read = reply.days[day] ?? null;
  return read === null ? { title: label, subtitle: formatDateOnly(reply.firstHour + day * reply.dayHours), ...tip } : { title: label, hour: read, ...tip };
}

function emptyModel(hour: number, extra: Partial<HeatLanesModel>): HeatLanesModel {
  return { hour, firstHour: hour, lastHour: hour, dayHours: 24, playhead: null, rows: [], ...extra };
}

function datedSignature(look: Look, first: number, last: number): string {
  if (!look.viewer) return '';
  const out: string[] = [];
  for (let at = first; at <= last;) {
    const dated = look.viewer(at);
    out.push(`${at}:${dated.colour}`);
    if (dated.through >= last) break;
    at = dated.through + 1;
  }
  return out.join(',');
}

export function projectAirTimeline(readView: BoundedReadView, options: AirTimelineProjectionOptions): HeatLanesModel {
  const hour: number = readView.hour;
  const previous = windows.get(options);
  windows.delete(options);
  const refused = readView.refused('now') ?? dayHistoryNames(options.tiles).map(name => readView.refused(name)).find(Boolean);
  if (refused) return emptyModel(hour, { refused });
  const now = readView.panel<AirTimelineReply>('now');
  if (!now || !options.surface || now.lastHour > hour) return emptyModel(hour, {});
  const tiles = closedDayTiles<AirTimelineReply>(readView, options.tiles, options.historyStart, now.firstHour);
  if (!tiles) return emptyModel(hour, { waiting: true });
  const through = now.lastHour;
  const surface = options.surface;
  const noViewer = measureColour('bopRight', surface);
  const viewpointCountry = options.viewpointCountry;
  const sideColours = new Set<string>();
  for (const reply of [...tiles, now]) for (const colour of sidePalette(reply, surface)) if (colour) sideColours.add(colour);
  const look: Look = {
    track: surfaceGrey(surface, 0.04),
    contested: contestedGrey(surface, [...sideColours].sort()),
    enemy: measureColour('short', surface),
    unknownSide: surfaceGrey(surface, 0.45),
    viewer: viewpointCountry ? at => countryColourAt(readView, viewpointCountry, at, surface, noViewer) : null,
    noViewer,
  };
  const subject = countrySubject(readView, options.viewpointCountry);
  const pieces = [...tiles, now].map(reply => tileLanes(reply, look, surface));
  const current = new Map<number, LanePieces>();
  windows.set(options, current);
  const rows: HeatLaneRow[] = now.rows.map(row => {
    const { label, nameMissing } = regionLabel(readView, row.region, row.name);
    const readout = readoutOf(row.readout, now, look, surface);
    const old = previous?.get(row.region);
    const parts = pieces.map((piece, i) => {
      const cells = piece.cells.get(row.region) ?? piece.reply.absent ?? emptyCells;
      let runs = piece.runs.get(cells);
      if (!runs) {
        const prior = old?.pieces[i] ?? noRuns;
        runs = piece.reply === now ? currentLane(cells, now, look, surface, piece.sig, prior) : lane(cells, piece.reply, look, surface, prior);
        piece.runs.set(cells, runs);
      }
      return runs;
    });
    const joined = joinLanePieces(parts, old);
    current.set(row.region, joined);
    return {
      id: String(row.region),
      label,
      ...(nameMissing ? { nameMissing } : {}),
      pinned: row.pinned,
      region: row.region,
      runs: joined.runs,
      ...(readout ? { readout } : {}),
    };
  });
  const gaps = new Map<string, { firstHour: number; lastHour: number; reason: string }>();
  for (const reply of [...tiles, now]) for (const gap of reply.gaps) gaps.set(`${gap.firstHour}:${gap.lastHour}`, { firstHour: gap.firstHour, lastHour: gap.lastHour, reason: 'The recording holds no hour here' });
  return {
    hour: through,
    firstHour: tiles[0]?.firstHour ?? now.firstHour,
    lastHour: through,
    dayHours: now.dayHours,
    playhead: through,
    gaps: [...gaps.values()],
    rows,
    regions: { ...regionLanes(subject), active: now.activeUnlisted },
    readoutWidth: LANE_READOUT_MAX_WIDTH,
    inspect: (rowIndex, at) => {
      const row = now.rows[rowIndex];
      const piece = pieces[spanIndexAt(pieces, at, item => item.reply.firstHour)];
      if (!row || !piece || at > piece.reply.lastHour) return null;
      const cells = piece.cells.get(row.region) ?? piece.reply.absent ?? emptyCells;
      return inspectDay(piece.reply, cells, piece.runs.get(cells)!, rows[rowIndex]!.label, at, look, surface);
    },
    ...(now.empty ? noRegionsBody(now.empty, { one: 'active region', many: 'active regions', none: `No air activity${subject ? ` for ${subject}` : ''} at this date.` }) : {}),
    warnings: [...new Set([...tiles, now].flatMap(reply => reply.warnings).concat(rows.length && now.rankMissing ? [now.rankMissing] : []))],
  };
}

export const airTimelineProjection: HeatLanesProjection<AirTimelineProjectionOptions> = {
  kind: 'air-superiority-timeline',
  family: 'heat-lanes',
  project: projectAirTimeline,
};

function airEntries(ctx: FamilyPlanContext, params: Record<string, unknown>): SourcePlan['entries'] {
  const { top, pinnedRegions: _pinned, ...unranked } = params;
  return dayTileEntries(ctx, {
    product: 'air-superiority-timeline',
    span: (base, first, last) => ({ ...base, firstHour: first, lastHour: last }),
    now: {
      params,
      coverage: { mode: 'population', form: 'top', limit: Number(top) },
    },
    tile: {
      params: { ...unranked, every: true },
      coverage: { mode: 'population', form: 'all' },
    },
  });
}

export const airTimelineDeclaration: HeatLanesWidgetDeclaration<AirTimelineProjectionOptions> = {
  family: 'heat-lanes',
  kind: 'air-superiority-timeline',
  label: 'Air superiority timeline',
  subject: 'air superiority',
  emptyCampaign: 'Load a campaign to see its air superiority.',
  icon: 'plane',
  countries: { mode: 'single' },
  unavailable: ctx => {
    const mode = airTimelineMode(ctx);
    return mode.mode === 'viewpoint' && !mode.viewpoint ? 'Choose a dashboard country or a viewpoint in the widget settings.' : null;
  },
  sources: (ctx): SourcePlan => {
    const mode = airTimelineMode(ctx);
    const params = airTimelineParams(ctx, mode);
    if (mode.mode === 'viewpoint' && !mode.viewpoint) {
      return { owner: `air-superiority-timeline:${ctx.branch.id}:none`, entries: [], unavailable: 'Choose a dashboard country or a viewpoint in the widget settings.' };
    }
    return { owner: `air-superiority-timeline:${JSON.stringify(params)}`, entries: airEntries(ctx, params) };
  },
  projection: airTimelineProjection,
  projectionOptions: (ctx, extra) => {
    const viewpoint = airTimelineMode(ctx).viewpoint;
    return { viewpointCountry: viewpoint.startsWith('country:') ? viewpoint.slice(8) : null, historyStart: ctx.staticSpan?.[0] ?? ctx.branch.firstHour, tiles: dayTileCount(ctx.branch), surface: extra.surface };
  },
  jumpToDate: true,
};
