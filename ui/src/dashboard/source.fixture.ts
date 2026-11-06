import type { DashboardSource } from './client';

export function syntheticDashboardSource(id: string, revision: string, path = `${id}.battleplan`): DashboardSource {
  return { id, path, revision, metadata: { path, revision, name: id, format: 1, captureMode: 'full', campaignKey: 'c0ffee',
    branches: [{ id: 'branch', sessionId: 'session', firstHour: 0, lastHour: 1000,
      parentId: null, parentHour: null, sampleBoundary: 'Sample origin unknown', gaps: [] }],
    countries: [], playerCountryId: null, contentVerified: false, partial: null, warnings: [], buildWarnings: [] } };
}
