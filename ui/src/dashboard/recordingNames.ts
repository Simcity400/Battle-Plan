export const recordingFilename = (path: string) => path.replaceAll('\\', '/').split('/').pop()!;

export const recordingDisplayName = (path: string) => recordingFilename(path).replace(/\.battleplan$/i, '');

export function recordingNameMatches(path: string, query: string): boolean {
  return recordingFilename(path).toLowerCase().includes(query.trim().toLowerCase());
}

const pathKey = (path: string) => path.replaceAll('\\', '/').toLowerCase();
const parentSegments = (path: string) => path.replaceAll('\\', '/').split('/').filter(Boolean).slice(0, -1);
export function distinguishingFolders(paths: readonly string[]): Map<string, string> {
  const groups = new Map<string, string[]>();
  for (const path of new Map(paths.map(path => [pathKey(path), path])).values()) {
    const name = recordingDisplayName(path).toLowerCase();
    groups.set(name, [...groups.get(name) ?? [], path]);
  }
  const qualifiers = new Map<string, string>();
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
      const folder = folders.get(parent);
      if (folder) qualifiers.set(pathKey(path), folder);
    }
  }
  return qualifiers;
}

export function distinguishingFolder(qualifiers: ReadonlyMap<string, string>, path: string): string | null {
  return qualifiers.get(pathKey(path)) ?? null;
}

export function qualifiedRecordingName(qualifiers: ReadonlyMap<string, string>, path: string): string {
  const folder = distinguishingFolder(qualifiers, path);
  const name = recordingDisplayName(path);
  return folder ? `${name} in ${folder}` : name;
}
