import { icDecimals } from '../../dashboard/icDecimals.ts';
import { breakdownAtHourModel, breakdownAtHourSource, breakdownRange, validateBreakdownAtHour } from '../../families/breakdown/atHour.ts';
import type { BreakdownWidgetSpec } from '../../families/breakdown/declaration.ts';
import type { BreakdownModel, BreakdownProjection } from '../../families/breakdown/model.ts';
import type { BoundedReadView, FamilyPlanContext } from '../../families/plan.ts';
import { formatRational } from '../../families/valueFormat.ts';
import type { ProductionSpendingReply, ProductionSpendingScope } from '../../generated/dashboard';

export interface ProductionSpendingOptions {
  readonly digits: number;
  readonly nothing: string;
  readonly notRecorded: string;
}

const validate = validateBreakdownAtHour<ProductionSpendingReply>('production spending breakdown');

function projectSpending(kind: string): BreakdownProjection<ProductionSpendingOptions> {
  return {
    family: 'breakdown',
    kind,
    project(readView: BoundedReadView, options: ProductionSpendingOptions): BreakdownModel {
      const reply = readView.panel<ProductionSpendingReply>('main');
      return breakdownAtHourModel(Math.floor(readView.hour), reply, {
        unit: 'IC',
        verb: 'Used',
        since: 'Used since',
        nothing: options.nothing,
        notRecorded: options.notRecorded,
        format: (numerator, denominator) => formatRational(numerator, denominator, { maxDecimals: options.digits }),
      });
    },
  };
}

function spendingParams(ctx: FamilyPlanContext, scope: ProductionSpendingScope, includeRefits?: boolean): Readonly<Record<string, unknown>> {
  return { scope, ...(includeRefits === undefined ? {} : { includeRefits }), ...breakdownRange(ctx) };
}

function productionScope(ctx: FamilyPlanContext): ProductionSpendingScope {
  return ctx.options.factories === 'dockyards' ? 'dockyards' : 'military';
}

function factoryScope(ctx: FamilyPlanContext): ProductionSpendingScope {
  return ctx.options.equipment === 'land' ? 'landPurposes' : 'airPurposes';
}

export const productionSpendingProjection = projectSpending('production-spending');

export const productionSpendingBreakdown: BreakdownWidgetSpec<ProductionSpendingOptions> = {
  kind: 'production-spending',
  label: 'Military production spending',
  subject: 'production spending',
  emptyCampaign: 'Load a campaign to see production spending.',
  icon: 'sunburst',
  countries: { mode: 'single', choose: 'Choose a country to see its production spending.' },
  projection: productionSpendingProjection,
  projectionOptions: ctx => ({
    digits: icDecimals(ctx.options, 5),
    nothing: ctx.options.factories === 'dockyards' ? 'dockyard spending' : 'military production spending',
    notRecorded: 'Production spending was not recorded.',
  }),
  sources: ctx => breakdownAtHourSource(ctx, 'production-spending', spendingParams(ctx, productionScope(ctx), ctx.options.refits === true), validate),
};

export const dockyardSpendingProjection = projectSpending('dockyard-spending');

export const dockyardSpendingBreakdown: BreakdownWidgetSpec<ProductionSpendingOptions> = {
  kind: 'dockyard-spending',
  label: 'Dockyard spending',
  subject: 'dockyard spending',
  emptyCampaign: 'Load a campaign to see dockyard spending.',
  icon: 'sunburst',
  countries: { mode: 'single', choose: 'Choose a country to see its dockyard spending.' },
  projection: dockyardSpendingProjection,
  projectionOptions: ctx => ({
    digits: icDecimals(ctx.options, 5),
    nothing: 'dockyard spending',
    notRecorded: 'Dockyard spending was not recorded.',
  }),
  sources: ctx => breakdownAtHourSource(ctx, 'production-spending', spendingParams(ctx, 'dockyardPurposes'), validate),
};

export const militaryFactorySpendingProjection = projectSpending('military-factory-spending');

export const militaryFactorySpendingBreakdown: BreakdownWidgetSpec<ProductionSpendingOptions> = {
  kind: 'military-factory-spending',
  label: 'Military factory spending',
  subject: 'military factory spending',
  emptyCampaign: 'Load a campaign to see military factory spending.',
  icon: 'sunburst',
  countries: { mode: 'single', choose: 'Choose a country to see its military factory spending.' },
  projection: militaryFactorySpendingProjection,
  projectionOptions: ctx => ({
    digits: icDecimals(ctx.options, 5),
    nothing: ctx.options.equipment === 'land' ? 'land factory spending' : 'air factory spending',
    notRecorded: 'Military factory spending was not recorded.',
  }),
  sources: ctx => breakdownAtHourSource(ctx, 'production-spending', spendingParams(ctx, factoryScope(ctx)), validate),
};
