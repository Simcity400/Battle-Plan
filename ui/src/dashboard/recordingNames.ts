export const recordingFilename = (path: string) => path.replaceAll('\\', '/').split('/').pop()!;

export const recordingDisplayName = (path: string) => recordingFilename(path).replace(/\.(?:battleplan|hoi4replay)$/i, '');

export function recordingNameMatches(path: string, query: string): boolean {
  return recordingFilename(path).toLowerCase().includes(query.trim().toLowerCase());
}

const pathKey = (path: string) => path.replaceAll('\\', '/').toLowerCase();
const parentSegments = (path: string) => path.replaceAll('\\', '/').split('/').filter(Boolean).slice(0, -1);
const isLegacyName = (path: string) => /\.hoi4replay$/i.test(path);

export interface RecordingQualifier { folder: string | null; legacy: boolean }

export function distinguishingFolders(paths: readonly string[]): Map<string, RecordingQualifier> {
  const groups = new Map<string, string[]>();
  for (const path of new Map(paths.map(path => [pathKey(path), path])).values()) {
    const name = recordingDisplayName(path).toLowerCase();
    groups.set(name, [...groups.get(name) ?? [], path]);
  }
  const qualifiers = new Map<string, RecordingQualifier>();
  const tail = (segments: string[], count: number) => segments.slice(-count).join('\\');
  for (const group of groups.values()) {
    if (group.length < 2) continue;
    const parents = new Map(group.map(path => [tail(parentSegments(path), Infinity).toLowerCase(), parentSegments(path)]));
    const folders = new Map<string, string>();
    for (const [key, segments] of parents) {
      if (parents.size < 2) break;
      for (let count = 1; count <= segments.length; count++) {
        const suffix = tail(segments, count).toLowerCase();
        if ([...parents].every(([other, at]) => other === key || tail(at, count).toLowerCase() !== suffix)) {
          folders.set(key, tail(segments, count));
          break;
        }
      }
    }
    for (const path of group) {
      const parent = tail(parentSegments(path), Infinity).toLowerCase();
      const legacy = isLegacyName(path) && group.some(other => other !== path && !isLegacyName(other) && tail(parentSegments(other), Infinity).toLowerCase() === parent);
      qualifiers.set(pathKey(path), { folder: folders.get(parent) ?? null, legacy });
    }
  }
  return qualifiers;
}

const qualifierOf = (qualifiers: ReadonlyMap<string, RecordingQualifier>, path: string) => qualifiers.get(pathKey(path)) ?? null;

export function distinguishingFolder(qualifiers: ReadonlyMap<string, RecordingQualifier>, path: string): string | null {
  const qualifier = qualifierOf(qualifiers, path);
  return [qualifier?.folder, qualifier?.legacy && 'Older file type'].filter(Boolean).join(' · ') || null;
}

export function qualifiedRecordingName(qualifiers: ReadonlyMap<string, RecordingQualifier>, path: string): string {
  const qualifier = qualifierOf(qualifiers, path);
  return `${recordingDisplayName(path)}${qualifier?.folder ? ` in ${qualifier.folder}` : ''}${qualifier?.legacy ? ', older file type' : ''}`;
}
