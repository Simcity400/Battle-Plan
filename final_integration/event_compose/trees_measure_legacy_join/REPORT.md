# `live_trees_measure` legacy static-SRV join oracle

This read-only closure packet freezes the exact producer contract that the shared
`frame_bundle` implementation must satisfy. It does not modify production code,
the preserved captures, or `PROGRAM_STATUS.md`.

The legacy terrain bundle contains seven draws and eight descriptor-only static
terrain SRVs per draw: 56 exact joins. Every descriptor is field-identical to
the corresponding raw resource in the retained `live_PAN2` terrain chunk
library. The game-file sources are uniquely selected by symbol, dimensions,
DXGI format, and mip count. In particular, the three season families select the
`_0.dds` source; `_1`/`_2` do not carry the captured full mip chain.

| Symbol | Slot | Descriptor | Exact source | Source SHA-256 | Normalized payload SHA-256 |
|---|---:|---|---|---|---|
| TerrainDiffuse | 0 | 2048x2048, DXGI 78, 10 mips | `map/terrain/atlas0.dds` | `133b3bd652f4175cdb83d33858ead8690027ddccc9b87cd9f21f481d7ad827bd` | `32534d5d4877d984a18a404d4eafd2b285a4c83fc3bfd8b629cadbab6e7987b1` |
| HeightNormal | 1 | 2816x1024, DXGI 88, 1 mip | `map/world_normal.bmp` | `0a050aa615dcf087ddf127b0d3ffdb2cbc24921a1c503405edb1a2aa141b5b87` | `00b4175ee12f6401f91de58697201065531c184e8cb308d4582d2a439bf4ffb7` |
| TerrainColorTint | 2 | 2816x1024, DXGI 91, 1 mip | `map/terrain/colormap_rgb_cityemissivemask_a.dds` | `94060e50e9d7de5fb7c69f7c1de35b7fd00efbb4f7321cd35f35d3150de988db` | `e30c4093d655ac4842275d637f262b73ef7138225be872679373e17f8fb0b88e` |
| SnowTexture | 3 | 512x512, DXGI 77, 10 mips | `map/terrain/snow_normal_rgb_diffuse_a.dds` | `353c56e7c67fc2c22828769233177e63838e4af0091d11eb80a0c7ed9b51af98` | `11021d5898f6eb1627c2a2deea6abfe0160db9800647cff5520ba9afdd4f937b` |
| TerrainNormal | 4 | 2048x2048, DXGI 77, 10 mips | `map/terrain/atlas_normal0.dds` | `c1ac9e7accd6b02fd5687d4ff50ab9b4bdb5a12523d410993be70b80c5f009a5` | `8d302f19972ab82fdc515d69abb7f55ad7e53fa0ee39554f350efbb59c7f2df0` |
| CityLightsAndSnowNoise | 8 | 512x512, DXGI 78, 10 mips | `map/terrain/citylights_rgb_snowmask_a_0.dds` | `a5d1048a1f40be399c30568e875f9897ec85753521427718b9de2f6986f5b58f` | `42f0c147d86ade77f21483d9feb9134e5648120238faac6e7400ee52ee7d6d9c` |
| MudNormalSpec | 9 | 1024x1024, DXGI 77, 11 mips | `map/terrain/mud_normal_rgb_spec_a_0.dds` | `92cd52e71b35c4ffce2f0712efddcc3bfca3579e513cffb87dc01990bb81b4ea` | `cb67dc9ca0ccc2d7fdf44c890a0f5671cd2299ec1ad3a058d2aa630ba03f27d9` |
| MudDiffuseGloss | 13 | 1024x1024, DXGI 78, 11 mips | `map/terrain/mud_diffuse_rgb_gloss_a_0.dds` | `ecf74eeccdc3528c4dc2484f35a8bbd2c36ceb697a79902d6ccef740d82cd0ab` | `b8efaf6a0faf7a0a436899461a38bccea86404fab01524e546cbb20c1f2d36e5` |

For BC3 sources, each compact DDS row is copied byte-exactly into the captured
mapped-subresource layout: `row_pitch=max(128,ceil(width/4)*16)`, with
`ceil(height/4)` block rows. New row padding must be zeroed. PAN2's two mud
payloads contain nonzero bytes only in ignored row padding at the smallest
mips; all sampled compact BC3 bytes are byte-identical to the `_0.dds` sources.
Copying those incidental padding bytes would make capture memory, rather than
the installed content, production authority.

The shared implementation must fail closed on unknown symbols, any descriptor
or source hash drift, missing/truncated inputs, DDS format/mip mismatch, and
normalized payload hash mismatch. Positive acceptance requires all 56 joins.
The focused verifier also carries explicit descriptor-drift and source-byte
corruption negative controls.

No Ghidra operation was used. Function address: none; job ID: none; artifact
path: none; manifest hash: none.
