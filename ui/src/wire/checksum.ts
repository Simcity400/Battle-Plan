// xxHash algorithm and default secret: Copyright (C) 2012-2023 Yann Collet.
// BSD-2-Clause; see LICENSES/xxhash-BSD-2-Clause.txt.
const MASK = (1n << 64n) - 1n;
const PRIME1 = 0x9e3779b185ebca87n;
const PRIME2 = 0xc2b2ae3d27d4eb4fn;
const PRIME3 = 0x165667b19e3779f9n;
const PRIME4 = 0x85ebca77c2b2ae63n;
const PRIME5 = 0x27d4eb2f165667c5n;
const MIX1 = 0x165667919e3779f9n;
const MIX2 = 0x9fb21c651e98df25n;
const SECRET = Uint8Array.from([
  0xb8,0xfe,0x6c,0x39,0x23,0xa4,0x4b,0xbe,0x7c,0x01,0x81,0x2c,0xf7,0x21,0xad,0x1c,
  0xde,0xd4,0x6d,0xe9,0x83,0x90,0x97,0xdb,0x72,0x40,0xa4,0xa4,0xb7,0xb3,0x67,0x1f,
  0xcb,0x79,0xe6,0x4e,0xcc,0xc0,0xe5,0x78,0x82,0x5a,0xd0,0x7d,0xcc,0xff,0x72,0x21,
  0xb8,0x08,0x46,0x74,0xf7,0x43,0x24,0x8e,0xe0,0x35,0x90,0xe6,0x81,0x3a,0x26,0x4c,
  0x3c,0x28,0x52,0xbb,0x91,0xc3,0x00,0xcb,0x88,0xd0,0x65,0x8b,0x1b,0x53,0x2e,0xa3,
  0x71,0x64,0x48,0x97,0xa2,0x0d,0xf9,0x4e,0x38,0x19,0xef,0x46,0xa9,0xde,0xac,0xd8,
  0xa8,0xfa,0x76,0x3f,0xe3,0x9c,0x34,0x3f,0xf9,0xdc,0xbb,0xc7,0xc7,0x0b,0x4f,0x1d,
  0x8a,0x51,0xe0,0x4b,0xcd,0xb4,0x59,0x31,0xc8,0x9f,0x7e,0xc9,0xd9,0x78,0x73,0x64,
  0xea,0xc5,0xac,0x83,0x34,0xd3,0xeb,0xc3,0xc5,0x81,0xa0,0xff,0xfa,0x13,0x63,0xeb,
  0x17,0x0d,0xdd,0x51,0xb7,0xf0,0xda,0x49,0xd3,0x16,0x55,0x26,0x29,0xd4,0x68,0x9e,
  0x2b,0x16,0xbe,0x58,0x7d,0x47,0xa1,0xfc,0x8f,0xf8,0xb8,0xd1,0x7a,0xd0,0x31,0xce,
  0x45,0xcb,0x3a,0x8f,0x95,0x16,0x04,0x28,0xaf,0xd7,0xfb,0xca,0xbb,0x4b,0x40,0x7e,
]);
const secret = new DataView(SECRET.buffer);
const u64 = (n: bigint) => n & MASK;
function avalanche(n: bigint): bigint {
  n = u64((n ^ (n >> 37n)) * MIX1);
  return n ^ (n >> 32n);
}
function avalanche64(n: bigint): bigint {
  n = u64((n ^ (n >> 33n)) * PRIME2);
  n = u64((n ^ (n >> 29n)) * PRIME3);
  return n ^ (n >> 32n);
}
function folded(a: bigint, b: bigint): bigint {
  const product = a * b;
  return u64(product) ^ (product >> 64n);
}
function rotate(n: bigint, shift: bigint): bigint {
  return u64(n << shift) | (n >> (64n - shift));
}
function swap(n: bigint): bigint {
  let result = 0n;
  for (let i = 0; i < 8; ++i) { result = (result << 8n) | (n & 255n); n >>= 8n; }
  return result;
}

