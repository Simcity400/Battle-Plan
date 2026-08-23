use crate::{
    BufferSource, DrawCommand, IndexFormat, RenderExecutionRef, RenderSubmit, S1DrawArguments,
    S1LightDepthSubmission, S1ShadowCarrier, S1ShadowPipeline, StaticGeometrySource,
    StaticMapCommand, StaticMapDraw, StaticMapOperation, StaticShaderResourceSource,
    S1_PARTITION_PRIMARY,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::OnceLock,
};

pub const C07_TREE_RUNTIME_SCHEMA: &str = "hoi4.c07-tree-runtime.a729.v2";
pub const C07_TREE_A729_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const C07_TREE_RETAINED_CONTRACT_SHA256: &str =
    "05c6219af1a5d7b0b24af82cded9b1a1c45955fba8098c6fa1193ea4496a405d";
pub const C07_TREE_RETAINED_RDC_SHA256: &str =
    "dec14f4b77f86f72f613a88f2cc5a129eb50a62b3b4c05f44840945500e921f1";
pub const C07_TREE_PRODUCER_FUNCTION_RVA: u32 = 0x01b1_1a70;
pub const C07_TREE_BUFFER_FACTORY_CALLSITE_RVA: u32 = 0x01b1_21e9;
pub const C07_TREE_INSTANCE_STRIDE_BYTES: u32 = 24;
pub const C07_TREE_GLOBAL_NONZERO_CELLS: u32 = 99_056;
pub const C07_TREE_RETAINED_VISIBLE_CELLS: u32 = 4_781;
pub const C07_TREE_UNCAPTURED_CELLS: u32 =
    C07_TREE_GLOBAL_NONZERO_CELLS - C07_TREE_RETAINED_VISIBLE_CELLS;
pub const C07_TREE_RETAINED_BUCKETS: usize = 13;
pub const C07_TREE_RETAINED_PASSES: usize = 3;
pub const C07_TREE_CONTENT_LIBRARY_SCHEMA: &str = "hoi4.c07-tree-content-library.a729.v1";
pub const C07_TREE_CONTENT_SELECTION_SCHEMA: &str = "hoi4.c07-tree-content-selection.a729.v1";
pub const C07_TREE_CONTENT_LIBRARY_GAME_FILE_SET_SHA256: &str =
    "42ae577a6a5765bec34de951e1f6dd8cabaf3ab60f8d7cb8cfb6068f82ef62b3";
pub const C07_TREE_CONTENT_LIBRARY_CENSUS_SHA256: &str =
    "d9cd200c9414bedf46af5a43fddf5c885324b47560feb15487d21fedd66bc5b2";
pub const C07_TREE_CONTENT_LIBRARY_COVERED_CELL_SET_SHA256: &str =
    "d4963414e4095e2894f0c37bb9ef782b574321358e4319ee9ba579ee353f8a3f";
pub const C07_TREE_CONTENT_LIBRARY_MISSING_CELL_SET_SHA256: &str =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
// Provenance for the two literals in the generated file included below.
// whole file. This note spent 0e0ad998..9178a5c3 inside the generated file,
/// This literal is generated from the exact producer and hash-pinned shipped
include!("c07_tree_content_hashes.rs");
pub const C07_TREE_CONTENT_LIBRARY_BUCKETS: usize = 296;
pub const C07_TREE_CONTENT_LIBRARY_CELLS: u32 = 99_056;
pub const C07_TREE_CONTENT_LIBRARY_INSTANCE_BYTES: u32 =
    C07_TREE_CONTENT_LIBRARY_CELLS * C07_TREE_INSTANCE_STRIDE_BYTES;
