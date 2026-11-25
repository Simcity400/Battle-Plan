import type { FamilyHostAdapter } from '../host/adapter.ts';
import { serializeTiledBySource } from '../sceneTiles.ts';
import type { FamilyMeterBinding } from './laneView.ts';
import { deriveMeterAudit, type MeterModel } from './model.ts';

export const meterHostAdapter: FamilyHostAdapter<MeterModel, unknown, FamilyMeterBinding> = {
  family: 'meters',
  skeleton: 'table',
  scroll: 'vertical',
  navigation: 'rows',
  mirror: 'listbox',
  auditOwnerId: -6,
  getScrollExtent: fields => {
    const extent = fields.extent as { readonly height?: number } | undefined;
    return extent?.height !== undefined ? { height: extent.height } : null;
  },
  deriveAuditRows: model => deriveMeterAudit(model),
  auditOrdered: true,
  buildBinding: ctx => (ctx.tiledSeries ? { tiledBySource: serializeTiledBySource(ctx.tiledSeries) } : {}),
};
