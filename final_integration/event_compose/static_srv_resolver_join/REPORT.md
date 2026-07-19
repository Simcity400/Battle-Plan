# Static SRV resolver join — frozen promotion evidence

Status: **PASS — production adapter ready to promote**

The production `frame_bundle` loader now admits the eight legacy descriptor-only terrain SRVs only when one resolver source uniquely matches the captured resource and SRV descriptors. Admission checks stage, slot/name identity, dimensions, DXGI encoding family, complete mip count, array/sample shape, bind flags, and SRV view extent. Zero matches and multiple exact matches fail closed. Existing terrain-chunk fallbacks are likewise descriptor-matched and ambiguity-rejected.

| SRV | Exact resolver source |
|---|---|
| TerrainDiffuse | `map/terrain/atlas0.dds` |
| HeightNormal | `map/world_normal.bmp` |
| TerrainColorTint | `map/terrain/colormap_rgb_cityemissivemask_a.dds` |
| SnowTexture | `map/terrain/snow_normal_rgb_diffuse_a.dds` |
| TerrainNormal | `map/terrain/atlas_normal0.dds` |
| CityLightsAndSnowNoise | uniquely matching `map/terrain/citylights_rgb_snowmask_a_0.dds` |
| MudNormalSpec | uniquely matching `map/terrain/mud_normal_rgb_spec_a_0.dds` |
| MudDiffuseGloss | uniquely matching `map/terrain/mud_diffuse_rgb_gloss_a_0.dds` |

DDS payloads are reconstructed from source rows using the retained capture's exact 128-byte-minimum staging RowPitch and per-mip subresource layout. PAN2 byte comparison proves full-file equality for TerrainDiffuse, TerrainNormal, SnowTexture, and CityLightsAndSnowNoise. MudNormalSpec and MudDiffuseGloss prove 100% equality for every meaningful BC3 row; retained mip 7–10 padding contains nonzero process residue, so production zero-fills only upload-ignored padding instead of reproducing nondeterministic bytes. HeightNormal uses the existing top-down 24-bit BMP to B8G8R8X8 conversion; TerrainColorTint uses the exact 32-bit DDS body.

Mechanical gates:

- `cargo check --manifest-path replay_interface/native/map_viewer/Cargo.toml`: PASS.
- `cargo test --manifest-path replay_interface/native/map_viewer/Cargo.toml descriptor_only_ -- --nocapture`: PASS, 4/4.
- trees legacy oracle: `PASS_EXACT_SOURCE_AND_CAPTURE_ORACLE_FROZEN`, 14 checks, 56 joins, two negative controls; manifest SHA-256 `875047b57fb58e8a9faa4d8b62e21d29f04ab11be2fb6f0372e181c9b852af01`.
- FoW-on complete water → rivers → trees graph: PASS, final hash `5a79c3503df3a519`.

The incomplete terrain-only FoW-off hash comparison (`866ceb9c97c5cab0`) is explicitly non-authoritative because it omitted the dependent graph. The canonical FoW-off and trees event runs remain owned by `event_compose`; their current preserved sessions stop at missing upstream water/rivers dependency gates, after this adapter has already removed the former raw-payload blocker.

Incidental retained finding: the preserved compressed mud payload hashes include nondeterministic mapped-row padding and therefore must not be used as source-content identities. Meaningful compressed row bytes and descriptor/subresource layout are the stable oracle.

No `PROGRAM_STATUS.md` edit was made by this lane.
