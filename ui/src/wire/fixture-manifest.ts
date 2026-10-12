import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import type { Bounds, ShapeContract } from './runtime.ts';

export interface Fixture {
  name: string; file: string; shapeId: number; expectedError?: string | null;
  bounds?: Bounds; benchmark?: boolean;
  assertions?: { column: string; index: number; value?: string | number; state?: (string | null)[] }[];
  header?: { unitStart?: string; unitEnd?: string; rowCount?: string; totalCount?: string };
  bufferHex?: string[];
  expected?: {
    header: Record<string, string | number | number[]>;
    buffers: string[];
    descriptors: {data_buffer:number;aux_buffer:number;state_buffer:number;state_layers:number}[];
    embeddedBuffers?: string[] | null;
    occurrences?: {column:number;index:string;path:{column:number;index:string}[]}[];
  } | null;
}
interface RawFixture extends Omit<Fixture, 'bounds'> {
  bounds?: {kind:number;start:string;end:string} | null;
  expectedBounds?: {kind:number;start:string;end:string} | null;
}
export interface DictionaryFixture {
  name:string;file:string;identity:string;expectedError?:string|null;
  expectedEntries:string[];
  byteExtents:{code:string;first:string;end:string}[];
}

/** Manifest and contracts are both emitted by Rust, with wide integers as strings. */
export function loadFixtureManifest(folder: string): {
  fixtures: Fixture[]; shapes: Map<number,ShapeContract>; dictionaryFixtures:DictionaryFixture[];
} {
  const manifest = JSON.parse(readFileSync(resolve(folder,'manifest.json'),'utf8'));
  const rawShapes = manifest.shapes ?? JSON.parse(readFileSync(resolve(folder,'contracts.json'),'utf8'));
  const shapes = new Map<number,ShapeContract>();
  for (const raw of rawShapes) {
    const shape: ShapeContract = {...raw,columns:raw.columns.map((c: Record<string,unknown>) => {
      const d = c.descriptor as Record<string,unknown>;
      return {...c,descriptor:{...d,length:BigInt(d.length as string)},
        staticEnum:c.staticEnum ?? undefined,dictionaryKind:c.dictionaryKind ?? undefined,
        fixedListLength:c.fixedListLength == null ? undefined : BigInt(c.fixedListLength as string)};
    })};
    if (shapes.has(shape.id)) throw new Error('duplicate fixture shape');
    shapes.set(shape.id,shape);
  }
  const fixtures: Fixture[] = manifest.fixtures.map((raw: RawFixture) => {
    const bounds = raw.bounds ?? raw.expectedBounds;
    return {...raw,bounds:bounds ? {kind:bounds.kind,start:BigInt(bounds.start),end:BigInt(bounds.end)} : undefined};
  });
  if (fixtures.length === 0) throw new Error('empty Rust fixture manifest');
  for (const fixture of fixtures) {
    if (!fixture.file || !shapes.has(fixture.shapeId)) throw new Error('fixture missing file or shape');
  }
  return {fixtures,shapes,dictionaryFixtures:manifest.dictionaryFixtures ?? []};
}
