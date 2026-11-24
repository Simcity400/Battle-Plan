export {
  marginColour,
  resolveEntityFromReadView,
  resolveOrdinalColour,
  resolveRecordedColour,
  sequentialRampColour,
  signedRampColour,
  splitStripes,
  type HeatPart,
  type ResolvedEntityPresentation,
} from './scale.ts';
export {
  cellAuditValue,
  deriveMatrixAudit,
  deriveMatrixMirror,
  deriveMatrixTooltip,
  matrixCellKey,
  matrixGroupSpans,
  parseMatrixCellKey,
  type HeatMatrixModel,
  type HeatMatrixProjection,
  type MatrixAxisEntity,
  type MatrixCell,
  type MatrixGroup,
  type MatrixGroupSpan,
  type MatrixLegend,
} from './matrixModel.ts';
export {
  hitHeatMatrix,
  layoutHeatMatrix,
  MATRIX_COL_HEADER_HEIGHT,
  MATRIX_COMPACT_WIDTH,
  MATRIX_GROUP_BAND_HEIGHT,
  MATRIX_LEGEND_HEIGHT,
  MATRIX_MAX_CELL,
  MATRIX_MIN_CELL,
  MATRIX_NAME_MAX_WIDTH,
  MATRIX_PAD,
  matrixKeyStep,
  revealMatrixOffset,
  type HeatMatrixLayout,
  type LayoutHeatMatrixInput,
  type MatrixHit,
} from './matrixLayout.ts';
export {
  collectVisibleMatrixArt,
  createMatrixPicture,
  drawHeatMatrix,
  type DrawHeatMatrixInput,
  type HeatMatrixPicture,
} from './matrixPainter.ts';
export { ensureHeatMatrixProjection } from './matrixRegistry.ts';
export {
  isHeatMatrixWidgetDeclaration,
  type AnyHeatMatrixWidgetDeclaration,
  type HeatMatrixWidgetDeclaration,
  type MatrixActivation,
} from './matrixDeclaration.ts';
export {
  familyHeatMatrixLaneView,
  isFamilyHeatMatrixBinding,
  type FamilyHeatMatrixBinding,
  type FamilyHeatMatrixLaneOptions,
} from './matrixLaneView.ts';

export {
  deriveLanesAudit,
  deriveLanesMirror,
  deriveLanesTooltip,
  findRunAtHour,
  laneInspectKey,
  parseLaneInspectKey,
  readoutAuditText,
  type HeatLaneCell,
  type HeatLaneGap,
  type HeatLaneLegend,
  type HeatLaneLegendItem,
  type HeatLaneReadout,
  type HeatLaneRow,
  type HeatLaneRun,
  type HeatLanesModel,
  type HeatLanesProjection,
} from './lanesModel.ts';
export {
  computeAxisTicks,
  hitHeatLanes,
  hourToLaneX,
  LANE_AXIS_HEIGHT,
  LANE_LEGEND_HEIGHT,
  LANE_NAME_COMPACT_WIDTH,
  LANE_NAME_WIDTH,
  LANE_PAD,
  LANE_READOUT_DEFAULT_WIDTH,
  LANE_READOUT_MAX_WIDTH,
  LANE_READOUT_MIN_WIDTH,
  LANE_ROW_DEFAULT,
  LANE_ROW_GAP,
  LANE_ROW_MAX,
  LANE_ROW_MIN,
  laneXToHour,
  layoutHeatLanes,
  projectLaneRunsToPixels,
  type AxisTick,
  type HeatLanesLayout,
  type LaneHit,
  type LayoutHeatLanesInput,
  type PixelLaneSpan,
} from './lanesLayout.ts';
export {
  collectVisibleLaneArt,
  createLanesPicture,
  drawHeatLanes,
  type DrawHeatLanesInput,
  type HeatLanesPicture,
} from './lanesPainter.ts';
export { ensureHeatLanesProjection } from './lanesRegistry.ts';
export {
  isHeatLanesWidgetDeclaration,
  type AnyHeatLanesWidgetDeclaration,
  type HeatLanesWidgetDeclaration,
  type LaneActivation,
} from './lanesDeclaration.ts';
export {
  familyHeatLanesLaneView,
  isFamilyHeatLanesBinding,
  type FamilyHeatLanesBinding,
  type FamilyHeatLanesLaneOptions,
} from './lanesLaneView.ts';
export {
  heatLanesHostAdapter,
  heatMatrixHostAdapter,
  parseHeatLanesHit,
  parseHeatMatrixHit,
} from './hostAdapter.ts';
