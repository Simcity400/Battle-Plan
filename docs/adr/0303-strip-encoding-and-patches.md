# 0303. Serve exact dashboard answers as reversible strips

The `strip` answer product asks one product at 1 to 17 parameter objects, in the supplied hour order. Each ask goes through `answer::answer`, so registry and series products retain their ordinary meanings, warnings and missing states. A strip cannot wrap `strip`, `follow`, `unfollow`, `follow-clock` or `follow-close`: the latter four are commands that change subscriptions or the playhead. These named refusals happen before any inner ask. Other failures abort the group; cancellation is checked before and after every ask.

The product uses the non-interactive Replies workers. At-hour reads select a pool scoped to this call through `Pool::of`. It shares immutable recording pages, but has its own four retained lane sets and warming state. Synchronous inner reads reuse it; current product work on other threads reads through separate replay readers, never `Pool::of`. No strip can install a panel. Returning, failing or unwinding drops the scope and its retained lanes; the interactive pool remains as it was.

## Request and reply

The request is `{ product: string, asks: object[], whole: boolean }`. The caller supplies the asks in hour order; the wrapper does not interpret the inner product's parameters. The reply is `{ data: string }`, whose only field is standard padded base64. Decoding it yields this versioned binary stream. Multi-byte framing integers are unsigned little-endian; MessagePack uses its own byte order.

| Byte | Meaning |
|---|---|
| 0–3 | ASCII `BPST` |
| 4 | Version, `1` |
| 5 | Flags: bit 0 means the first state is included whole; other bits are zero |
| 6 | Number of asks, 1 to 17, including the first even when omitted |
| 7 | Reserved, zero |

After the eight-byte header, every part has a four-byte byte length followed by exactly that many MessagePack bytes. There is no padding. When flag bit 0 is set, the first part is state 0. For each index `i` from 1 to count minus one, the next two parts are the forward patch from state `i-1` to state `i`, then the backward patch from `i` to `i-1`. No parts follow the last pair. With `whole: false`, only state 0's part is omitted: the caller must already hold it. A one-ask reply without a whole is just the header. Unknown versions, flags, nonzero reserved bytes, invalid counts, truncated parts and trailing bytes are invalid.

Each state is a one-key map: `{ "answer": <the exact JSON reply> }` or `{ "refused": <the named refusal text> }`. Refusals returned through `QueryError::Refused` carry the same text. Missing fields and not-recorded states inside a successful reply remain inside `answer`, unchanged. Failures that are not named refusals are errors of the strip request, never fabricated keyframes.

## Patches

A patch is a MessagePack array of operations, applied in order. Every operation is a one-key map with one of these shapes:

| Operation | Meaning |
|---|---|
| `{ "replace": [path, value] }` | Replace the target, or insert an object member at the last path part. An empty path replaces the whole state. |
| `{ "remove": path }` | Remove an existing object member. The path is nonempty and ends in a string key. |
| `{ "splice": [path, index, removed, inserted] }` | At the target array, remove `removed` elements starting at `index`, then insert the `inserted` array there. |

A path is an array of string object keys and nonnegative integer array indices. Strings are literal: `"0"` addresses an object key, while `0` addresses an array element. Empty strings, slashes and tildes need no escaping. Every intermediate target must exist and match its path part's type. Replace cannot append an array element; splice may insert at its length. A splice's removed range must be within the target array. Patch values use only JSON-compatible MessagePack: null, booleans, strings, numbers, arrays and maps with string keys; there are no extension or binary values.

The differ removes absent object keys, then visits the destination object's keys. Arrays retain the longest equal prefix and non-overlapping equal suffix. When each middle contains only objects with unique string `key` members, the differ retains a longest order-preserving subsequence of shared keys and patches each retained row at its destination index. Splices remove the other rows and insert their full destination values. Splice indices refer to the array after the preceding operations.

Other middles use a longest common subsequence of equal values, found by hash and confirmed by value, with a 262,144-cell limit. Above that limit the entire middle becomes one hunk. Unequal-count hunks use one splice, including when one side is empty. For equal-count hunks, each pair uses the nested patch when it encodes smaller than a whole replacement. All other unequal values are replaced, and equal values produce an empty patch. Each direction uses its own diff. Map member order has no meaning. A decoder must retain integer precision while applying patches.

The existing product reply budget covers the base64 JSON reply. The encoder also bounds the binary stream before appending parts, accounting for base64 expansion, and returns the existing named size refusal. The shared fixture stores the encoder's raw bytes as integer arrays ready for `Uint8Array.from`. There is no recording-format change, new recorded fact, cache or TypeScript decoder in this change.

[Patch codec](../../crates/dashboard/src/strip/patch.rs) · [Strip answer](../../crates/dashboard/src/strip.rs) · [Shared bytes fixture](../../ui/src/dashboard/strip/messages.fixture.json)
