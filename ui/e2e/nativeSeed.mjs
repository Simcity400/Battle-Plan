import { cp, lstat, realpath } from 'node:fs/promises';
import path from 'node:path';

/** Copy explicitly supplied prepared pages into this adapter's private cache.
 * Source stamps describe the copy; the bridge still decides whether the actual
 * recording/content inputs and domain schemas permit reuse. */
export async function copyPreparedSeed(directory, destination) {
  if (!directory) return null;
  const source = path.resolve(directory), target = path.resolve(destination);
  const root = await lstat(source);
  if (root.isSymbolicLink() || !root.isDirectory()) throw Error(`Prepared seed must be a real directory: ${source}`);
  const relative = path.relative(source, target);
  if (!relative || (!relative.startsWith('..') && !path.isAbsolute(relative))) throw Error('Prepared seed cannot contain its copy destination');
  const files = [];
  let bytes = 0n;
  await cp(source, target, { recursive: true, preserveTimestamps: true, errorOnExist: true, force: false,
    filter: async input => {
      const stamp = await lstat(input, { bigint: true });
      if (stamp.isSymbolicLink()) throw Error(`Linked prepared input refused: ${input}`);
      if (stamp.isFile()) {
        bytes += stamp.size;
        files.push({ path: path.relative(source, input).replaceAll('\\', '/'),
          bytes: stamp.size.toString(), modifiedNs: stamp.mtimeNs.toString() });
      } else if (!stamp.isDirectory()) throw Error(`Unsupported prepared input: ${input}`);
      return true;
    },
  });
  for (const file of files) {
    const after = await lstat(path.join(source, file.path), { bigint: true });
    if (!after.isFile() || after.size.toString() !== file.bytes || after.mtimeNs.toString() !== file.modifiedNs)
      throw Error(`Prepared input changed during copy: ${file.path}`);
  }
  files.sort((left, right) => left.path < right.path ? -1 : left.path > right.path ? 1 : 0);
  return { directory: await realpath(source), files, bytes: bytes.toString(),
    reuse: 'Bridge validates recording/content stamps and prepared schemas on open; copied files alone establish no readiness or parity.' };
}