pub const C07_TREE_CONTENT_LIBRARY_UNAVAILABLE_BUCKETS: u32 = 0;
pub const C07_TREE_CONTENT_LIBRARY_UNAVAILABLE_CELLS: u32 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum C07TreeCoverageKind {
    RetainedVisibleSubset,
    CompleteStaticPool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum C07TreeCoverageRequirement {
    RetainedVisibleSubset,
    CompleteStaticPool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct C07TreeCoverage {
    pub kind: C07TreeCoverageKind,
    pub global_nonzero_cells: u32,
    pub captured_nonzero_cells: u32,
    pub uncaptured_nonzero_cells: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct C07TreeBucketCarrier {
    pub bucket_id: u16,
    pub palette_index: u8,
    pub chunk_x: u8,
    pub chunk_z: u8,
    pub producer_create_ordinal: u16,
    pub record_count: u32,
    pub instance_bytes: u32,
    pub instance_payload_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct C07TreeImmutableBucketContent {
    pub bucket_id: u16,
    pub palette_index: u8,
    pub chunk_x: u8,
    pub chunk_z: u8,
    pub global_producer_create_ordinal: u16,
    pub packed_byte_offset: u32,
    pub record_count: u32,
    pub instance_bytes: u32,
    pub instance_payload_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct C07TreeCellOrdinalRange {
    pub first_ordinal: u32,
    pub last_ordinal: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct C07TreeContentLibrary {
    pub schema: String,
    pub executable_sha256: String,
    pub game_file_set_sha256: String,
    pub census_payload_sha256: String,
    pub covered_cell_set_sha256: String,
    pub missing_cell_set_sha256: String,
    pub instance_stride_bytes: u32,
    pub global_nonzero_cells: u32,
    pub global_nonempty_buckets: u32,
    pub authenticated_cells: u32,
    pub authenticated_buckets: u32,
    pub unavailable_cells: u32,
    pub unavailable_buckets: u32,
    pub ordered_buckets: Vec<C07TreeImmutableBucketContent>,
    pub authenticated_cell_ordinal_ranges: Vec<C07TreeCellOrdinalRange>,
    #[serde(with = "crate::base64_bytes")]
    pub packed_instance_bytes: Vec<u8>,
    pub payload_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct C07TreeContentSelection {
    pub schema: String,
    pub library_payload_sha256: String,
    pub selected_submit_tree_runtime_payload_sha256: String,
    pub ordered_bucket_ids: Vec<u16>,
    pub selected_cells: u32,
    pub selected_instance_bytes: u32,
    pub unselected_library_cells: u32,
    pub globally_unavailable_cells: u32,
    pub payload_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum C07TreePassKind {
    Shadow,
    Unlit,
    Main,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct C07TreeDrawRef {
    pub operation_ordinal: u32,
    pub bucket_id: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct C07TreePassCarrier {
    pub pass: C07TreePassKind,
    pub vertex_shader_sha256: String,
    pub pixel_shader_sha256: String,
    pub ordered_draws: Vec<C07TreeDrawRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct C07TreeRuntimeCarrier {
    pub schema: String,
    pub executable_sha256: String,
    pub retained_contract_sha256: String,
    pub retained_rdc_sha256: String,
    pub producer_function_rva: u32,
    pub buffer_factory_callsite_rva: u32,
    pub coverage: C07TreeCoverage,
    pub ordered_buckets: Vec<C07TreeBucketCarrier>,
    pub ordered_passes: Vec<C07TreePassCarrier>,
    pub payload_sha256: String,
}

#[derive(Clone, Copy)]
struct ExpectedBucket {
    bucket_id: u16,
    palette: u8,
    chunk_x: u8,
    chunk_z: u8,
    records: u32,
    bytes: u32,
    instance_sha256: &'static str,
}

#[derive(Clone, Copy)]
struct ExpectedLibraryBucket {
    bucket_id: u16,
    palette: u8,
    chunk_x: u8,
    chunk_z: u8,
    global_ordinal: u16,
    records: u32,
    bytes: u32,
    instance_sha256: &'static str,
}

#[derive(Clone, Copy)]
struct ExpectedDraw {
    bucket_id: u16,
    index_count: u32,
    instance_count: u32,
    mesh_vb_bytes: u32,
    mesh_vb_sha256: &'static str,
    mesh_ib_bytes: u32,
    mesh_ib_sha256: &'static str,
}

#[derive(Clone, Copy)]
struct ExpectedS1TreeAsset {
    selected_lod_index: u32,
    selected_mesh: &'static str,
    selected_material: &'static str,
    material_textures: &'static [(&'static str, &'static str); 3],
}

const PALMS_BLEND_MESH: &str = "gfx/models/mapitems/trees/palms_blend.mesh";
const FOREST_DUNGE_BLEND_MESH: &str = "gfx/models/mapitems/trees/forest_dunge_blend.mesh";
const FOREST_DUNGE_MESH: &str = "gfx/models/mapitems/trees/forest_dunge.mesh";
const PALMS_BLEND_LOD1_MATERIAL: &str =
    "lodettShape|PdxMeshAdvancedSnow|palmblad_diffuse.dds|palmblad_normal.dds|palmblad_specular.dds";
const FOREST_DUNGE_BLEND_LOD1_MATERIAL: &str =
    "polySurfaceShape874|PdxMeshAdvancedSnow|pinetree_diffuse.dds|pinetree_normal.dds|nospec.dds";
const FOREST_DUNGE_BLEND_LOD2_MATERIAL: &str =
    "polySurfaceShape875|PdxMeshAdvancedSnow|pinetree_diffuse.dds|pinetree_normal.dds|nospec.dds";
const FOREST_DUNGE_LOD1_MATERIAL: &str =
    "polySurfaceShape874|PdxMeshAdvancedSnow|pinetree_diffuse.dds|pinetree_normal.dds|nospec.dds";
const FOREST_DUNGE_LOD2_MATERIAL: &str =
    "polySurfaceShape875|PdxMeshAdvancedSnow|pinetree_diffuse.dds|pinetree_normal.dds|nospec.dds";
const PALM_MATERIAL_TEXTURES: [(&str, &str); 3] = [
    (
        "gfx/models/mapitems/trees/palmblad_diffuse.dds",
        "01f93979fbd087a230bc3cb7c26c83ced43938730ba2e3bef9cb9f4656297293",
    ),
    (
        "gfx/models/mapitems/trees/palmblad_normal.dds",
        "101e9e9d16df5886c534b2ebc557b209dd85a7533721bdc5da4e165c247039ac",
    ),
    (
        "gfx/models/mapitems/trees/palmblad_specular.dds",
        "14f89b99427a6327b4e47d1806e9588c6a06128cf07cb95fef01bf8d8b20ec63",
    ),
];
const FOREST_MATERIAL_TEXTURES: [(&str, &str); 3] = [
    (
        "gfx/models/mapitems/trees/pinetree_diffuse.dds",
        "706d3c27456d2cd06689ae410c3f7776a883b5ab2792529bf90f789614b31088",
    ),
    (
        "gfx/models/mapitems/trees/pinetree_normal.dds",
        "2465ef94e116514590a66efb1ab259f34e2e6ea64e0c2f28d9976043a9d56e7e",
    ),
    (
        "gfx/models/nospec.dds",
        "d1cfb5c6ed8e1fb23919710dbf504a535e00c63e75c55da0bd0725560e6fb354",
    ),
];

#[allow(dead_code)]
const RETAINED_EXPECTED_LIBRARY_BUCKETS: [ExpectedLibraryBucket; 36] = [
    ExpectedLibraryBucket {
        bucket_id: 354,
        palette: 2,
        chunk_x: 10,
        chunk_z: 4,
        global_ordinal: 28,
        records: 74,
        bytes: 1_776,
        instance_sha256: "85016a70948bef065f531865cc31861c3fbad3b2d2432bff21078ca17c15920c",
    },
    ExpectedLibraryBucket {
        bucket_id: 355,
        palette: 2,
        chunk_x: 11,
        chunk_z: 4,
        global_ordinal: 29,
        records: 8,
        bytes: 192,
        instance_sha256: "9bd7ed3ccfac320d6e5cd578d3a2c3adfb00e33bd30b49ce8d023b93e9082b70",
    },
    ExpectedLibraryBucket {
        bucket_id: 356,
        palette: 2,
        chunk_x: 12,
        chunk_z: 4,
        global_ordinal: 30,
        records: 53,
        bytes: 1_272,
        instance_sha256: "b6787ed08caafc1687561832b5f95ebbe4498d8ddf55f3a8996e557554c101f0",
    },
    ExpectedLibraryBucket {
        bucket_id: 379,
        palette: 2,
        chunk_x: 13,
        chunk_z: 5,
        global_ordinal: 36,
        records: 5,
        bytes: 120,
        instance_sha256: "898f804f92ee8ff565700e123238a241684496a9d8f935bdc36ea7ecdf44ecaa",
    },
    ExpectedLibraryBucket {
        bucket_id: 380,
        palette: 2,
        chunk_x: 14,
        chunk_z: 5,
        global_ordinal: 37,
        records: 26,
        bytes: 624,
        instance_sha256: "ba1c32c7de0be0c7960418849bd3de0c9c6b37fd774b8a07f59bec541eafe6ab",
    },
    ExpectedLibraryBucket {
        bucket_id: 610,
        palette: 3,
        chunk_x: 10,
        chunk_z: 4,
        global_ordinal: 67,
        records: 1,
        bytes: 24,
        instance_sha256: "12d729eeb5f0189fffd75b1da6db3a070bb11bb19c85211ba8901b7395bec21a",
    },
    ExpectedLibraryBucket {
        bucket_id: 611,
        palette: 3,
        chunk_x: 11,
        chunk_z: 4,
        global_ordinal: 68,
        records: 70,
        bytes: 1_680,
        instance_sha256: "dfa92f3a28d41aee78dc9350d4c59e84853b1d6712d2cb19e93458391f9ed6ae",
    },
    ExpectedLibraryBucket {
        bucket_id: 612,
        palette: 3,
        chunk_x: 12,
        chunk_z: 4,
        global_ordinal: 69,
        records: 13,
        bytes: 312,
        instance_sha256: "695b11975a51c7c53872bed7b5c053f4dab62a36604a3ec8efcfd56d08a21f3c",
    },
    ExpectedLibraryBucket {
        bucket_id: 634,
        palette: 3,
        chunk_x: 12,
        chunk_z: 5,
        global_ordinal: 76,
        records: 2,
        bytes: 48,
        instance_sha256: "0621eb7cb25f2bae50be9b003169d48198c292b36307102c5f36cc7fb3112e07",
    },
    ExpectedLibraryBucket {
        bucket_id: 635,
        palette: 3,
        chunk_x: 13,
        chunk_z: 5,
        global_ordinal: 77,
        records: 30,
        bytes: 720,
        instance_sha256: "cbc214b16d36004151cb2c7be5e3b096cfcdc55ac70576803795158bdf929d9c",
    },
    ExpectedLibraryBucket {
        bucket_id: 636,
        palette: 3,
        chunk_x: 14,
        chunk_z: 5,
        global_ordinal: 78,
        records: 2,
        bytes: 48,
        instance_sha256: "5e46d164118e127abddd3e770a9aa748602a000acfda450def4f96f2ba63f2e0",
    },
    ExpectedLibraryBucket {
        bucket_id: 1144,
        palette: 5,
        chunk_x: 10,
        chunk_z: 5,
        global_ordinal: 105,
        records: 86,
        bytes: 2_064,
        instance_sha256: "acb488ccbd70a1550111c121681f5c2abfc295599f82552dfb5c2b8d40cd3ed2",
    },
    ExpectedLibraryBucket {
        bucket_id: 1145,
        palette: 5,
        chunk_x: 11,
        chunk_z: 5,
        global_ordinal: 106,
        records: 296,
        bytes: 7_104,
        instance_sha256: "dca20484fbf4397f4150b7d01d4a40030d948c07bdfa2685d8bea4fd8e9ebae7",
    },
    ExpectedLibraryBucket {
        bucket_id: 1146,
        palette: 5,
        chunk_x: 12,
        chunk_z: 5,
        global_ordinal: 107,
        records: 230,
        bytes: 5_520,
        instance_sha256: "59219daf7d689dabfc251739ac0f35224c5ba428779b56dab3c01ed24cd7fe24",
    },
    ExpectedLibraryBucket {
        bucket_id: 1147,
        palette: 5,
        chunk_x: 13,
        chunk_z: 5,
        global_ordinal: 108,
        records: 180,
        bytes: 4_320,
        instance_sha256: "3df760762625b7896b742c677201999846c27bb7ea9515896bc632565038ceff",
    },
    ExpectedLibraryBucket {
        bucket_id: 1148,
        palette: 5,
        chunk_x: 14,
        chunk_z: 5,
        global_ordinal: 109,
        records: 6,
        bytes: 144,
        instance_sha256: "37ce881e604f08f59084cf1f9ca7e611dace86520e2668ac90df4935055a7e87",
    },
    ExpectedLibraryBucket {
        bucket_id: 1166,
        palette: 5,
        chunk_x: 10,
        chunk_z: 6,
        global_ordinal: 120,
        records: 37,
        bytes: 888,
        instance_sha256: "9d83a0141b5cdb87dbf2f59e9c1988aa0379615913c8d132474d12dd1d88e1dc",
    },
    ExpectedLibraryBucket {
        bucket_id: 1167,
        palette: 5,
        chunk_x: 11,
        chunk_z: 6,
        global_ordinal: 121,
        records: 452,
        bytes: 10_848,
        instance_sha256: "20961cf6ecf9cda1c2a134498ac748235441b166f9128c18da9441c88f45ace8",
    },
    ExpectedLibraryBucket {
        bucket_id: 1168,
        palette: 5,
        chunk_x: 12,
        chunk_z: 6,
        global_ordinal: 122,
        records: 616,
        bytes: 14_784,
        instance_sha256: "8050de8fc60c144d79a171d372c6a1c80590b83ca02b3b285089a90795e19a97",
    },
    ExpectedLibraryBucket {
        bucket_id: 1169,
        palette: 5,
        chunk_x: 13,
        chunk_z: 6,
        global_ordinal: 123,
        records: 856,
        bytes: 20_544,
        instance_sha256: "885d36ea204e82952c50c830e2052d55c9f4d9706c957c520f6eb05f99b6acf3",
    },
    ExpectedLibraryBucket {
        bucket_id: 1170,
        palette: 5,
        chunk_x: 14,
        chunk_z: 6,
        global_ordinal: 124,
        records: 1_041,
        bytes: 24_984,
        instance_sha256: "0a1f42e7ef72f0fe5a8e2e2b9c5e0c9ca1215943559457add96e6034a8840e90",
    },
    ExpectedLibraryBucket {
        bucket_id: 1184,
        palette: 5,
        chunk_x: 6,
        chunk_z: 7,
        global_ordinal: 136,
        records: 35,
        bytes: 840,
        instance_sha256: "ae42646f532f21fe0c972020ef92522416d0b09d4db199458a98adb92c97a0a2",
    },
    ExpectedLibraryBucket {
        bucket_id: 1185,
        palette: 5,
        chunk_x: 7,
        chunk_z: 7,
        global_ordinal: 137,
        records: 19,
        bytes: 456,
        instance_sha256: "1655026c46fc211e5a6f1eed42974dae1f68705101959555c3f06138e6c5c5b4",
    },
    ExpectedLibraryBucket {
        bucket_id: 1378,
        palette: 6,
        chunk_x: 10,
        chunk_z: 4,
        global_ordinal: 164,
        records: 5,
        bytes: 120,
        instance_sha256: "3b0d3e26e0a92c24c416fcc8c5bc16ad6a483eff6a3b2c240b455cd5a7330ef4",
    },
    ExpectedLibraryBucket {
        bucket_id: 1400,
        palette: 6,
        chunk_x: 10,
        chunk_z: 5,
        global_ordinal: 173,
        records: 141,
        bytes: 3_384,
        instance_sha256: "5988088b6b5dc704ddde071de395388ab5d85a0330e8d1d18308039f1d66f239",
    },
    ExpectedLibraryBucket {
        bucket_id: 1401,
        palette: 6,
        chunk_x: 11,
        chunk_z: 5,
        global_ordinal: 174,
        records: 416,
        bytes: 9_984,
        instance_sha256: "56a8fe5381f46def68b5e80b6ee041cbc0674892b84d8cb810b2897f980d0b74",
    },
    ExpectedLibraryBucket {
        bucket_id: 1402,
        palette: 6,
        chunk_x: 12,
        chunk_z: 5,
        global_ordinal: 175,
        records: 337,
        bytes: 8_088,
        instance_sha256: "c83f20ed4628cf64084adf263e37faf26490c6933b1a8d0fc60071c9f72874cd",
    },
    ExpectedLibraryBucket {
        bucket_id: 1403,
        palette: 6,
        chunk_x: 13,
        chunk_z: 5,
        global_ordinal: 176,
        records: 262,
        bytes: 6_288,
        instance_sha256: "f2c4eb06aeff7e703c1b19bf92fe1e9cf5a3a6eb2e070ac2f555b6681afa3976",
    },
    ExpectedLibraryBucket {
        bucket_id: 1404,
        palette: 6,
        chunk_x: 14,
        chunk_z: 5,
        global_ordinal: 177,
        records: 20,
        bytes: 480,
        instance_sha256: "e6eca5c94706f8571e9dd55b04af191203d8551de5989191b2ffa12268a93e74",
    },
    ExpectedLibraryBucket {
        bucket_id: 1422,
        palette: 6,
        chunk_x: 10,
        chunk_z: 6,
        global_ordinal: 188,
        records: 69,
        bytes: 1_656,
        instance_sha256: "3afed6a88129f81e65a474e9141413372578f59c3debe5fd5a080aa49b4162e0",
    },
    ExpectedLibraryBucket {
        bucket_id: 1423,
        palette: 6,
        chunk_x: 11,
        chunk_z: 6,
        global_ordinal: 189,
        records: 783,
        bytes: 18_792,
        instance_sha256: "e12cd7015191eea49cff41da4fa632305cc3c4ae319f4ece4f183718d41391e6",
    },
    ExpectedLibraryBucket {
        bucket_id: 1424,
        palette: 6,
        chunk_x: 12,
        chunk_z: 6,
        global_ordinal: 190,
        records: 1_316,
        bytes: 31_584,
        instance_sha256: "05d1783ee39ff7be3603ef86b01a1dc2be19100aa404fb640de526c57fb216f3",
    },
    ExpectedLibraryBucket {
        bucket_id: 1425,
        palette: 6,
        chunk_x: 13,
        chunk_z: 6,
        global_ordinal: 191,
        records: 2_288,
        bytes: 54_912,
        instance_sha256: "6e6fc91359c330061ae8ff110115e8d9727d4e9ce12e119c23e478eaff299578",
    },
    ExpectedLibraryBucket {
        bucket_id: 1426,
        palette: 6,
        chunk_x: 14,
        chunk_z: 6,
        global_ordinal: 192,
        records: 1_498,
        bytes: 35_952,
        instance_sha256: "eaceb3aa6c98d66e040c27a4b7477b703c92916606a2ec4de26add825c4779e9",
    },
    ExpectedLibraryBucket {
        bucket_id: 1440,
        palette: 6,
        chunk_x: 6,
        chunk_z: 7,
        global_ordinal: 205,
        records: 21,
        bytes: 504,
        instance_sha256: "881dbb86c1eda0c31f417cf7befb36a765e9730f48a234330631a715f79ead10",
    },
    ExpectedLibraryBucket {
        bucket_id: 1441,
        palette: 6,
        chunk_x: 7,
        chunk_z: 7,
        global_ordinal: 206,
        records: 71,
        bytes: 1_704,
        instance_sha256: "fe4139f5236fbf57f5b21ff8cb8957381c9cf783f558b4e7ed23988ca2f3eac2",
    },
];

#[allow(dead_code)]
const RETAINED_EXPECTED_LIBRARY_CELL_RANGES: [(u32, u32); 212] = [
    (22333, 22333),
    (22829, 22843),
    (23333, 23351),
    (23835, 23860),
    (24348, 24374),
    (24882, 24908),
    (25420, 25450),
    (25995, 26153),
    (26531, 26693),
    (27078, 27236),
    (27614, 27764),
    (28131, 28291),
    (28660, 28822),
    (29210, 29385),
    (29774, 29936),
    (30321, 30472),
    (30882, 31040),
    (31447, 31608),
    (32014, 32174),
    (32575, 32723),
    (33095, 33246),
    (33617, 33763),
    (34138, 34267),
    (34660, 34786),
    (35188, 35324),
    (35734, 35882),
    (36294, 36439),
    (36865, 37009),
    (37438, 37584),
    (38005, 38162),
    (38572, 38715),
    (39110, 39256),
    (39646, 39792),
    (40178, 40314),
    (40684, 40804),
    (41152, 41283),
    (41602, 41739),
    (42047, 42197),
    (42492, 42643),
    (42919, 43070),
    (43338, 43473),
    (43750, 43886),
    (44178, 44323),
    (44628, 44759),
    (45072, 45202),
    (45514, 45654),
    (45948, 46095),
    (46366, 46513),
    (46780, 46924),
    (47180, 47327),
    (47562, 47688),
    (47916, 48016),
    (48244, 48325),
    (48551, 48628),
    (48839, 48914),
    (49116, 49189),
    (49410, 49491),
    (49718, 49804),
    (50023, 50108),
    (50317, 50400),
    (50605, 50694),
    (50899, 50987),
    (51191, 51283),
    (51494, 51595),
    (51810, 51907),
    (52125, 52224),
    (52440, 52520),
    (52725, 52794),
    (52999, 53080),
    (53284, 53385),
    (53581, 53682),
    (53878, 53956),
    (54143, 54210),
    (54391, 54452),
    (54626, 54691),
    (54866, 54932),
    (55103, 55164),
    (55339, 55397),
    (55575, 55635),
    (55801, 55864),
    (56023, 56089),
    (56238, 56303),
    (56432, 56482),
    (56597, 56645),
    (56742, 56792),
    (56893, 56948),
    (57039, 57098),
    (57198, 57258),
    (57374, 57462),
    (57585, 57675),
    (57800, 57889),
    (58016, 58096),
    (58215, 58288),
    (58406, 58453),
    (58567, 58614),
    (58722, 58785),
    (58886, 58950),
    (59042, 59100),
    (59190, 59247),
    (59316, 59368),
    (59426, 59463),
    (59518, 59549),
    (59604, 59635),
    (59688, 59717),
    (59758, 59788),
    (59820, 59847),
    (59877, 59894),
    (59929, 59942),
    (59975, 59987),
    (60021, 60037),
    (60071, 60095),
    (60129, 60152),
    (60185, 60207),
    (60243, 60263),
    (60298, 60323),
    (60366, 60392),
    (60439, 60464),
    (60511, 60531),
    (60577, 60600),
    (60638, 60660),
    (60703, 60730),
    (60782, 60809),
    (60874, 60893),
    (60955, 60967),
    (61022, 61032),
    (61084, 61106),
    (61160, 61180),
    (61241, 61257),
    (61320, 61334),
    (61396, 61404),
    (61457, 61466),
    (61517, 61529),
    (61568, 61576),
    (61611, 61621),
    (61648, 61656),
    (61679, 61683),
    (61703, 61707),
    (61725, 61730),
    (61745, 61758),
    (61771, 61778),
    (61787, 61789),
    (61793, 61799),
    (61808, 61814),
    (61828, 61831),
    (61844, 61848),
    (61855, 61862),
    (61866, 61874),
    (61877, 61886),
    (61892, 61903),
    (61909, 61922),
    (61926, 61940),
    (61944, 61974),
    (61979, 61980),
    (61987, 61988),
    (62004, 62007),
    (62021, 62021),
    (62036, 62036),
    (62051, 62052),
    (62088, 62088),
    (62097, 62102),
    (62112, 62114),
    (62126, 62128),
    (62141, 62144),
    (62155, 62159),
    (62171, 62177),
    (62191, 62197),
    (62210, 62216),
    (62229, 62233),
    (62256, 62259),
    (62285, 62287),
    (62319, 62320),
    (62409, 62410),
    (62441, 62442),
    (62474, 62479),
    (62518, 62523),
    (62561, 62567),
    (62599, 62604),
    (62643, 62650),
    (62688, 62691),
    (62731, 62734),
    (62800, 62800),
    (62955, 62956),
    (62991, 62992),
    (63024, 63024),
    (63077, 63077),
    (63097, 63099),
    (63119, 63121),
    (63137, 63139),
    (63165, 63167),
    (63186, 63189),
    (63206, 63210),
    (63223, 63225),
    (63258, 63263),
    (63285, 63290),
    (63314, 63319),
    (63336, 63338),
    (63349, 63351),
    (63361, 63362),
    (63370, 63370),
    (63381, 63381),
    (63458, 63458),
    (63483, 63483),
    (63505, 63505),
    (63533, 63539),
    (63576, 63585),
    (63625, 63631),
    (63690, 63695),
    (63766, 63767),
    (63837, 63838),
    (63899, 63900),
    (63958, 63962),
    (64018, 64022),
];

include!("c07_tree_complete_buckets.rs");
const EXPECTED_LIBRARY_CELL_RANGES: [(u32, u32); 1] = [(0, 99_055)];

const EXPECTED_BUCKETS: [ExpectedBucket; C07_TREE_RETAINED_BUCKETS] = [
    ExpectedBucket {
        bucket_id: 634,
        palette: 3,
        chunk_x: 12,
        chunk_z: 5,
        records: 2,
        bytes: 48,
        instance_sha256: "0621eb7cb25f2bae50be9b003169d48198c292b36307102c5f36cc7fb3112e07",
    },
    ExpectedBucket {
        bucket_id: 1144,
        palette: 5,
        chunk_x: 10,
        chunk_z: 5,
        records: 86,
        bytes: 2_064,
        instance_sha256: "acb488ccbd70a1550111c121681f5c2abfc295599f82552dfb5c2b8d40cd3ed2",
    },
    ExpectedBucket {
        bucket_id: 1145,
        palette: 5,
        chunk_x: 11,
        chunk_z: 5,
        records: 296,
        bytes: 7_104,
        instance_sha256: "dca20484fbf4397f4150b7d01d4a40030d948c07bdfa2685d8bea4fd8e9ebae7",
    },
    ExpectedBucket {
        bucket_id: 1146,
        palette: 5,
        chunk_x: 12,
        chunk_z: 5,
        records: 230,
        bytes: 5_520,
        instance_sha256: "59219daf7d689dabfc251739ac0f35224c5ba428779b56dab3c01ed24cd7fe24",
    },
    ExpectedBucket {
        bucket_id: 1166,
        palette: 5,
        chunk_x: 10,
        chunk_z: 6,
        records: 37,
        bytes: 888,
        instance_sha256: "9d83a0141b5cdb87dbf2f59e9c1988aa0379615913c8d132474d12dd1d88e1dc",
    },
    ExpectedBucket {
        bucket_id: 1167,
        palette: 5,
        chunk_x: 11,
        chunk_z: 6,
        records: 452,
        bytes: 10_848,
        instance_sha256: "20961cf6ecf9cda1c2a134498ac748235441b166f9128c18da9441c88f45ace8",
    },
    ExpectedBucket {
        bucket_id: 1168,
        palette: 5,
        chunk_x: 12,
        chunk_z: 6,
        records: 616,
        bytes: 14_784,
        instance_sha256: "8050de8fc60c144d79a171d372c6a1c80590b83ca02b3b285089a90795e19a97",
    },
    ExpectedBucket {
        bucket_id: 1400,
        palette: 6,
        chunk_x: 10,
        chunk_z: 5,
        records: 141,
        bytes: 3_384,
        instance_sha256: "5988088b6b5dc704ddde071de395388ab5d85a0330e8d1d18308039f1d66f239",
    },
    ExpectedBucket {
        bucket_id: 1401,
        palette: 6,
        chunk_x: 11,
        chunk_z: 5,
        records: 416,
        bytes: 9_984,
        instance_sha256: "56a8fe5381f46def68b5e80b6ee041cbc0674892b84d8cb810b2897f980d0b74",
    },
    ExpectedBucket {
        bucket_id: 1402,
        palette: 6,
        chunk_x: 12,
        chunk_z: 5,
        records: 337,
        bytes: 8_088,
        instance_sha256: "c83f20ed4628cf64084adf263e37faf26490c6933b1a8d0fc60071c9f72874cd",
    },
    ExpectedBucket {
        bucket_id: 1422,
        palette: 6,
        chunk_x: 10,
        chunk_z: 6,
        records: 69,
        bytes: 1_656,
        instance_sha256: "3afed6a88129f81e65a474e9141413372578f59c3debe5fd5a080aa49b4162e0",
    },
    ExpectedBucket {
        bucket_id: 1423,
        palette: 6,
        chunk_x: 11,
        chunk_z: 6,
        records: 783,
        bytes: 18_792,
        instance_sha256: "e12cd7015191eea49cff41da4fa632305cc3c4ae319f4ece4f183718d41391e6",
    },
    ExpectedBucket {
        bucket_id: 1424,
        palette: 6,
        chunk_x: 12,
        chunk_z: 6,
        records: 1_316,
        bytes: 31_584,
        instance_sha256: "05d1783ee39ff7be3603ef86b01a1dc2be19100aa404fb640de526c57fb216f3",
    },
];

const EXPECTED_DRAW_ORDER: [ExpectedDraw; C07_TREE_RETAINED_BUCKETS] = [
    ExpectedDraw {
        bucket_id: 634,
        index_count: 729,
        instance_count: 2,
        mesh_vb_bytes: 22_680,
        mesh_vb_sha256: "d91f7f4866e3f7cfc8e900559ffae3e79d2951ee7dd8be340c5542f8092a5015",
        mesh_ib_bytes: 1_458,
        mesh_ib_sha256: "e8a33a1991fde9cb21b0e78713a4748463ecbf2d59472b041d7f45ded4982974",
    },
    ExpectedDraw {
        bucket_id: 1145,
        index_count: 672,
        instance_count: 296,
        mesh_vb_bytes: 20_160,
        mesh_vb_sha256: "aed2a35ee4d37c21a8d0e37291d094af92d4289e520b44f6ade1ada9144955f7",
        mesh_ib_bytes: 1_344,
        mesh_ib_sha256: "6a33db4c518efa9a7e85be99ff6143d7a054f86b63d3473a696c9d76238d8a97",
    },
    ExpectedDraw {
        bucket_id: 1167,
        index_count: 216,
        instance_count: 452,
        mesh_vb_bytes: 6_720,
        mesh_vb_sha256: "a6118ef3faf9a96551547f8233e5e1f0a1612b0ce82c9bd04a0448a854d9ef30",
        mesh_ib_bytes: 432,
        mesh_ib_sha256: "63739844f8222d6d83d6507789d5f122f980b7456bc295dd572d1df32fc4dc92",
    },
    ExpectedDraw {
        bucket_id: 1146,
        index_count: 216,
        instance_count: 230,
        mesh_vb_bytes: 6_720,
        mesh_vb_sha256: "a6118ef3faf9a96551547f8233e5e1f0a1612b0ce82c9bd04a0448a854d9ef30",
        mesh_ib_bytes: 432,
        mesh_ib_sha256: "63739844f8222d6d83d6507789d5f122f980b7456bc295dd572d1df32fc4dc92",
    },
    ExpectedDraw {
        bucket_id: 1168,
        index_count: 216,
        instance_count: 616,
        mesh_vb_bytes: 6_720,
        mesh_vb_sha256: "a6118ef3faf9a96551547f8233e5e1f0a1612b0ce82c9bd04a0448a854d9ef30",
        mesh_ib_bytes: 432,
        mesh_ib_sha256: "63739844f8222d6d83d6507789d5f122f980b7456bc295dd572d1df32fc4dc92",
    },
    ExpectedDraw {
        bucket_id: 1144,
        index_count: 216,
        instance_count: 86,
        mesh_vb_bytes: 6_720,
        mesh_vb_sha256: "a6118ef3faf9a96551547f8233e5e1f0a1612b0ce82c9bd04a0448a854d9ef30",
        mesh_ib_bytes: 432,
        mesh_ib_sha256: "63739844f8222d6d83d6507789d5f122f980b7456bc295dd572d1df32fc4dc92",
    },
    ExpectedDraw {
        bucket_id: 1166,
        index_count: 216,
        instance_count: 37,
        mesh_vb_bytes: 6_720,
        mesh_vb_sha256: "a6118ef3faf9a96551547f8233e5e1f0a1612b0ce82c9bd04a0448a854d9ef30",
        mesh_ib_bytes: 432,
        mesh_ib_sha256: "63739844f8222d6d83d6507789d5f122f980b7456bc295dd572d1df32fc4dc92",
    },
    ExpectedDraw {
        bucket_id: 1401,
        index_count: 558,
        instance_count: 416,
        mesh_vb_bytes: 16_800,
        mesh_vb_sha256: "6df4306c980812091df3e64d22150f5b91526b4775f182391f68073b3c01a803",
        mesh_ib_bytes: 1_116,
        mesh_ib_sha256: "946e369c074edc1a5b42abfadd21d5d75309a139343f535b13b8873c30d30d9a",
    },
    ExpectedDraw {
        bucket_id: 1423,
        index_count: 252,
        instance_count: 783,
        mesh_vb_bytes: 7_840,
        mesh_vb_sha256: "536407cc1ca471757335e185e433fa334d38b9d14cb27fb4ef3bcfe95eb473e2",
        mesh_ib_bytes: 504,
        mesh_ib_sha256: "6589f569a7b71213b2f12afeb2d7dae2a916e8779f00dfe672e5c70f67a46d6b",
    },
    ExpectedDraw {
        bucket_id: 1402,
        index_count: 252,
        instance_count: 337,
        mesh_vb_bytes: 7_840,
        mesh_vb_sha256: "536407cc1ca471757335e185e433fa334d38b9d14cb27fb4ef3bcfe95eb473e2",
        mesh_ib_bytes: 504,
        mesh_ib_sha256: "6589f569a7b71213b2f12afeb2d7dae2a916e8779f00dfe672e5c70f67a46d6b",
    },
    ExpectedDraw {
        bucket_id: 1424,
        index_count: 252,
        instance_count: 1_316,
        mesh_vb_bytes: 7_840,
        mesh_vb_sha256: "536407cc1ca471757335e185e433fa334d38b9d14cb27fb4ef3bcfe95eb473e2",
        mesh_ib_bytes: 504,
        mesh_ib_sha256: "6589f569a7b71213b2f12afeb2d7dae2a916e8779f00dfe672e5c70f67a46d6b",
    },
    ExpectedDraw {
        bucket_id: 1400,
        index_count: 252,
        instance_count: 141,
        mesh_vb_bytes: 7_840,
        mesh_vb_sha256: "536407cc1ca471757335e185e433fa334d38b9d14cb27fb4ef3bcfe95eb473e2",
        mesh_ib_bytes: 504,
        mesh_ib_sha256: "6589f569a7b71213b2f12afeb2d7dae2a916e8779f00dfe672e5c70f67a46d6b",
    },
    ExpectedDraw {
        bucket_id: 1422,
        index_count: 252,
        instance_count: 69,
        mesh_vb_bytes: 7_840,
        mesh_vb_sha256: "536407cc1ca471757335e185e433fa334d38b9d14cb27fb4ef3bcfe95eb473e2",
        mesh_ib_bytes: 504,
        mesh_ib_sha256: "6589f569a7b71213b2f12afeb2d7dae2a916e8779f00dfe672e5c70f67a46d6b",
    },
];

const EXPECTED_PASSES: [(C07TreePassKind, &str, &str); C07_TREE_RETAINED_PASSES] = [
    (
        C07TreePassKind::Shadow,
        "8d0282797eba0f96e188934d1b39af4dcc799dfca5e752b3ae3d9711ec3dc381",
        "9f41fd0da5f2b65eef99944ec3b273921a80907ba1169fe890ed3b744cc0bd2e",
    ),
    (
        C07TreePassKind::Unlit,
        "41a6bbe3f07796ea6af979188969520ee8969a1fa2669a3edd63bb33700d2830",
        "da5469c914e979db49e3bfeb247317fa80db1d345de4c565bbfa2500036923e4",
    ),
    (
        C07TreePassKind::Main,
        "41a6bbe3f07796ea6af979188969520ee8969a1fa2669a3edd63bb33700d2830",
        "be306dd34558fa5aade221ee6f34bc164d2b0dd4e0123923174349029d7791be",
    ),
];

fn expected_s1_tree_asset(bucket_id: u16) -> Option<ExpectedS1TreeAsset> {
    let asset = match bucket_id {
        634 => ExpectedS1TreeAsset {
            selected_lod_index: 1,
            selected_mesh: PALMS_BLEND_MESH,
            selected_material: PALMS_BLEND_LOD1_MATERIAL,
            material_textures: &PALM_MATERIAL_TEXTURES,
        },
        1145 => ExpectedS1TreeAsset {
            selected_lod_index: 1,
            selected_mesh: FOREST_DUNGE_BLEND_MESH,
            selected_material: FOREST_DUNGE_BLEND_LOD1_MATERIAL,
            material_textures: &FOREST_MATERIAL_TEXTURES,
        },
        1144 | 1146 | 1166 | 1167 | 1168 => ExpectedS1TreeAsset {
            selected_lod_index: 2,
            selected_mesh: FOREST_DUNGE_BLEND_MESH,
            selected_material: FOREST_DUNGE_BLEND_LOD2_MATERIAL,
            material_textures: &FOREST_MATERIAL_TEXTURES,
        },
        1401 => ExpectedS1TreeAsset {
            selected_lod_index: 1,
            selected_mesh: FOREST_DUNGE_MESH,
            selected_material: FOREST_DUNGE_LOD1_MATERIAL,
            material_textures: &FOREST_MATERIAL_TEXTURES,
        },
        1400 | 1402 | 1422 | 1423 | 1424 => ExpectedS1TreeAsset {
            selected_lod_index: 2,
            selected_mesh: FOREST_DUNGE_MESH,
            selected_material: FOREST_DUNGE_LOD2_MATERIAL,
            material_textures: &FOREST_MATERIAL_TEXTURES,
        },
        _ => return None,
    };
    Some(asset)
}

impl C07TreeContentLibrary {
    pub fn authenticated_retained_v1() -> Result<Self, String> {
        Ok(Self::authenticated_retained_v1_ref()?.clone())
    }

    pub fn authenticated_retained_v1_ref() -> Result<&'static Self, String> {
        static LIBRARY: OnceLock<Result<C07TreeContentLibrary, String>> = OnceLock::new();
        LIBRARY
            .get_or_init(Self::build_authenticated_retained_v1)
            .as_ref()
            .map_err(|error| error.clone())
    }

    fn build_authenticated_retained_v1() -> Result<Self, String> {
        let mut packed_byte_offset = 0u32;
        let ordered_buckets = EXPECTED_LIBRARY_BUCKETS
            .iter()
            .map(|expected| {
                let offset = packed_byte_offset;
                packed_byte_offset += expected.bytes;
                C07TreeImmutableBucketContent {
                    bucket_id: expected.bucket_id,
                    palette_index: expected.palette,
                    chunk_x: expected.chunk_x,
                    chunk_z: expected.chunk_z,
                    global_producer_create_ordinal: expected.global_ordinal,
                    packed_byte_offset: offset,
                    record_count: expected.records,
                    instance_bytes: expected.bytes,
                    instance_payload_sha256: expected.instance_sha256.into(),
                }
            })
            .collect();
        let authenticated_cell_ordinal_ranges = EXPECTED_LIBRARY_CELL_RANGES
            .iter()
            .map(|(first_ordinal, last_ordinal)| C07TreeCellOrdinalRange {
                first_ordinal: *first_ordinal,
                last_ordinal: *last_ordinal,
            })
            .collect();
        let mut library = Self {
            schema: C07_TREE_CONTENT_LIBRARY_SCHEMA.into(),
            executable_sha256: C07_TREE_A729_EXECUTABLE_SHA256.into(),
            game_file_set_sha256: C07_TREE_CONTENT_LIBRARY_GAME_FILE_SET_SHA256.into(),
            census_payload_sha256: C07_TREE_CONTENT_LIBRARY_CENSUS_SHA256.into(),
            covered_cell_set_sha256: C07_TREE_CONTENT_LIBRARY_COVERED_CELL_SET_SHA256.into(),
            missing_cell_set_sha256: C07_TREE_CONTENT_LIBRARY_MISSING_CELL_SET_SHA256.into(),
            instance_stride_bytes: C07_TREE_INSTANCE_STRIDE_BYTES,
            global_nonzero_cells: C07_TREE_GLOBAL_NONZERO_CELLS,
            global_nonempty_buckets: 296,
            authenticated_cells: C07_TREE_CONTENT_LIBRARY_CELLS,
            authenticated_buckets: C07_TREE_CONTENT_LIBRARY_BUCKETS as u32,
            unavailable_cells: C07_TREE_CONTENT_LIBRARY_UNAVAILABLE_CELLS,
            unavailable_buckets: C07_TREE_CONTENT_LIBRARY_UNAVAILABLE_BUCKETS,
            ordered_buckets,
            authenticated_cell_ordinal_ranges,
            packed_instance_bytes: include_bytes!("c07_tree_content_library.bin").to_vec(),
            payload_sha256: String::new(),
        };
        library.seal_payload_sha256()?;
        library.validate()?;
        Ok(library)
    }

    pub fn computed_payload_sha256(&self) -> Result<String, String> {
        let mut canonical = self.clone();
        canonical.payload_sha256.clear();
        let bytes = serde_json::to_vec(&canonical).map_err(|error| error.to_string())?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }

    pub fn seal_payload_sha256(&mut self) -> Result<(), String> {
        self.payload_sha256 = self.computed_payload_sha256()?;
        Ok(())
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != C07_TREE_CONTENT_LIBRARY_SCHEMA
            || self.executable_sha256 != C07_TREE_A729_EXECUTABLE_SHA256
            || self.game_file_set_sha256 != C07_TREE_CONTENT_LIBRARY_GAME_FILE_SET_SHA256
            || self.census_payload_sha256 != C07_TREE_CONTENT_LIBRARY_CENSUS_SHA256
            || self.covered_cell_set_sha256 != C07_TREE_CONTENT_LIBRARY_COVERED_CELL_SET_SHA256
            || self.missing_cell_set_sha256 != C07_TREE_CONTENT_LIBRARY_MISSING_CELL_SET_SHA256
            || self.instance_stride_bytes != C07_TREE_INSTANCE_STRIDE_BYTES
            || self.global_nonzero_cells != C07_TREE_GLOBAL_NONZERO_CELLS
            || self.global_nonempty_buckets != 296
            || self.authenticated_cells != C07_TREE_CONTENT_LIBRARY_CELLS
            || self.authenticated_buckets != C07_TREE_CONTENT_LIBRARY_BUCKETS as u32
            || self.unavailable_cells != C07_TREE_CONTENT_LIBRARY_UNAVAILABLE_CELLS
            || self.unavailable_buckets != C07_TREE_CONTENT_LIBRARY_UNAVAILABLE_BUCKETS
            || self.authenticated_cells + self.unavailable_cells != self.global_nonzero_cells
            || self.authenticated_buckets + self.unavailable_buckets != self.global_nonempty_buckets
        {
            return Err("C07 tree content-library identity/coverage mismatch".into());
        }
        if self.ordered_buckets.len() != EXPECTED_LIBRARY_BUCKETS.len() {
            return Err("C07 tree content-library bucket count mismatch".into());
        }
        let mut expected_packed_offset = 0u32;
        for (ordinal, (actual, expected)) in self
            .ordered_buckets
            .iter()
            .zip(EXPECTED_LIBRARY_BUCKETS)
            .enumerate()
        {
            if actual.bucket_id != expected.bucket_id
                || actual.palette_index != expected.palette
                || actual.chunk_x != expected.chunk_x
                || actual.chunk_z != expected.chunk_z
                || actual.global_producer_create_ordinal != expected.global_ordinal
                || actual.packed_byte_offset != expected_packed_offset
                || actual.record_count != expected.records
                || actual.instance_bytes != expected.bytes
                || actual.instance_bytes != actual.record_count * C07_TREE_INSTANCE_STRIDE_BYTES
                || actual.instance_payload_sha256 != expected.instance_sha256
            {
                return Err(format!("C07 tree content-library bucket {ordinal} drift"));
            }
            let packed_end = expected_packed_offset
                .checked_add(actual.instance_bytes)
                .ok_or("C07 tree content-library packed offset overflow")?;
            let payload = self
                .packed_instance_bytes
                .get(expected_packed_offset as usize..packed_end as usize)
                .ok_or("C07 tree content-library packed payload is truncated")?;
            if format!("{:x}", Sha256::digest(payload)) != actual.instance_payload_sha256 {
                return Err(format!(
                    "C07 tree content-library packed bucket {ordinal} SHA-256 mismatch"
                ));
            }
            expected_packed_offset = packed_end;
        }
        if self.authenticated_cell_ordinal_ranges.len() != EXPECTED_LIBRARY_CELL_RANGES.len() {
            return Err("C07 tree content-library cell-range count mismatch".into());
        }
        let mut previous_last = None;
        let mut covered_cells = 0u32;
        for (ordinal, (actual, expected)) in self
            .authenticated_cell_ordinal_ranges
            .iter()
            .zip(EXPECTED_LIBRARY_CELL_RANGES)
            .enumerate()
        {
            if (actual.first_ordinal, actual.last_ordinal) != expected
                || actual.first_ordinal > actual.last_ordinal
                || actual.last_ordinal >= C07_TREE_GLOBAL_NONZERO_CELLS
                || previous_last.is_some_and(|last| actual.first_ordinal <= last)
            {
                return Err(format!(
                    "C07 tree content-library cell range {ordinal} drift"
                ));
            }
            covered_cells = covered_cells
                .checked_add(actual.last_ordinal - actual.first_ordinal + 1)
                .ok_or("C07 tree content-library cell-range count overflow")?;
            previous_last = Some(actual.last_ordinal);
        }
        if covered_cells != self.authenticated_cells
            || self
                .ordered_buckets
                .iter()
                .map(|bucket| bucket.record_count)
                .sum::<u32>()
                != self.authenticated_cells
            || self
                .ordered_buckets
                .iter()
                .map(|bucket| bucket.instance_bytes)
                .sum::<u32>()
                != C07_TREE_CONTENT_LIBRARY_INSTANCE_BYTES
            || expected_packed_offset != C07_TREE_CONTENT_LIBRARY_INSTANCE_BYTES
            || self.packed_instance_bytes.len() != C07_TREE_CONTENT_LIBRARY_INSTANCE_BYTES as usize
            || format!("{:x}", Sha256::digest(&self.packed_instance_bytes))
                != C07_TREE_CONTENT_LIBRARY_PACKED_BYTES_SHA256
        {
            return Err("C07 tree content-library cell/byte total mismatch".into());
        }
        if self.payload_sha256 != C07_TREE_CONTENT_LIBRARY_PAYLOAD_SHA256
            || self.computed_payload_sha256()? != self.payload_sha256
        {
            return Err("C07 tree content-library payload SHA-256 mismatch".into());
        }
        Ok(())
    }

    pub fn require_bucket(&self, bucket_id: u16) -> Result<&C07TreeImmutableBucketContent, String> {
        self.validate()?;
        self.bucket_by_id(bucket_id).ok_or_else(|| {
            format!(
                "C07 bucket {bucket_id} is outside the authenticated 296-bucket a729 content library"
            )
        })
    }

    pub fn admit_bucket_source(&self, bucket_id: u16, source: &BufferSource) -> Result<(), String> {
        let bucket = self.require_bucket(bucket_id)?;
        if let BufferSource::UsedContentRange {
            content,
            source_byte_offset,
            byte_count,
            ..
        } = source
        {
            content
                .validate()
                .map_err(|error| format!("C07 bucket content key rejected: {error}"))?;
            let end = (*source_byte_offset)
                .checked_add(*byte_count)
                .ok_or("C07 bucket used-content range overflows")?;
            if end > content.payload_bytes {
                return Err("C07 bucket used-content range exceeds its portable payload".into());
            }
        }
        let inline_bytes_match = match source {
            BufferSource::Inline { bytes, .. } => {
                bytes.as_slice() == self.bucket_payload_unchecked(bucket)?
            }
            _ => true,
        };
        if inline_bytes_match
            && source_matches(
                source,
                bucket.instance_bytes,
                &bucket.instance_payload_sha256,
            )
        {
            Ok(())
        } else {
            Err(format!(
                "C07 bucket {bucket_id} buffer source differs from immutable library content"
            ))
        }
    }

    fn bucket_by_id(&self, bucket_id: u16) -> Option<&C07TreeImmutableBucketContent> {
        self.ordered_buckets
            .binary_search_by_key(&bucket_id, |bucket| bucket.bucket_id)
            .map(|index| &self.ordered_buckets[index])
            .ok()
    }

    pub fn bucket_payload(&self, bucket_id: u16) -> Result<&[u8], String> {
        let bucket = self.require_bucket(bucket_id)?;
        self.bucket_payload_unchecked(bucket)
    }

    fn bucket_payload_unchecked(
        &self,
        bucket: &C07TreeImmutableBucketContent,
    ) -> Result<&[u8], String> {
        let end = bucket
            .packed_byte_offset
            .checked_add(bucket.instance_bytes)
            .ok_or("C07 tree content-library packed bucket range overflows")?;
        self.packed_instance_bytes
            .get(bucket.packed_byte_offset as usize..end as usize)
            .ok_or_else(|| "C07 tree content-library packed bucket range is unavailable".into())
    }

    pub fn require_cell_ordinal(&self, nonzero_cell_ordinal: u32) -> Result<(), String> {
        self.validate()?;
        if nonzero_cell_ordinal >= self.global_nonzero_cells {
            return Err(format!(
                "C07 nonzero cell ordinal {nonzero_cell_ordinal} is outside the 99,056-cell a729 pool"
            ));
        }
        if self
            .authenticated_cell_ordinal_ranges
            .iter()
            .any(|range| (range.first_ordinal..=range.last_ordinal).contains(&nonzero_cell_ordinal))
        {
            return Ok(());
        }
        Err(format!(
            "C07 nonzero cell ordinal {nonzero_cell_ordinal} is absent from the sealed complete-pool range"
        ))
    }

    pub fn require_complete_static_pool(&self) -> Result<(), String> {
        self.validate()?;
        if self.authenticated_cells == self.global_nonzero_cells
            && self.authenticated_buckets == self.global_nonempty_buckets
            && self.unavailable_cells == 0
            && self.unavailable_buckets == 0
        {
            Ok(())
        } else {
            Err("C07 complete static pool identity/coverage mismatch".into())
        }
    }

    fn validate_runtime_bucket_membership(
        &self,
        carrier: &C07TreeRuntimeCarrier,
    ) -> Result<(), String> {
        self.validate()?;
        self.validate_runtime_bucket_membership_trusted(carrier)
    }

    fn validate_runtime_bucket_membership_trusted(
        &self,
        carrier: &C07TreeRuntimeCarrier,
    ) -> Result<(), String> {
        let mut bucket_ids = BTreeSet::new();
        let mut selected_cells = 0u32;
        for bucket in &carrier.ordered_buckets {
            if !bucket_ids.insert(bucket.bucket_id) {
                return Err("C07 selected-submit subset repeats a content bucket".into());
            }
            let content = self.bucket_by_id(bucket.bucket_id).ok_or_else(|| {
                format!(
                    "C07 selected-submit bucket {} is outside the authenticated 296-bucket a729 content library",
                    bucket.bucket_id
                )
            })?;
            if bucket.palette_index != content.palette_index
                || bucket.chunk_x != content.chunk_x
                || bucket.chunk_z != content.chunk_z
                || bucket.record_count != content.record_count
                || bucket.instance_bytes != content.instance_bytes
                || bucket.instance_payload_sha256 != content.instance_payload_sha256
            {
                return Err(format!(
                    "C07 selected-submit bucket {} differs from immutable library content",
                    bucket.bucket_id
                ));
            }
            selected_cells = selected_cells
                .checked_add(bucket.record_count)
                .ok_or("C07 selected-submit subset cell count overflow")?;
        }
        if selected_cells != carrier.coverage.captured_nonzero_cells {
            return Err("C07 selected-submit subset coverage differs from bucket content".into());
        }
        Ok(())
    }

    pub fn select_for_submit(
        &self,
        carrier: &C07TreeRuntimeCarrier,
        submit: &RenderSubmit,
    ) -> Result<C07TreeContentSelection, String> {
        self.validate()?;
        carrier
            .validate_against_submit(submit, C07TreeCoverageRequirement::RetainedVisibleSubset)?;
        self.validate_runtime_bucket_membership(carrier)?;
        C07TreeContentSelection::from_validated_subset(self, carrier)
    }
}

impl C07TreeContentSelection {
    fn from_validated_subset(
        library: &C07TreeContentLibrary,
        carrier: &C07TreeRuntimeCarrier,
    ) -> Result<Self, String> {
        let selected_cells = carrier
            .ordered_buckets
            .iter()
            .map(|bucket| bucket.record_count)
            .sum();
        let selected_instance_bytes = carrier
            .ordered_buckets
            .iter()
            .map(|bucket| bucket.instance_bytes)
            .sum();
        let mut selection = Self {
            schema: C07_TREE_CONTENT_SELECTION_SCHEMA.into(),
            library_payload_sha256: library.payload_sha256.clone(),
            selected_submit_tree_runtime_payload_sha256: carrier.payload_sha256.clone(),
            ordered_bucket_ids: carrier
                .ordered_buckets
                .iter()
                .map(|bucket| bucket.bucket_id)
                .collect(),
            selected_cells,
            selected_instance_bytes,
            unselected_library_cells: library.authenticated_cells - selected_cells,
            globally_unavailable_cells: library.unavailable_cells,
            payload_sha256: String::new(),
        };
        selection.seal_payload_sha256()?;
        selection.validate_against_trusted(library, carrier)?;
        Ok(selection)
    }

    pub fn computed_payload_sha256(&self) -> Result<String, String> {
        let mut canonical = self.clone();
        canonical.payload_sha256.clear();
        let bytes = serde_json::to_vec(&canonical).map_err(|error| error.to_string())?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }

    pub fn seal_payload_sha256(&mut self) -> Result<(), String> {
        self.payload_sha256 = self.computed_payload_sha256()?;
        Ok(())
    }

    pub fn validate_against(
        &self,
        library: &C07TreeContentLibrary,
        carrier: &C07TreeRuntimeCarrier,
    ) -> Result<(), String> {
        library.validate()?;
        self.validate_against_trusted(library, carrier)
    }

    fn validate_against_trusted(
        &self,
        library: &C07TreeContentLibrary,
        carrier: &C07TreeRuntimeCarrier,
    ) -> Result<(), String> {
        library.validate_runtime_bucket_membership_trusted(carrier)?;
        let ordered_bucket_ids = carrier
            .ordered_buckets
            .iter()
            .map(|bucket| bucket.bucket_id)
            .collect::<Vec<_>>();
        let selected_cells = carrier
            .ordered_buckets
            .iter()
            .map(|bucket| bucket.record_count)
            .sum::<u32>();
        if self.schema != C07_TREE_CONTENT_SELECTION_SCHEMA
            || self.library_payload_sha256 != library.payload_sha256
            || self.selected_submit_tree_runtime_payload_sha256 != carrier.payload_sha256
            || self.ordered_bucket_ids != ordered_bucket_ids
            || self.selected_cells != selected_cells
            || self.selected_instance_bytes != selected_cells * C07_TREE_INSTANCE_STRIDE_BYTES
            || self.unselected_library_cells != library.authenticated_cells - selected_cells
            || self.globally_unavailable_cells != library.unavailable_cells
            || self.payload_sha256.len() != 64
            || !self
                .payload_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || self.computed_payload_sha256()? != self.payload_sha256
        {
            return Err("C07 selected-submit content selection mismatch".into());
        }
        Ok(())
    }
}

impl C07TreeRuntimeCarrier {
    pub fn is_retained_tree_shader_draw(draw: &StaticMapDraw) -> bool {
        EXPECTED_PASSES.iter().any(|(_, vertex, pixel)| {
            draw.shader_program.compiled_vertex_shader.sha256 == *vertex
                && draw.shader_program.compiled_pixel_shader.sha256 == *pixel
        })
    }

    pub fn from_authenticated_selected_submit(
        submit: &RenderSubmit,
    ) -> Result<Option<Self>, String> {
        if submit.ordered_static_map_programs.is_empty()
            || submit.ordered_render_operations.is_empty()
        {
            return Ok(None);
        }

        let mut static_operations = BTreeMap::<u32, &StaticMapOperation>::new();
        for program in &submit.ordered_static_map_programs {
            for operation in &program.ordered_operations {
                if static_operations
                    .insert(operation.operation_ordinal, operation)
                    .is_some()
                {
                    return Err(
                        "C07 tree discovery sees duplicate static operation ordinal".into()
                    );
                }
            }
        }

        let library = C07TreeContentLibrary::authenticated_retained_v1_ref()?;
        library.require_complete_static_pool()?;
        let mut pass_draws = [Vec::new(), Vec::new(), Vec::new()];
        let mut selected_bucket_ids = BTreeSet::new();
        for execution in &submit.ordered_render_operations {
            let RenderExecutionRef::Static { operation_ordinal } = execution else {
                continue;
            };
            let Some(operation) = static_operations.get(operation_ordinal) else {
                return Err("C07 selected execution references an absent static draw".into());
            };
            // A selected execution of a non-draw static operation (target
            // clears and similar) is executed static work, not tree content -
            // skipped exactly like a draw whose shader pair is not a tree pass.
            let StaticMapCommand::Draw { draw } = &operation.command else {
                continue;
            };
            let Some((pass_index, expected_pass)) = EXPECTED_PASSES
                .iter()
                .copied()
                .enumerate()
                .find(|(_, (_, vertex, pixel))| {
                    draw.shader_program.compiled_vertex_shader.sha256 == *vertex
                        && draw.shader_program.compiled_pixel_shader.sha256 == *pixel
                })
            else {
                continue;
            };
            let instance_source = exact_tree_instance_source(draw)?;
            let matches = library
                .ordered_buckets
                .iter()
                .filter(|bucket| {
                    source_matches(
                        instance_source,
                        bucket.instance_bytes,
                        &bucket.instance_payload_sha256,
                    )
                })
                .collect::<Vec<_>>();
            let [bucket] = matches.as_slice() else {
                return Err(format!(
                    "C07 selected tree operation {operation_ordinal} does not match exactly one complete-pool producer bucket"
                ));
            };
            let bucket_carrier = C07TreeBucketCarrier {
                bucket_id: bucket.bucket_id,
                palette_index: bucket.palette_index,
                chunk_x: bucket.chunk_x,
                chunk_z: bucket.chunk_z,
                producer_create_ordinal: bucket.global_producer_create_ordinal,
                record_count: bucket.record_count,
                instance_bytes: bucket.instance_bytes,
                instance_payload_sha256: bucket.instance_payload_sha256.clone(),
            };
            validate_generic_draw(draw, &bucket_carrier, expected_pass)?;
            if pass_draws[pass_index]
                .iter()
                .any(|row: &C07TreeDrawRef| row.bucket_id == bucket.bucket_id)
            {
                return Err("C07 selected tree pass repeats a producer bucket".into());
            }
            pass_draws[pass_index].push(C07TreeDrawRef {
                operation_ordinal: *operation_ordinal,
                bucket_id: bucket.bucket_id,
            });
            selected_bucket_ids.insert(bucket.bucket_id);
        }
        if pass_draws.iter().all(Vec::is_empty) {
            return Ok(None);
        }
        if pass_draws.iter().any(Vec::is_empty) {
            return Err("C07 selected submit contains an incomplete tree pass family".into());
        }
        let ordered_buckets = selected_bucket_ids
            .iter()
            .map(|bucket_id| {
                let bucket = library.require_bucket(*bucket_id)?;
                Ok(C07TreeBucketCarrier {
                    bucket_id: bucket.bucket_id,
                    palette_index: bucket.palette_index,
                    chunk_x: bucket.chunk_x,
                    chunk_z: bucket.chunk_z,
                    producer_create_ordinal: bucket.global_producer_create_ordinal,
                    record_count: bucket.record_count,
                    instance_bytes: bucket.instance_bytes,
                    instance_payload_sha256: bucket.instance_payload_sha256.clone(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let captured_nonzero_cells = ordered_buckets
            .iter()
            .map(|bucket| bucket.record_count)
            .sum::<u32>();
        let ordered_passes = EXPECTED_PASSES
            .iter()
            .enumerate()
            .map(|(index, (pass, vertex, pixel))| C07TreePassCarrier {
                pass: *pass,
                vertex_shader_sha256: (*vertex).into(),
                pixel_shader_sha256: (*pixel).into(),
                ordered_draws: std::mem::take(&mut pass_draws[index]),
            })
            .collect();
        let selected_submit_payload_sha256 = Self::selected_submit_hash_projection(submit)?;
        let mut carrier = Self {
            schema: C07_TREE_RUNTIME_SCHEMA.into(),
            executable_sha256: C07_TREE_A729_EXECUTABLE_SHA256.into(),
            retained_contract_sha256: C07_TREE_RETAINED_CONTRACT_SHA256.into(),
            retained_rdc_sha256: selected_submit_payload_sha256,
            producer_function_rva: C07_TREE_PRODUCER_FUNCTION_RVA,
            buffer_factory_callsite_rva: C07_TREE_BUFFER_FACTORY_CALLSITE_RVA,
            coverage: C07TreeCoverage {
                kind: C07TreeCoverageKind::RetainedVisibleSubset,
                global_nonzero_cells: C07_TREE_GLOBAL_NONZERO_CELLS,
                captured_nonzero_cells,
                uncaptured_nonzero_cells: C07_TREE_GLOBAL_NONZERO_CELLS
                    .checked_sub(captured_nonzero_cells)
                    .ok_or("C07 selected bucket union exceeds the complete pool")?,
            },
            ordered_buckets,
            ordered_passes,
            payload_sha256: String::new(),
        };
        carrier.seal_payload_sha256()?;
        carrier
            .validate_against_submit(submit, C07TreeCoverageRequirement::RetainedVisibleSubset)?;
        Ok(Some(carrier))
    }

    #[doc(hidden)]
    pub fn from_authenticated_submit_retained_frame36669(
        submit: &RenderSubmit,
    ) -> Result<Option<Self>, String> {
        Self::from_authenticated_selected_submit(submit)
    }

    pub fn retained_frame36669(
        ordered_operation_ordinals: [[u32; C07_TREE_RETAINED_BUCKETS]; C07_TREE_RETAINED_PASSES],
    ) -> Result<Self, String> {
        let library = C07TreeContentLibrary::authenticated_retained_v1_ref()?;
        let ordered_buckets = EXPECTED_BUCKETS
            .iter()
            .map(|expected| {
                let complete = library.require_bucket(expected.bucket_id)?;
                Ok(C07TreeBucketCarrier {
                    bucket_id: expected.bucket_id,
                    palette_index: expected.palette,
                    chunk_x: expected.chunk_x,
                    chunk_z: expected.chunk_z,
                    producer_create_ordinal: complete.global_producer_create_ordinal,
                    record_count: expected.records,
                    instance_bytes: expected.bytes,
                    instance_payload_sha256: expected.instance_sha256.into(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let ordered_passes = EXPECTED_PASSES
            .iter()
            .enumerate()
            .map(
                |(pass_index, (pass, vertex_sha256, pixel_sha256))| C07TreePassCarrier {
                    pass: *pass,
                    vertex_shader_sha256: (*vertex_sha256).into(),
                    pixel_shader_sha256: (*pixel_sha256).into(),
                    ordered_draws: EXPECTED_DRAW_ORDER
                        .iter()
                        .zip(ordered_operation_ordinals[pass_index])
                        .map(|(expected, operation_ordinal)| C07TreeDrawRef {
                            operation_ordinal,
                            bucket_id: expected.bucket_id,
                        })
                        .collect(),
                },
            )
            .collect();
        let mut carrier = Self {
            schema: C07_TREE_RUNTIME_SCHEMA.into(),
            executable_sha256: C07_TREE_A729_EXECUTABLE_SHA256.into(),
            retained_contract_sha256: C07_TREE_RETAINED_CONTRACT_SHA256.into(),
            retained_rdc_sha256: C07_TREE_RETAINED_RDC_SHA256.into(),
            producer_function_rva: C07_TREE_PRODUCER_FUNCTION_RVA,
            buffer_factory_callsite_rva: C07_TREE_BUFFER_FACTORY_CALLSITE_RVA,
            coverage: C07TreeCoverage {
                kind: C07TreeCoverageKind::RetainedVisibleSubset,
                global_nonzero_cells: C07_TREE_GLOBAL_NONZERO_CELLS,
                captured_nonzero_cells: C07_TREE_RETAINED_VISIBLE_CELLS,
                uncaptured_nonzero_cells: C07_TREE_UNCAPTURED_CELLS,
            },
            ordered_buckets,
            ordered_passes,
            payload_sha256: String::new(),
        };
        carrier.seal_payload_sha256()?;
        Ok(carrier)
    }

    pub fn seal_payload_sha256(&mut self) -> Result<(), String> {
        self.payload_sha256 = self.computed_payload_sha256()?;
        Ok(())
    }

    pub fn computed_payload_sha256(&self) -> Result<String, String> {
        let mut canonical = self.clone();
        canonical.payload_sha256.clear();
        let bytes = serde_json::to_vec(&canonical).map_err(|error| error.to_string())?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }

    pub fn required_operation_ordinals(&self) -> Vec<u32> {
        self.ordered_passes
            .iter()
            .flat_map(|pass| pass.ordered_draws.iter().map(|draw| draw.operation_ordinal))
            .collect()
    }

    pub fn require_coverage(&self, requirement: C07TreeCoverageRequirement) -> Result<(), String> {
        if requirement == C07TreeCoverageRequirement::CompleteStaticPool
            && self.coverage.kind != C07TreeCoverageKind::CompleteStaticPool
        {
            return Err(format!(
                "C07 complete tree pool required but retained carrier covers only {}/{} nonzero cells; {} uncaptured global cells remain",
                self.coverage.captured_nonzero_cells,
                self.coverage.global_nonzero_cells,
                self.coverage.uncaptured_nonzero_cells
            ));
        }
        Ok(())
    }

    /// Canonical hash projection of the selected submit for the generic
    /// (non-pinned) seal: the submit exactly as it stood when the capture-side
    /// producer folded the static programs and derived the C07 carrier — before
    /// the later S1 and weather folds mutated it. Excluding those two fields
    /// makes the derivation fold-position-invariant: deriving from a fully
    /// folded decoded submit equals, field for field, the output of the same
    /// producer function evaluated at its capture-time fold position (the
    /// kills-test pins this directly). This is not v13 byte-equality: the v13
    /// grammar embedded fields the v14 grammar derives, which is why the wire
    /// schema was bumped.
    fn selected_submit_hash_projection(submit: &RenderSubmit) -> Result<String, String> {
        let mut hashed = submit.clone();
        hashed.s1_shadow = None;
        hashed.weather_membership = Vec::new();
        Ok(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&hashed).map_err(|error| error.to_string())?)
        ))
    }

    pub fn validate_against_submit(
        &self,
        submit: &RenderSubmit,
        requirement: C07TreeCoverageRequirement,
    ) -> Result<(), String> {        self.validate_contract(requirement)?;
        let pinned_geometry_law = self.retained_rdc_sha256 == C07_TREE_RETAINED_RDC_SHA256;
        if !pinned_geometry_law {
            let selected_submit_sha256 = Self::selected_submit_hash_projection(submit)?;
            if self.retained_rdc_sha256 != selected_submit_sha256 {
                return Err("C07 runtime carrier targets a different selected submit".into());
            }
        }

        let mut static_draws = BTreeMap::<u32, &StaticMapDraw>::new();
        for program in &submit.ordered_static_map_programs {
            for operation in &program.ordered_operations {
                if let StaticMapCommand::Draw { draw } = &operation.command {
                    if static_draws
                        .insert(operation.operation_ordinal, draw)
                        .is_some()
                    {
                        return Err(
                            "C07 tree carrier sees duplicate static operation ordinal".into()
                        );
                    }
                }
            }
        }
        let execution_positions = submit
            .ordered_render_operations
            .iter()
            .enumerate()
            .filter_map(|(position, reference)| match reference {
                RenderExecutionRef::Static { operation_ordinal } => {
                    Some((*operation_ordinal, position))
                }
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();
        let buckets = self
            .ordered_buckets
            .iter()
            .map(|bucket| (bucket.bucket_id, bucket))
            .collect::<BTreeMap<_, _>>();
        let mut referenced_operations = BTreeSet::new();
        let tree_shader_operations = static_draws
            .iter()
            .filter_map(|(operation_ordinal, draw)| {
                Self::is_retained_tree_shader_draw(draw).then_some(*operation_ordinal)
            })
            .collect::<BTreeSet<_>>();

        let mut previous_tree_execution_position = None;
        for (pass, expected_pass) in self.ordered_passes.iter().zip(EXPECTED_PASSES) {
            for draw_ref in &pass.ordered_draws {
                if !referenced_operations.insert(draw_ref.operation_ordinal) {
                    return Err("C07 tree carrier repeats a static draw operation".into());
                }
                let execution_position = *execution_positions
                    .get(&draw_ref.operation_ordinal)
                    .ok_or("C07 tree draw is absent from unified render-operation order")?;
                if previous_tree_execution_position
                    .is_some_and(|previous| execution_position <= previous)
                {
                    return Err("C07 tree draw/pass order differs from exact consumer order".into());
                }
                previous_tree_execution_position = Some(execution_position);
                let draw = static_draws
                    .get(&draw_ref.operation_ordinal)
                    .copied()
                    .ok_or("C07 tree carrier references an absent static draw")?;
                let bucket = buckets
                    .get(&draw_ref.bucket_id)
                    .copied()
                    .ok_or("C07 tree draw references an absent bucket")?;
                validate_generic_draw(draw, bucket, expected_pass)?;
                if pinned_geometry_law {
                    let expected = EXPECTED_DRAW_ORDER
                        .iter()
                        .find(|row| row.bucket_id == draw_ref.bucket_id)
                        .ok_or(
                            "C07 pinned frame36669 carrier references a bucket outside the pinned draw law",
                        )?;
                    validate_draw(draw, bucket, expected, expected_pass)?;
                }
            }
        }
        if tree_shader_operations != referenced_operations {
            return Err(
                "C07 tree carrier does not cover the complete selected-submit tree shader operation set"
                    .into(),
            );
        }
        Ok(())
    }

    pub fn content_selection_against_submit(
        &self,
        submit: &RenderSubmit,
    ) -> Result<C07TreeContentSelection, String> {
        let library = C07TreeContentLibrary::authenticated_retained_v1_ref()?;
        self.validate_against_submit(submit, C07TreeCoverageRequirement::RetainedVisibleSubset)?;
        library.validate_runtime_bucket_membership_trusted(self)?;
        C07TreeContentSelection::from_validated_subset(library, self)
    }

    pub fn s1_primary_shadow_submissions_against_submit(
        &self,
        submit: &RenderSubmit,
    ) -> Result<Vec<S1LightDepthSubmission>, String> {
        self.validate_against_submit(submit, C07TreeCoverageRequirement::RetainedVisibleSubset)?;

        let shadow_pass = self
            .ordered_passes
            .iter()
            .find(|pass| pass.pass == C07TreePassKind::Shadow)
            .ok_or("C07 retained carrier has no shadow pass")?;
        let static_draws = submit
            .ordered_static_map_programs
            .iter()
            .flat_map(|program| program.ordered_operations.iter())
            .filter_map(|operation| match &operation.command {
                StaticMapCommand::Draw { draw } => Some((operation.operation_ordinal, draw)),
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();

        let mut source_rows = BTreeSet::new();
        let mut submissions = Vec::with_capacity(shadow_pass.ordered_draws.len());
        for draw_ref in &shadow_pass.ordered_draws {
            if !source_rows.insert(u32::from(draw_ref.bucket_id)) {
                return Err("C07 S1 tree source repeats a producer bucket row".into());
            }
            let asset = expected_s1_tree_asset(draw_ref.bucket_id)
                .ok_or("C07 S1 tree source has no authenticated asset/LOD mapping")?;
            let draw = static_draws
                .get(&draw_ref.operation_ordinal)
                .copied()
                .ok_or("C07 S1 tree source operation is absent from the submit")?;
            validate_s1_tree_material(draw, asset)?;
            let StaticGeometrySource::Exact { geometry } = &draw.geometry else {
                return Err("C07 S1 tree source does not carry exact geometry".into());
            };
            let DrawCommand::DrawIndexedInstanced {
                index_count_per_instance,
                instance_count,
                start_index,
                base_vertex,
                start_instance,
            } = &geometry.draw
            else {
                return Err("C07 S1 tree source is not indexed-instanced".into());
            };
            submissions.push(S1LightDepthSubmission::Tree {
                partition_mask: S1_PARTITION_PRIMARY,
                source_row_ordinal: u32::from(draw_ref.bucket_id),
                selected_mesh_ordinal: 0,
                selected_lod_index: asset.selected_lod_index,
                selected_mesh: asset.selected_mesh.into(),
                selected_material: asset.selected_material.into(),
                pipeline: S1ShadowPipeline::TreeShadow,
                draw: S1DrawArguments::DrawIndexedInstanced {
                    index_count_per_instance: *index_count_per_instance,
                    start_index: *start_index,
                    base_vertex: *base_vertex,
                    instance_count: *instance_count,
                    start_instance: *start_instance,
                },
                execution: RenderExecutionRef::Static {
                    operation_ordinal: draw_ref.operation_ordinal,
                },
            });
        }
        Ok(submissions)
    }

    /// The retired wire join between a carried C07 and a carried S1 carrier,
    /// re-established against the derived C07: an active S1 chain must draw
    /// exactly the derived shadow-pass operations, in order, and a tree-bearing
    /// submit may never pair with an inactive S1 skip.
    pub fn validate_s1_tree_shadow_join(&self, shadow: &S1ShadowCarrier) -> Result<(), String> {
        if shadow.is_inactive() {
            return Err("C07 tree-shadow operations conflict with an inactive S1 skip".into());
        }
        let retained_shadow = self
            .ordered_passes
            .iter()
            .find(|pass| pass.pass == C07TreePassKind::Shadow)
            .ok_or("C07 carrier has no retained shadow pass")?
            .ordered_draws
            .iter()
            .map(|draw| RenderExecutionRef::Static {
                operation_ordinal: draw.operation_ordinal,
            })
            .collect::<Vec<_>>();
        let s1_tree_shadow = shadow
            .ordered_light_depth_submissions
            .iter()
            .filter_map(|submission| match submission {
                S1LightDepthSubmission::Tree { execution, .. } => Some(execution.clone()),
                S1LightDepthSubmission::Pdxmesh { .. } => None,
            })
            .collect::<Vec<_>>();
        if retained_shadow != s1_tree_shadow {
            return Err(
                "C07 retained shadow operation order differs from S1 L03 tree submissions".into(),
            );
        }
        Ok(())
    }

    fn validate_contract(&self, requirement: C07TreeCoverageRequirement) -> Result<(), String> {
        if self.schema != C07_TREE_RUNTIME_SCHEMA
            || self.executable_sha256 != C07_TREE_A729_EXECUTABLE_SHA256
            || self.retained_contract_sha256 != C07_TREE_RETAINED_CONTRACT_SHA256
            || self.retained_rdc_sha256.len() != 64
            || !self
                .retained_rdc_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || self.producer_function_rva != C07_TREE_PRODUCER_FUNCTION_RVA
            || self.buffer_factory_callsite_rva != C07_TREE_BUFFER_FACTORY_CALLSITE_RVA
        {
            return Err("C07 tree runtime binary/evidence identity mismatch".into());
        }
        if self.coverage.kind != C07TreeCoverageKind::RetainedVisibleSubset
            || self.coverage.global_nonzero_cells != C07_TREE_GLOBAL_NONZERO_CELLS
            || self.coverage.captured_nonzero_cells + self.coverage.uncaptured_nonzero_cells
                != self.coverage.global_nonzero_cells
        {
            return Err("C07 tree retained coverage identity mismatch".into());
        }
        self.require_coverage(requirement)?;
        if self.ordered_buckets.is_empty() || self.ordered_passes.len() != EXPECTED_PASSES.len() {
            return Err("C07 tree selected bucket/pass count mismatch".into());
        }
        let library = C07TreeContentLibrary::authenticated_retained_v1_ref()?;
        library.require_complete_static_pool()?;
        let mut bucket_ids = BTreeSet::new();
        let mut captured_nonzero_cells = 0u32;
        let mut previous_global_producer_ordinal = None;
        for (ordinal, actual) in self.ordered_buckets.iter().enumerate() {
            let expected = library.require_bucket(actual.bucket_id)?;
            if !bucket_ids.insert(actual.bucket_id)
                || actual.palette_index != expected.palette_index
                || actual.chunk_x != expected.chunk_x
                || actual.chunk_z != expected.chunk_z
                || actual.producer_create_ordinal != expected.global_producer_create_ordinal
                || actual.record_count != expected.record_count
                || actual.instance_bytes != expected.instance_bytes
                || actual.instance_bytes != actual.record_count * C07_TREE_INSTANCE_STRIDE_BYTES
                || actual.instance_payload_sha256 != expected.instance_payload_sha256
                || previous_global_producer_ordinal
                    .is_some_and(|previous| actual.producer_create_ordinal <= previous)
            {
                return Err(format!("C07 tree selected bucket {ordinal} drift"));
            }
            previous_global_producer_ordinal = Some(actual.producer_create_ordinal);
            captured_nonzero_cells = captured_nonzero_cells
                .checked_add(actual.record_count)
                .ok_or("C07 tree selected cell count overflow")?;
        }
        if captured_nonzero_cells != self.coverage.captured_nonzero_cells {
            return Err("C07 tree selected coverage differs from the bucket union".into());
        }
        for (actual, (kind, vertex_sha256, pixel_sha256)) in
            self.ordered_passes.iter().zip(EXPECTED_PASSES)
        {
            if actual.pass != kind
                || actual.vertex_shader_sha256 != vertex_sha256
                || actual.pixel_shader_sha256 != pixel_sha256
                || actual.ordered_draws.is_empty()
            {
                return Err("C07 tree selected pass identity mismatch".into());
            }
            let mut pass_buckets = BTreeSet::new();
            if actual.ordered_draws.iter().any(|draw| {
                !bucket_ids.contains(&draw.bucket_id) || !pass_buckets.insert(draw.bucket_id)
            }) {
                return Err("C07 tree selected pass repeats or references an absent bucket".into());
            }
        }
        if self.payload_sha256.len() != 64
            || !self
                .payload_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || self.computed_payload_sha256()? != self.payload_sha256
        {
            return Err("C07 tree runtime payload SHA-256 mismatch".into());
        }
        library.validate_runtime_bucket_membership_trusted(self)?;
        Ok(())
    }
}

fn exact_tree_instance_source(draw: &StaticMapDraw) -> Result<&BufferSource, String> {
    let StaticGeometrySource::Exact { geometry } = &draw.geometry else {
        return Err("C07 tree draw requires exact captured geometry".into());
    };
    if !geometry.stream_output_targets.is_empty() || geometry.ordered_vertex_buffers.len() != 2 {
        return Err("C07 tree draw has invalid vertex/stream-output cardinality".into());
    }
    let instances = geometry
        .ordered_vertex_buffers
        .iter()
        .find(|binding| binding.input_slot == 1)
        .ok_or("C07 tree draw has no slot-one instance buffer")?;
    if instances.stride_bytes != C07_TREE_INSTANCE_STRIDE_BYTES || instances.byte_offset != 0 {
    }
    Ok(&instances.source)
}

fn validate_generic_draw(
    draw: &StaticMapDraw,
    bucket: &C07TreeBucketCarrier,
    expected_pass: (C07TreePassKind, &str, &str),
) -> Result<(), String> {
    if draw.shader_program.compiled_vertex_shader.sha256 != expected_pass.1
        || draw.shader_program.compiled_pixel_shader.sha256 != expected_pass.2
    {
        return Err("C07 tree draw shader identity differs from the exact pass".into());
    }
    let [color] = draw.output_state.ordered_color_targets.as_slice() else {
        return Err("C07 tree pass requires exactly one observed color target".into());
    };
    let depth = draw
        .output_state
        .depth_target
        .as_ref()
        .ok_or("C07 tree pass has no observed depth target")?;
    let expected_color_format = match expected_pass.0 {
        C07TreePassKind::Shadow => 41,
        C07TreePassKind::Unlit => 87,
        C07TreePassKind::Main => 10,
    };
    if color.width == 0
        || color.height == 0
        || color.width != depth.width
        || color.height != depth.height
        || color.dxgi_format_u32 != expected_color_format
        || depth.dxgi_format_u32 != 45
    {
        return Err(
            "C07 tree pass output-target descriptor differs from the exact pass law".into(),
        );
    }
    let StaticGeometrySource::Exact { geometry } = &draw.geometry else {
        return Err("C07 tree draw requires exact captured geometry".into());
    };
    let DrawCommand::DrawIndexedInstanced {
        instance_count,
        start_instance,
        ..
    } = &geometry.draw
    else {
        return Err("C07 tree consumer did not issue DrawIndexedInstanced".into());
    };
    if *instance_count != bucket.record_count || *start_instance != 0 {
        return Err("C07 tree draw instance range differs from its exact producer bucket".into());
    }
    let instance_source = exact_tree_instance_source(draw)?;
    if !source_matches(
        instance_source,
        bucket.instance_bytes,
        &bucket.instance_payload_sha256,
    ) {
        return Err("C07 tree draw slot-one payload differs from its exact producer bucket".into());
    }
    let mesh = geometry
        .ordered_vertex_buffers
        .iter()
        .find(|binding| binding.input_slot == 0)
        .ok_or("C07 tree draw has no observed slot-zero model/LOD mesh")?;
    if mesh.byte_offset != 0 || mesh.stride_bytes == 0 {
        return Err("C07 tree model/LOD mesh binding is not an exact base stream".into());
    }
    let index = geometry
        .index_buffer
        .as_ref()
        .ok_or("C07 tree draw has no observed model/LOD index buffer")?;
    if index.format != IndexFormat::R16Uint || index.byte_offset != 0 {
        return Err("C07 tree model/LOD index binding differs from the exact consumer".into());
    }
    Ok(())
}

fn validate_draw(
    draw: &StaticMapDraw,
    bucket: &C07TreeBucketCarrier,
    expected: &ExpectedDraw,
    expected_pass: (C07TreePassKind, &str, &str),
) -> Result<(), String> {
    if draw.shader_program.compiled_vertex_shader.sha256 != expected_pass.1
        || draw.shader_program.compiled_pixel_shader.sha256 != expected_pass.2
    {
        return Err("C07 tree draw shader identity differs from retained pass".into());
    }
    let (color_width, color_height, color_format, depth_format) = match expected_pass.0 {
        C07TreePassKind::Shadow => (2_048, 2_048, 41, 45),
        C07TreePassKind::Unlit => (1_920, 1_080, 87, 45),
        C07TreePassKind::Main => (1_920, 1_080, 10, 45),
    };
    let [color] = draw.output_state.ordered_color_targets.as_slice() else {
        return Err("C07 tree retained pass requires exactly one color target".into());
    };
    let depth = draw
        .output_state
        .depth_target
        .as_ref()
        .ok_or("C07 tree retained pass has no depth target")?;
    if color.width != color_width
        || color.height != color_height
        || color.dxgi_format_u32 != color_format
        || depth.width != color_width
        || depth.height != color_height
        || depth.dxgi_format_u32 != depth_format
    {
        return Err("C07 tree retained pass output-target identity mismatch".into());
    }
    let StaticGeometrySource::Exact { geometry } = &draw.geometry else {
        return Err("C07 tree draw requires exact captured geometry".into());
    };
    if !geometry.stream_output_targets.is_empty() {
        return Err("C07 tree draw unexpectedly binds stream output".into());
    }
    let DrawCommand::DrawIndexedInstanced {
        index_count_per_instance,
        instance_count,
        ..
    } = &geometry.draw
    else {
        return Err("C07 tree carrier references a non-indexed-instanced draw".into());
    };
    if *index_count_per_instance != expected.index_count
        || *instance_count != expected.instance_count
        || *instance_count != bucket.record_count
    {
        return Err("C07 tree draw count differs from retained mesh/bucket contract".into());
    }
    if geometry.ordered_vertex_buffers.len() != 2 {
        return Err("C07 tree draw does not carry exactly mesh and instance streams".into());
    }
    let mesh = geometry
        .ordered_vertex_buffers
        .iter()
        .find(|binding| binding.input_slot == 0)
        .ok_or("C07 tree draw has no slot-zero mesh buffer")?;
    let instances = geometry
        .ordered_vertex_buffers
        .iter()
        .find(|binding| binding.input_slot == 1)
        .ok_or("C07 tree draw has no slot-one instance buffer")?;
    if mesh.byte_offset != 0
        || !source_matches(
            &mesh.source,
            expected.mesh_vb_bytes,
            expected.mesh_vb_sha256,
        )
        || instances.stride_bytes != C07_TREE_INSTANCE_STRIDE_BYTES
        || instances.byte_offset != 0
        || !source_matches(
            &instances.source,
            bucket.instance_bytes,
            &bucket.instance_payload_sha256,
        )
    {
        return Err("C07 tree vertex/instance buffer identity mismatch".into());
    }
    let index = geometry
        .index_buffer
        .as_ref()
        .ok_or("C07 tree draw has no index buffer")?;
    if index.format != IndexFormat::R16Uint
        || index.byte_offset != 0
        || !source_matches(
            &index.source,
            expected.mesh_ib_bytes,
            expected.mesh_ib_sha256,
        )
    {
        return Err("C07 tree index-buffer identity mismatch".into());
    }
    Ok(())
}

fn source_matches(source: &BufferSource, expected_bytes: u32, expected_sha256: &str) -> bool {
    match source {
        BufferSource::Inline { bytes, sha256 } => {
            bytes.len() == expected_bytes as usize
                && sha256 == expected_sha256
                && format!("{:x}", Sha256::digest(bytes)) == expected_sha256
        }
        BufferSource::UsedContentRange {
            byte_count,
            range_sha256,
            ..
        } => *byte_count == u64::from(expected_bytes) && range_sha256 == expected_sha256,
        BufferSource::ParticleInstanceRecords => false,
    }
}

fn validate_s1_tree_material(
    draw: &StaticMapDraw,
    asset: ExpectedS1TreeAsset,
) -> Result<(), String> {
    for (expected_path, expected_sha256) in asset.material_textures {
        let matches = draw
            .ordered_shader_resources
            .iter()
            .filter(|binding| match &binding.source {
                StaticShaderResourceSource::UsedContent { content, .. } => {
                    content.normalized_game_relative_path == *expected_path
                        && content.payload_sha256 == *expected_sha256
                }
                _ => false,
            })
            .count();
        if matches != 1 {
            return Err(format!(
                "C07 S1 tree material does not uniquely bind {expected_path} at its admitted hash"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame36669_carrier() -> C07TreeRuntimeCarrier {
        C07TreeRuntimeCarrier::retained_frame36669([
            [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
            [14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26],
            [27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39],
        ])
        .unwrap()
    }

    #[test]
    fn complete_content_library_has_exact_identity_and_no_fabricated_remainder() {
        let library = C07TreeContentLibrary::authenticated_retained_v1().unwrap();
        assert_eq!(
            library.payload_sha256,
            C07_TREE_CONTENT_LIBRARY_PAYLOAD_SHA256
        );
        assert_eq!(library.ordered_buckets.len(), 296);
        assert_eq!(library.authenticated_cells, 99_056);
        assert_eq!(library.unavailable_cells, 0);
        assert_eq!(library.unavailable_buckets, 0);
        assert_eq!(library.packed_instance_bytes.len(), 2_377_344);
        assert_eq!(
            library
                .ordered_buckets
                .first()
                .unwrap()
                .global_producer_create_ordinal,
            0
        );
        assert_eq!(
            library
                .ordered_buckets
                .last()
                .unwrap()
                .global_producer_create_ordinal,
            295
        );
        assert_eq!(
            format!("{:x}", Sha256::digest(&library.packed_instance_bytes)),
            C07_TREE_CONTENT_LIBRARY_PACKED_BYTES_SHA256
        );
        assert!(library.require_bucket(379).is_ok());
        assert!(library.require_bucket(1426).is_ok());
        assert!(library.require_bucket(354).is_ok());
        assert_eq!(library.bucket_payload(379).unwrap().len(), 120);
        assert!(library
            .admit_bucket_source(
                379,
                &BufferSource::Inline {
                    bytes: vec![0; 120],
                    sha256: "898f804f92ee8ff565700e123238a241684496a9d8f935bdc36ea7ecdf44ecaa"
                        .into(),
                },
            )
            .is_err());
        assert!(library.require_cell_ordinal(25_995).is_ok());
        assert!(library.require_cell_ordinal(61_972).is_ok());
        assert!(library.require_cell_ordinal(0).is_ok());
        assert!(library.require_cell_ordinal(61_973).is_ok());
        assert!(library.require_cell_ordinal(99_056).is_err());
        assert!(library.require_complete_static_pool().is_ok());
    }

    #[test]
    fn library_rejects_resealed_bucket_or_coverage_drift() {
        let mut library = C07TreeContentLibrary::authenticated_retained_v1().unwrap();
        library.ordered_buckets[0].record_count += 1;
        library.ordered_buckets[0].instance_bytes += C07_TREE_INSTANCE_STRIDE_BYTES;
        library.seal_payload_sha256().unwrap();
        assert!(library.validate().is_err());

        let mut library = C07TreeContentLibrary::authenticated_retained_v1().unwrap();
        library.authenticated_cell_ordinal_ranges[0].first_ordinal += 1;
        library.seal_payload_sha256().unwrap();
        assert!(library.validate().is_err());

        let mut library = C07TreeContentLibrary::authenticated_retained_v1().unwrap();
        library.packed_instance_bytes[0] ^= 1;
        library.seal_payload_sha256().unwrap();
        assert!(library.validate().is_err());
    }

    #[test]
    fn selected_submit_subset_never_expands_to_complete_static_pool() {
        let library = C07TreeContentLibrary::authenticated_retained_v1().unwrap();
        let carrier = frame36669_carrier();
        carrier
            .validate_contract(C07TreeCoverageRequirement::RetainedVisibleSubset)
            .unwrap();
        assert_eq!(C07_TREE_UNCAPTURED_CELLS, 94_275);
        assert_eq!(carrier.coverage.uncaptured_nonzero_cells, 94_275);
        library
            .validate_runtime_bucket_membership(&carrier)
            .unwrap();
        assert!(carrier.ordered_buckets.iter().all(|selected| {
            selected.producer_create_ordinal
                == library
                    .require_bucket(selected.bucket_id)
                    .unwrap()
                    .global_producer_create_ordinal
        }));
        let selection = C07TreeContentSelection::from_validated_subset(&library, &carrier).unwrap();
        assert_eq!(
            selection.ordered_bucket_ids.len(),
            C07_TREE_RETAINED_BUCKETS
        );
        assert_eq!(selection.selected_cells, C07_TREE_RETAINED_VISIBLE_CELLS);
        assert_eq!(selection.unselected_library_cells, 94_275);
        assert_eq!(selection.globally_unavailable_cells, 0);
        assert_eq!(
            selection.ordered_bucket_ids,
            vec![634, 1144, 1145, 1146, 1166, 1167, 1168, 1400, 1401, 1402, 1422, 1423, 1424]
        );
    }

    fn tree_target(width: u32, height: u32, format: u32, ordinal: u32) -> crate::RenderTargetDesc {
        crate::RenderTargetDesc {
            prepared_target: crate::PreparedRenderTarget::OpaqueRenderTarget {
                attachment_ordinal: ordinal,
            },
            width,
            height,
            dxgi_format_u32: format,
            mip_level: 0,
            sample_count: 1,
            sample_quality: 0,
            persistent_content: None,
            exact_resource: None,
        }
    }

    fn tree_shader(vertex_sha256: &str, pixel_sha256: &str) -> crate::ShaderProgram {
        let compiled = |sha256: &str, profile: &str| crate::CompiledShaderPayload {
            compiler_identity: "fxc-10.1".into(),
            target_profile: profile.into(),
            compiler_flags_u64: 0,
            bytecode: vec![1, 2, 3, 4],
            sha256: sha256.into(),
        };
        crate::ShaderProgram {
            source: crate::UsedContentKey {
                effective_vfs_manifest_sha256: "2".repeat(64),
                normalized_game_relative_path: "gfx/fx/tree.shader".into(),
                winning_layer_ordinal: 0,
                layer_kind: portable_content_manifest::ContentLayerKind::Base,
                layer_id: "base".into(),
                source_sha256: "c".repeat(64),
                payload_sha256: "c".repeat(64),
                payload_bytes: 1,
            },
            producer_registration_root_rva: None,
            effect_token: "PdxMeshAdvancedSnow".into(),
            vertex_entry_token: "VS".into(),
            pixel_entry_token: "PS".into(),
            ordered_defines: vec![],
            compiled_vertex_shader: compiled(vertex_sha256, "vs_5_0"),
            compiled_pixel_shader: compiled(pixel_sha256, "ps_5_0"),
            geometry_shader: None,
            hull_shader: None,
            domain_shader: None,
            compute_shader: None,
        }
    }

    fn submit_with_generic_tree_draws(carrier: &C07TreeRuntimeCarrier) -> RenderSubmit {
        let library = C07TreeContentLibrary::authenticated_retained_v1().unwrap();
        let buckets = carrier
            .ordered_buckets
            .iter()
            .map(|bucket| (bucket.bucket_id, bucket))
            .collect::<BTreeMap<_, _>>();
        let mesh_bytes = vec![9u8; 64];
        let index_bytes = vec![7u8; 32];
        let mut operations = Vec::new();
        let mut executions = Vec::new();
        for (pass, expected_pass) in carrier.ordered_passes.iter().zip(EXPECTED_PASSES) {
            let (width, height, color_format) = match expected_pass.0 {
                C07TreePassKind::Shadow => (2_048, 2_048, 41),
                C07TreePassKind::Unlit => (1_920, 1_080, 87),
                C07TreePassKind::Main => (1_920, 1_080, 10),
            };
            for draw_ref in &pass.ordered_draws {
                let bucket = buckets[&draw_ref.bucket_id];
                let instance_bytes = library.bucket_payload(bucket.bucket_id).unwrap().to_vec();
                let draw = StaticMapDraw {
                    draw_token: format!("c07.tree.{}", draw_ref.bucket_id),
                    shader_program: tree_shader(expected_pass.1, expected_pass.2),
                    ordered_shader_resources: vec![],
                    pipeline: crate::tests::shared_test_pipeline(),
                    geometry: StaticGeometrySource::Exact {
                        geometry: crate::GeometryBinding {
                            ordered_vertex_buffers: vec![
                                crate::VertexBufferBinding {
                                    input_slot: 0,
                                    stride_bytes: 32,
                                    byte_offset: 0,
                                    source: BufferSource::Inline {
                                        sha256: format!("{:x}", Sha256::digest(&mesh_bytes)),
                                        bytes: mesh_bytes.clone(),
                                    },
                                },
                                crate::VertexBufferBinding {
                                    input_slot: 1,
                                    stride_bytes: C07_TREE_INSTANCE_STRIDE_BYTES,
                                    byte_offset: 0,
                                    source: BufferSource::Inline {
                                        sha256: bucket.instance_payload_sha256.clone(),
                                        bytes: instance_bytes,
                                    },
                                },
                            ],
                            index_buffer: Some(crate::IndexBufferBinding {
                                format: IndexFormat::R16Uint,
                                byte_offset: 0,
                                source: BufferSource::Inline {
                                    sha256: format!("{:x}", Sha256::digest(&index_bytes)),
                                    bytes: index_bytes.clone(),
                                },
                            }),
                            stream_output_targets: vec![],
                            draw: DrawCommand::DrawIndexedInstanced {
                                index_count_per_instance: 12,
                                instance_count: bucket.record_count,
                                start_index: 0,
                                base_vertex: 0,
                                start_instance: 0,
                            },
                        },
                    },
                    ordered_constant_buffers: vec![],
                    output_state: crate::RenderOutputState {
                        ordered_color_targets: vec![tree_target(width, height, color_format, 1)],
                        depth_target: Some(tree_target(width, height, 45, 2)),
                        viewport: crate::ViewportDesc {
                            top_left_x_f32_bits: 0,
                            top_left_y_f32_bits: 0,
                            width_f32_bits: (width as f32).to_bits(),
                            height_f32_bits: (height as f32).to_bits(),
                            min_depth_f32_bits: 0.0f32.to_bits(),
                            max_depth_f32_bits: 1.0f32.to_bits(),
                        },
                        scissor: None,
                    },
                    payload_sha256: String::new(),
                };
                operations.push(crate::StaticMapOperation {
                    operation_ordinal: draw_ref.operation_ordinal,
                    payload_sha256: String::new(),
                    command: StaticMapCommand::Draw { draw },
                });
                executions.push(RenderExecutionRef::Static {
                    operation_ordinal: draw_ref.operation_ordinal,
                });
            }
        }
        operations.sort_by_key(|operation| operation.operation_ordinal);
        executions.sort_by_key(|execution| match execution {
            RenderExecutionRef::Static { operation_ordinal } => *operation_ordinal,
            _ => unreachable!(),
        });
        RenderSubmit {
            global_order: 1,
            camera_carrier_ordinal: 0,
            camera_carrier: None,
            ordered_static_map_programs: vec![crate::StaticMapPassProgram {
                program_ordinal: 0,
                pass_token: format!("opaque-pass:{}", "a".repeat(64)),
                ordered_content_keys: vec![],
                ordered_operations: operations,
                payload_sha256: String::new(),
            }],
            authenticated_texture_snapshots: vec![],
            s1_shadow: None,
            particles: vec![],
            particle_packet_set_sha256: "5".repeat(64),
            entities: vec![],
            rail_construction_arrows: vec![],
            ordered_render_operations: executions,
            tiled_lights: vec![],
            weather_membership: vec![],
            political_textures: vec![],
        }
    }

    #[test]
    fn pinned_frame36669_carrier_enforces_the_pinned_geometry_law() {
        let pinned = frame36669_carrier();
        assert_eq!(pinned.retained_rdc_sha256, C07_TREE_RETAINED_RDC_SHA256);
        let submit = submit_with_generic_tree_draws(&pinned);

        let error = pinned
            .validate_against_submit(&submit, C07TreeCoverageRequirement::RetainedVisibleSubset)
            .unwrap_err();
        assert_eq!(
            error,
            "C07 tree draw count differs from retained mesh/bucket contract"
        );

        let mut counted = submit.clone();
        for operation in &mut counted.ordered_static_map_programs[0].ordered_operations {
            let StaticMapCommand::Draw { draw } = &mut operation.command else {
                unreachable!()
            };
            let bucket_id = pinned
                .ordered_passes
                .iter()
                .flat_map(|pass| pass.ordered_draws.iter())
                .find(|row| row.operation_ordinal == operation.operation_ordinal)
                .unwrap()
                .bucket_id;
            let expected = EXPECTED_DRAW_ORDER
                .iter()
                .find(|row| row.bucket_id == bucket_id)
                .unwrap();
            let StaticGeometrySource::Exact { geometry } = &mut draw.geometry else {
                unreachable!()
            };
            let DrawCommand::DrawIndexedInstanced {
                index_count_per_instance,
                ..
            } = &mut geometry.draw
            else {
                unreachable!()
            };
            *index_count_per_instance = expected.index_count;
        }
        assert_eq!(
            pinned
                .validate_against_submit(
                    &counted,
                    C07TreeCoverageRequirement::RetainedVisibleSubset
                )
                .unwrap_err(),
            "C07 tree vertex/instance buffer identity mismatch"
        );

        let mut generic = pinned.clone();
        generic.retained_rdc_sha256 =
            C07TreeRuntimeCarrier::selected_submit_hash_projection(&submit).unwrap();
        generic.seal_payload_sha256().unwrap();
        generic
            .validate_against_submit(&submit, C07TreeCoverageRequirement::RetainedVisibleSubset)
            .unwrap();
    }

    #[test]
    fn derived_tree_runtime_reproduces_the_retired_carried_carrier() {
        // kills: RenderSubmit.tree_runtime used to ride the wire verbatim.
        // The capture-side producer (collector fold_a729_static_program_capture)
        // stored exactly C07TreeRuntimeCarrier::from_authenticated_selected_submit
        // over the submit as it stood at static-fold time — before the later S1
        // and weather folds mutated it. This test derives from that pre-fold
        // state (the previously carried construction, verbatim), then folds S1
        // and weather on top exactly as the drain did and requires the
        // replay-side derivation of the folded state to equal it field-for-field.
        // It also fails closed instead of sealing a partial tree family, and
        // preserves the retained visible-subset coverage law against the pinned
        // carrier.
        let pinned = frame36669_carrier();
        let mut pre_fold = submit_with_generic_tree_draws(&pinned);

        let carried =
            C07TreeRuntimeCarrier::from_authenticated_selected_submit(&pre_fold)
                .unwrap()
                .expect("sealed fixture carries static tree draws");
        assert_eq!(
            carried.coverage,
            pinned.coverage,
            "derivation preserves the retained visible-subset coverage law"
        );

        pre_fold.s1_shadow = Some(crate::S1ShadowCarrier {
            schema: crate::S1_SHADOW_SCHEMA.into(),
            executable_sha256: String::new(),
            l02_authority_sha256: String::new(),
            l03_authority_sha256: String::new(),
            s1_chain_authority_sha256: String::new(),
            shadow_pass_active: false,
            blur_camera_factor_f32_bits: 0,
            selected_light: None,
            light_view_projection_f32_bits: None,
            shadow_map_texture_matrix_f32_bits: None,
            camera_y_f32_bits: None,
            enabled_partition_mask: None,
            light_depth_target: None,
            producer_and_final_target: None,
            blur_intermediate_target: None,
            light_depth_clear: None,
            ordered_light_depth_submissions: Vec::new(),
            ordered_terrain_unlit_draws: Vec::new(),
            horizontal_blur_draw: None,
            vertical_blur_draw: None,
            ordered_shadow_consumers: Vec::new(),
            payload_sha256: String::new(),
        });
        pre_fold.weather_membership = vec![weather_membership_fixture()];
        let replay_derived = C07TreeRuntimeCarrier::from_authenticated_selected_submit(&pre_fold)
            .unwrap()
            .unwrap();
        assert_eq!(
            replay_derived, carried,
            "replay-side derivation must reproduce the previously carried carrier across the S1 and weather folds"
        );
        let derived = replay_derived;

        let mut programless = pre_fold.clone();
        programless.ordered_render_operations.clear();
        assert!(C07TreeRuntimeCarrier::from_authenticated_selected_submit(&programless)
            .unwrap()
            .is_none());

        let shadow_ordinals: Vec<u32> = derived
            .ordered_passes
            .iter()
            .find(|pass| pass.pass == C07TreePassKind::Shadow)
            .unwrap()
            .ordered_draws
            .iter()
            .map(|draw| draw.operation_ordinal)
            .collect();
        let mut halved = pre_fold.clone();
        halved.ordered_render_operations.retain(|execution| {
            !matches!(execution, RenderExecutionRef::Static { operation_ordinal }
                if shadow_ordinals.contains(operation_ordinal))
        });
        assert_eq!(
            C07TreeRuntimeCarrier::from_authenticated_selected_submit(&halved).unwrap_err(),
            "C07 selected submit contains an incomplete tree pass family"
        );
    }

    #[test]
    fn clear_only_selected_executions_derive_no_tree_content_and_dangling_refs_still_refuse() {
        // kills: a selected execution of a NON-draw static operation (target
        // clear and similar) is executed static work, not tree content - the
        // derivation must skip it exactly like a draw whose shader pair is not
        // a tree pass, so a legitimate clear-only submit seals without a C07
        // carrier instead of being refused. A reference to an operation
        // ordinal that exists nowhere in the static pool still fails closed
        // with the same named refusal as before.
        let submit = RenderSubmit {
            global_order: 1,
            camera_carrier_ordinal: 0,
            camera_carrier: None,
            ordered_static_map_programs: vec![crate::StaticMapPassProgram {
                program_ordinal: 0,
                pass_token: "exact-target-clear".into(),
                ordered_content_keys: vec![],
                ordered_operations: vec![crate::StaticMapOperation {
                    operation_ordinal: 7,
                    payload_sha256: String::new(),
                    command: StaticMapCommand::ClearColor {
                        target: tree_target(64, 64, 10, 0),
                        rgba_f32_bits: [0; 4],
                    },
                }],
                payload_sha256: String::new(),
            }],
            authenticated_texture_snapshots: vec![],
            s1_shadow: None,
            particles: vec![],
            particle_packet_set_sha256: "5".repeat(64),
            entities: vec![],
            rail_construction_arrows: vec![],
            ordered_render_operations: vec![RenderExecutionRef::Static {
                operation_ordinal: 7,
            }],
            tiled_lights: vec![],
            weather_membership: vec![],
            political_textures: vec![],
        };
        assert!(
            C07TreeRuntimeCarrier::from_authenticated_selected_submit(&submit)
                .unwrap()
                .is_none()
        );
        let mut dangling = submit.clone();
        dangling.ordered_render_operations[0] =
            RenderExecutionRef::Static { operation_ordinal: 9 };
        assert_eq!(
            C07TreeRuntimeCarrier::from_authenticated_selected_submit(&dangling).unwrap_err(),
            "C07 selected execution references an absent static draw"
        );
    }

    fn weather_membership_fixture() -> crate::WeatherMembership {
        crate::WeatherMembership {
            sequence: 1,
            update_index: 0,
            strategic_region_id: 7,
            rain_light_active: false,
            rain_heavy_active: false,
            snow_active: false,
            blizzard_active: false,
            sandstorm_active: false,
            arctic_water_active: false,
            region_value_i64_fixed_1e5: 0,
            transition_expiry_game_hour_i32: 0,
            airborne_runtime_code: 0,
            selection_seed_words: None,
            visual_epoch: None,
            weather_effect_token: None,
            weather_position_source_content: crate::UsedContentKey {
                effective_vfs_manifest_sha256: "2".repeat(64),
                normalized_game_relative_path: "gfx/weather/weather.system".into(),
                winning_layer_ordinal: 0,
                layer_kind: portable_content_manifest::ContentLayerKind::Base,
                layer_id: "base".into(),
                source_sha256: "c".repeat(64),
                payload_sha256: "c".repeat(64),
                payload_bytes: 1,
            },
            ordered_variant_content: Vec::new(),
            ordered_positions: Vec::new(),
            ordered_weather_instances: Vec::new(),
            visibility_updates: Vec::new(),
            membership_sha256: String::new(),
        }
    }

    #[test]
    fn s1_tree_shadow_join_reproduces_the_retired_wire_check() {
        // kills: RenderSubmit wire validation used to refuse an S1 carrier whose
        // L03 tree submissions diverged from the C07 retained shadow pass, and
        // refused tree-shadow operations paired with an inactive S1 skip. With
        // the C07 derived instead of carried, the join must hold between the
        // derived carrier and the carried S1 carrier with identical strength.
        let pinned = frame36669_carrier();
        let submit = submit_with_generic_tree_draws(&pinned);
        let derived = C07TreeRuntimeCarrier::from_authenticated_selected_submit(&submit)
            .unwrap()
            .unwrap();

        let shadow_pass = derived
            .ordered_passes
            .iter()
            .find(|pass| pass.pass == C07TreePassKind::Shadow)
            .unwrap();
        let submissions = shadow_pass
            .ordered_draws
            .iter()
            .map(|draw_ref| {
                S1LightDepthSubmission::Tree {
                    partition_mask: S1_PARTITION_PRIMARY,
                    source_row_ordinal: u32::from(draw_ref.bucket_id),
                    selected_mesh_ordinal: 0,
                    selected_lod_index: 0,
                    selected_mesh: "gfx/models/tree.mesh".into(),
                    selected_material: "tree_shadow".into(),
                    pipeline: S1ShadowPipeline::TreeShadow,
                    draw: S1DrawArguments::DrawIndexedInstanced {
                        index_count_per_instance: 12,
                        start_index: 0,
                        base_vertex: 0,
                        instance_count: 1,
                        start_instance: 0,
                    },
                    execution: RenderExecutionRef::Static {
                        operation_ordinal: draw_ref.operation_ordinal,
                    },
                }
            })
            .collect::<Vec<_>>();

        let mut shadow = crate::S1ShadowCarrier {
            schema: crate::S1_SHADOW_SCHEMA.into(),
            executable_sha256: String::new(),
            l02_authority_sha256: String::new(),
            l03_authority_sha256: String::new(),
            s1_chain_authority_sha256: String::new(),
            shadow_pass_active: true,
            blur_camera_factor_f32_bits: 0,
            selected_light: None,
            light_view_projection_f32_bits: None,
            shadow_map_texture_matrix_f32_bits: None,
            camera_y_f32_bits: None,
            enabled_partition_mask: None,
            light_depth_target: None,
            producer_and_final_target: None,
            blur_intermediate_target: None,
            light_depth_clear: None,
            ordered_light_depth_submissions: submissions.clone(),
            ordered_terrain_unlit_draws: Vec::new(),
            horizontal_blur_draw: None,
            vertical_blur_draw: None,
            ordered_shadow_consumers: Vec::new(),
            payload_sha256: String::new(),
        };
        derived.validate_s1_tree_shadow_join(&shadow).unwrap();

        let mut reordered = shadow.clone();
        let last = reordered.ordered_light_depth_submissions.pop().unwrap();
        reordered
            .ordered_light_depth_submissions
            .insert(0, last);
        assert_eq!(
            derived
                .validate_s1_tree_shadow_join(&reordered)
                .unwrap_err(),
            "C07 retained shadow operation order differs from S1 L03 tree submissions"
        );

        shadow.shadow_pass_active = false;
        assert_eq!(
            derived.validate_s1_tree_shadow_join(&shadow).unwrap_err(),
            "C07 tree-shadow operations conflict with an inactive S1 skip"
        );
    }

    #[test]
    fn s1_admission_enforces_the_derived_c07_tree_shadow_join() {
        // kills: the retired wire check ran inside RenderSubmit validation; it
        // now rides S1 admission itself, so every path that accepts a carried
        // S1 carrier pays it. Deleting the join inside
        // S1ShadowCarrier::validate_against_submit fails this test: a
        // tree-bearing submit paired with an inactive S1 skip must be refused
        // exactly as the old wire validator refused it.
        let pinned = frame36669_carrier();
        let mut submit = submit_with_generic_tree_draws(&pinned);
        submit.s1_shadow = Some(sealed_inactive_s1());
        assert_eq!(
            submit
                .s1_shadow
                .as_ref()
                .unwrap()
                .validate_against_submit(&submit)
                .unwrap_err(),
            "C07 tree-shadow operations conflict with an inactive S1 skip"
        );
    }

    fn sealed_inactive_s1() -> crate::S1ShadowCarrier {
        let mut carrier = crate::S1ShadowCarrier {
            schema: crate::S1_SHADOW_SCHEMA.into(),
            executable_sha256: crate::S1_A729_EXECUTABLE_SHA256.into(),
            l02_authority_sha256: crate::S1_L02_AUTHORITY_SHA256.into(),
            l03_authority_sha256: crate::S1_L03_AUTHORITY_SHA256.into(),
            s1_chain_authority_sha256: crate::S1_CHAIN_AUTHORITY_SHA256.into(),
            shadow_pass_active: false,
            blur_camera_factor_f32_bits: 0,
            selected_light: None,
            light_view_projection_f32_bits: None,
            shadow_map_texture_matrix_f32_bits: None,
            camera_y_f32_bits: None,
            enabled_partition_mask: None,
            light_depth_target: None,
            producer_and_final_target: None,
            blur_intermediate_target: None,
            light_depth_clear: None,
            ordered_light_depth_submissions: Vec::new(),
            ordered_terrain_unlit_draws: Vec::new(),
            horizontal_blur_draw: None,
            vertical_blur_draw: None,
            ordered_shadow_consumers: Vec::new(),
            payload_sha256: String::new(),
        };
        carrier.payload_sha256 = carrier.computed_payload_sha256().unwrap();
        carrier
    }

    #[test]
    fn portable_library_roundtrip_preserves_exact_admission() {
        let library = C07TreeContentLibrary::authenticated_retained_v1().unwrap();
        let bytes = serde_json::to_vec(&library).unwrap();
        let decoded: C07TreeContentLibrary = serde_json::from_slice(&bytes).unwrap();
        decoded.validate().unwrap();
        assert_eq!(decoded, library);
    }
}