/** Default, seedless xxh3-64. Subarray offsets are respected without copying. */
export function xxh3(bytes: Uint8Array): bigint {
  const input = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const length = bytes.length;
  const read = (offset: number) => input.getBigUint64(offset, true);
  const key = (offset: number) => secret.getBigUint64(offset, true);
  const mix = (offset: number, secretOffset: number) =>
    folded(read(offset) ^ key(secretOffset), read(offset + 8) ^ key(secretOffset + 8));
  if (length <= 16) {
    if (length > 8) {
      const low = read(0) ^ key(24) ^ key(32);
      const high = read(length - 8) ^ key(40) ^ key(48);
      return avalanche(u64(BigInt(length) + swap(low) + high + folded(low, high)));
    }
    if (length >= 4) {
      let n = (BigInt(input.getUint32(0, true)) << 32n) | BigInt(input.getUint32(length - 4, true));
      n ^= key(8) ^ key(16);
      n = u64((n ^ rotate(n, 49n) ^ rotate(n, 24n)) * MIX2);
      n = u64((n ^ ((n >> 35n) + BigInt(length))) * MIX2);
      return n ^ (n >> 28n);
    }
    if (length !== 0) {
      const combined = ((bytes[0] << 16) | (bytes[length >> 1] << 24) | bytes[length - 1] | (length << 8)) >>> 0;
      const flip = (secret.getUint32(0, true) ^ secret.getUint32(4, true)) >>> 0;
      return avalanche64(BigInt(combined) ^ BigInt(flip));
    }
    return avalanche64(key(56) ^ key(64));
  }
  let acc = u64(BigInt(length) * PRIME1);
  if (length <= 128) {
    for (let i = Math.floor((length - 1) / 32); i >= 0; --i) {
      acc = u64(acc + mix(16 * i, 32 * i) + mix(length - 16 * (i + 1), 32 * i + 16));
    }
    return avalanche(acc);
  }
  if (length <= 240) {
    for (let i = 0; i < 8; ++i) acc = u64(acc + mix(i * 16, i * 16));
    acc = avalanche(acc);
    let tail = mix(length - 16, 119);
    for (let i = 8; i < Math.floor(length / 16); ++i) tail = u64(tail + mix(i * 16, (i - 8) * 16 + 3));
    return avalanche(u64(acc + tail));
  }
  // Long inputs use exact pairs of 32-bit limbs. No BigInts or copied input in the stripe loop.
  const initial = [0xc2b2ae3dn, PRIME1, PRIME2, PRIME3, PRIME4, 0x85ebca77n, PRIME5, 0x9e3779b1n];
  const lo = Uint32Array.from(initial, n => Number(n & 0xffffffffn));
  const hi = Uint32Array.from(initial, n => Number(n >> 32n));
  const add = (lane: number, low: number, high: number) => {
    const sum = lo[lane] + low;
    lo[lane] = sum;
    hi[lane] = hi[lane] + high + (sum >= 0x100000000 ? 1 : 0);
  };
  const multiply = (lane: number, a: number, b: number, replace = false, extraHigh = 0): void => {
    const al = a & 65535, ah = a >>> 16, bl = b & 65535, bh = b >>> 16;
    const lowProduct = al * bl;
    const middle = ah * bl + al * bh + (lowProduct >>> 16);
    const low = ((middle & 65535) * 65536 + (lowProduct & 65535)) >>> 0;
    const high = (ah * bh + Math.floor(middle / 65536)) >>> 0;
    if (replace) { lo[lane] = low; hi[lane] = high + extraHigh; }
    else add(lane, low, high);
  };
  const stripe = (offset: number, secretOffset: number) => {
    for (let lane = 0; lane < 8; ++lane) {
      const low = input.getUint32(offset + lane * 8, true);
      const high = input.getUint32(offset + lane * 8 + 4, true);
      add(lane ^ 1, low, high);
      const a = (low ^ secret.getUint32(secretOffset + lane * 8, true)) >>> 0;
      const b = (high ^ secret.getUint32(secretOffset + lane * 8 + 4, true)) >>> 0;
      multiply(lane, a, b);
    }
  };
  const blocks = Math.floor((length - 1) / 1024);
  for (let block = 0; block < blocks; ++block) {
    for (let i = 0; i < 16; ++i) stripe(block * 1024 + i * 64, i * 8);
    for (let lane = 0; lane < 8; ++lane) {
      const low = (lo[lane] ^ (hi[lane] >>> 15) ^ secret.getUint32(128 + lane * 8, true)) >>> 0;
      const high = (hi[lane] ^ secret.getUint32(132 + lane * 8, true)) >>> 0;
      multiply(lane, low, 0x9e3779b1, true, Math.imul(high, 0x9e3779b1));
    }
  }
  const remaining = Math.floor(((length - 1) - blocks * 1024) / 64);
  for (let i = 0; i < remaining; ++i) stripe(blocks * 1024 + i * 64, i * 8);
  stripe(length - 64, 121);
  for (let i = 0; i < 4; ++i) {
    const left = (BigInt(hi[i * 2]) << 32n) | BigInt(lo[i * 2]);
    const right = (BigInt(hi[i * 2 + 1]) << 32n) | BigInt(lo[i * 2 + 1]);
    acc = u64(acc + folded(left ^ key(11 + i * 16), right ^ key(19 + i * 16)));
  }
  return avalanche(acc);
}
