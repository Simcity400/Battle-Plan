# Wave 3 exact-20 job and consumer binding ledger

Date: 2026-07-20

Scoped jobs: **56**; complete: **50**; failed: **6**; running: **0**.

This ledger admits static exact-20 evidence custody only. It does not enable a live source, satisfy a runtime fixture, or change a lane activation latch.

No standalone preflight JSON was persisted before dispatch. The pool receipt is an explicitly reconstructed postflight snapshot, and every consumer gate that requires a persisted preflight remains false.

| # | Group | Job | Status | Operation | Shard | Members | Artifact SHA-256 | Relationship |
|---:|---|---|---|---|---|---:|---|---|
| 1 | `A18_DYNAMIC_MODIFIERS` | `20260720T020812Z-w3-a18-dynmod-5b64758d` | `complete` | `export_functions` | `shard-06` | 10 | `7a58d944c59e95a6a7edac5ec3af8b6b8deda13db38eaf3c5589e6f287ffc489` | `authoritative_success` |
| 2 | `A28_COLLECTION_CONTROLLERS` | `20260720T020812Z-w3-a28-air-mio-fba35b10` | `failed` | `export_functions` | `shard-18` | 23 | `none` | `failed_superseded_by_depth0_retry_and_segmented_depth1_attempts` |
| 3 | `A31_FOCUS_OWNER` | `20260720T020812Z-w3-a31-focus-owner-93fbbf62` | `complete` | `export_functions` | `shard-17` | 10 | `303351ed65468aa7655b904998819c96ca1c325f906b5a3bf32c4f18decc7969` | `authoritative_success` |
| 4 | `A33_SUNK_SHIP_HISTORY` | `20260720T020812Z-w3-a33-sunk-ship-4aecc451` | `complete` | `export_functions` | `shard-13` | 10 | `37797c3175af53fc483a26555a492a65ac926b815eb4abd650ff01a9c3d5487e` | `authoritative_success` |
| 5 | `A34_DECISION_PANEL` | `20260720T020812Z-w3-a34-decision-panel-7be8b60f` | `complete` | `export_functions` | `shard-20` | 31 | `2341981c14983a46bafe8c1be0fa69272a9eb9a33c58d4c8ddef2bd1617a1a8e` | `authoritative_success` |
| 6 | `A36_COMMAND_ABILITIES` | `20260720T020812Z-w3-a36-ability-list-feb98226` | `complete` | `export_functions` | `shard-19` | 16 | `12768dfa9a8b47ceda4feae9cdc45b76811f602ecd557008aa936e95adf2db45` | `authoritative_success` |
| 7 | `A37_POWER_BALANCE` | `20260720T020812Z-w3-a37-power-balance-5c5d63f5` | `complete` | `export_functions` | `shard-11` | 10 | `a540af1c030d9ef2fbb1cd290627087627771a196e7a60e4241cb5287b66e4e2` | `authoritative_success` |
| 8 | `A38_SCRIPTED_GUI` | `20260720T020812Z-w3-a38-controller-owner-41ee5208` | `complete` | `export_functions` | `shard-01` | 15 | `7d0a36b1822faf85a949d628a79bc4a09f95dc9c9aa3ed4aecb715b72b0a9082` | `authoritative_success` |
| 9 | `A40_PEACE_CONFERENCE` | `20260720T020812Z-w3-a40-peace-anchors-06e5895a` | `complete` | `export_functions` | `shard-09` | 9 | `f04183c9803c44fa0e5ebcd4d7a8ce279b06f57b4429e4d03118435df3792fcf` | `authoritative_success` |
| 10 | `A40_PEACE_CONFERENCE` | `20260720T020812Z-w3-a40-peace-strings-89672ebe` | `complete` | `search_strings` | `shard-14` | 15 | `7018fa7180ee84d57160d5e9d0626b9847080f0a781483eb4fcabe81433fe0bf` | `supplemental_success` |
| 11 | `B5_WAR_RESIDUALS` | `20260720T020812Z-w3-b5-war-residual-b9f9ff7a` | `complete` | `export_functions` | `shard-12` | 8 | `193c2aa522d334594c688ba57e15e65e92f1f8ee8ef9d40b17a06b8613487dd6` | `authoritative_success` |
| 12 | `SUPPLY_ROOTS` | `20260720T020812Z-w3-supply-roots-2f6d1ec1` | `complete` | `export_functions` | `shard-07` | 11 | `3f346cfcaf37895c347002fcfbd0782bd15672342aa6ba3da56f2396016108fd` | `authoritative_success` |
| 13 | `W15_CAMPAIGN_IDENTITY` | `20260720T020812Z-w3-w15-cid1-helpers-0935bd0c` | `complete` | `export_functions` | `shard-05` | 2 | `b8f20c26ddc03fd721a87e551afccdfb66d48aee80521bdebdf60597326e79a1` | `supplemental_success` |
| 14 | `WEATHER_SETTER_CHAIN` | `20260720T020812Z-w3-weather-setters-a042b9e8` | `complete` | `export_functions` | `shard-02` | 5 | `17cf975de21495b4689c6aab208a7606b9bee9b09aab93cd89b48d42960538b1` | `authoritative_success` |
| 15 | `X2_CURRENT_LOSSES` | `20260720T020812Z-w3-x2-loss-owner-5246af02` | `complete` | `export_functions` | `shard-08` | 14 | `fe48d284c6c70452d5afa31cf982d8646a3cb0a9ede11781c986d6fc087d5645` | `authoritative_success` |
| 16 | `X8_MULTI_ORDER_MEMBERSHIP` | `20260720T020812Z-w3-x8-multi-order-91af7599` | `complete` | `export_functions` | `shard-10` | 10 | `0a4a87781a0dbe7c5ee425e10502cf36c1474d8bc2a670b9dde64c10a745f764` | `authoritative_success` |
| 17 | `X9_SPECIALIZATIONS` | `20260720T020812Z-w3-x9-specialization-8ca04933` | `complete` | `export_functions` | `shard-03` | 5 | `7819eda447c5be7dc1359842cdcad53fb7da1c2d44ec009d1326ab61130c9302` | `authoritative_success` |
| 18 | `A40_DESIGNER_IDENTITY` | `20260720T020813Z-w3-a40-designers-fc2ae305` | `complete` | `export_functions` | `shard-04` | 10 | `6dff47e4fae1f9d3aeabb63ac7e9a9cc13a27c429d5e217d686311d5a349cb6b` | `authoritative_success` |
| 19 | `A40_PEACE_CONFERENCE` | `20260720T020813Z-w3-a40-peace-graph-ef209889` | `complete` | `export_functions` | `shard-16` | 23 | `5e4b593671d9b9bd1c48d662ba592f1e9fb59a70eac2ca87a8bf677484b156e4` | `authoritative_success` |
| 20 | `DIRECT_FOCUS_RESEARCH_DECISION_HOOKS` | `20260720T020813Z-w3-focus-direct-hooks-d987b89a` | `complete` | `export_functions` | `shard-15` | 6 | `a9855541571204160bc4ac78dcf6f1dbdfa112496234e84540406a71c37d2fb3` | `authoritative_success` |
| 21 | `A16_SUBJECT_AUTONOMY` | `20260720T021840Z-w3-a16-autonomy-82187f8f` | `complete` | `export_functions` | `shard-03` | 14 | `976ff8736206d02f6450086a2020d0fe3f5de54409bd147f9caf372ad0ffeea7` | `successful_superseded_by_complete_membership_retry` |
| 22 | `A16_SUBJECT_AUTONOMY` | `20260720T021840Z-w3-a16-autonomy-data-8b05dcec` | `complete` | `dump_data` | `shard-01` | 9 | `34b30eb65bb97243ee4b9fe8c2baf09a45becfb9a198ad8d84f70676be883a80` | `supplemental_data_success` |
| 23 | `A17_NUKE_STOCKPILE` | `20260720T021840Z-w3-a17-nuke-data-3a526906` | `complete` | `dump_data` | `shard-05` | 7 | `20f780f52303e5c59868fcce8ee09258d4dc997bf5d3aadbdec3a3c00ce509bf` | `supplemental_data_success` |
| 24 | `A28_COLLECTION_CONTROLLERS` | `20260720T021840Z-w3-a28-controller-vtables-2f8bc20b` | `complete` | `dump_data` | `shard-17` | 2 | `1ad43b85bee90ee0428c785d8d825eff7f8256669e1470d5bd9d43fc29a00b83` | `supplemental_data_success` |
| 25 | `A28_COLLECTION_CONTROLLERS` | `20260720T021840Z-w3-a28-direct-retry-5065e703` | `complete` | `export_functions` | `shard-09` | 23 | `8ce0d88387228dc8a67553e6772c253f40792971760e2f2b695a87af2199b841` | `successful_depth0_retry_supplemented_by_segmented_depth1_attempts` |
| 26 | `A33_SUNK_SHIP_HISTORY` | `20260720T021840Z-w3-a33-vtables-1328220f` | `complete` | `dump_data` | `shard-10` | 4 | `c09db5903cf588bc792bbe63a3a34929316d2136ec0e104f4c99931b86b24369` | `supplemental_data_success` |
| 27 | `A34_DECISION_PANEL` | `20260720T021840Z-w3-a34-vtables-fe465659` | `complete` | `dump_data` | `shard-08` | 10 | `a5b06165bef6b8cc92d22595633ac8c778f539cf98f1f84f3dec576b8f307a24` | `supplemental_data_success` |
| 28 | `A40_DESIGNER_IDENTITY` | `20260720T021840Z-w3-a40-classifiers-065d374b` | `failed` | `export_functions` | `shard-18` | 12 | `none` | `failed_superseded_by_narrow_retry_byte_dump_and_final_request` |
| 29 | `B5_WAR_RESIDUALS` | `20260720T021840Z-w3-b5-war-data-84a4ed4a` | `complete` | `dump_data` | `shard-19` | 29 | `8155d09aba1bccff747087ea69cdb41439a7efdff7c99d7624510bd889f186bf` | `supplemental_data_success` |
| 30 | `WEATHER_SETTER_CHAIN` | `20260720T021840Z-w3-weather-data-3422e445` | `complete` | `dump_data` | `shard-07` | 3 | `225f21f45e4744ac489237459636ed94948f3192e0648d56fa532f1ffaa7db7a` | `supplemental_data_success` |
| 31 | `X12_SPEED_PAUSE` | `20260720T021840Z-w3-x12-speed-pause-e662af9e` | `complete` | `export_functions` | `shard-02` | 3 | `41f0ac897d332ab6868c10445d620bf35b19bc7bdc2828f017ebf53cfed14cc6` | `authoritative_success` |
| 32 | `X9_SPECIALIZATIONS` | `20260720T021840Z-w3-x9-program-vtables-c64eb8bf` | `complete` | `dump_data` | `shard-11` | 3 | `66724280816a21603e6cdff416b58da03473a5227b94de740fa8838171e7afc3` | `supplemental_data_success` |
| 33 | `X9_SPECIALIZATIONS` | `20260720T021840Z-w3-x9-specialization-strings-7106cf6c` | `complete` | `search_strings` | `shard-06` | 4 | `e14989d7b81aa36baf9499811e0ab8665207057e7fc265fd2c4533e20a9e56f5` | `supplemental_string_search_success` |
| 34 | `A17_NUKE_STOCKPILE` | `20260720T021841Z-w3-a17-nuke-743578ed` | `complete` | `export_functions` | `shard-04` | 9 | `3befb21facb2fdd5724602b2a72655ade23995b8270521aeecbe5eddeed34301` | `authoritative_success` |
| 35 | `A29_TECHNOLOGY_CANVAS` | `20260720T021841Z-w3-a29-tech-helpers-7487927f` | `complete` | `export_functions` | `shard-13` | 7 | `9837e5da6e0fb0c9cc4efdc49b1351b71a69810e5f94968e726da75679b2df5e` | `supplemental_function_success` |
| 36 | `A29_TECHNOLOGY_CANVAS` | `20260720T021841Z-w3-a29-tech-roots-b3f08ee5` | `complete` | `export_functions` | `shard-12` | 6 | `eab85630d79f7592bd1ae38f9ae5fb6102ddb5e2201cb3a1d6482a83cf5c74cb` | `authoritative_success` |
| 37 | `X2_CURRENT_LOSSES` | `20260720T021842Z-w3-x2-loss-supplement-b7d24965` | `complete` | `export_functions` | `shard-20` | 2 | `75bce4aae8857b18188689fdc8d3ac1f2af30c8d4581f049b2e4d7a794e218b0` | `supplemental_function_success` |
| 38 | `A16_SUBJECT_AUTONOMY` | `20260720T022347Z-w3-a16-autonomy-final15-d370faae` | `complete` | `export_functions` | `shard-17` | 15 | `797922914452ac03f21f9fe666f021625b6e0f6b9dfb49728a4a813d4ebe88a7` | `authoritative_success` |
| 39 | `A40_DESIGNER_IDENTITY` | `20260720T022347Z-w3-a40-classifier-retry-ad6a474d` | `failed` | `export_functions` | `shard-09` | 11 | `none` | `failed_superseded_by_wrapper_byte_dump_and_final_request` |
| 40 | `A40_DESIGNER_IDENTITY` | `20260720T022347Z-w3-a40-thunk-bytes-81f810fe` | `complete` | `dump_data` | `shard-01` | 1 | `856f8fabd23ad52b8ade502346b059ae9ff76fdc9a9d908814c985904e100451` | `supplemental_data_success` |
| 41 | `A40_DESIGNER_IDENTITY` | `20260720T022731Z-w3-a40-classifier-final-65c68d38` | `complete` | `export_functions` | `shard-09` | 10 | `76684486678fdb87f4a679c4faa855b61d4551c0a13bebbd6ba0c1553dbf6a41` | `authoritative_successful_retry` |
| 42 | `A40_DESIGNER_IDENTITY` | `20260720T022731Z-w3-a40-wrapper-bytes-f1cf3253` | `complete` | `dump_data` | `shard-01` | 1 | `7dbcad8466163a3fe1b0a31ca3276662e68fd7ea395e2d102b9c44e08df5ef0d` | `supplemental_data_success` |
| 43 | `A40_PEACE_CONFERENCE` | `20260720T023151Z-w3-a40-peace-omitted-terms-c70ebf5a` | `complete` | `search_strings` | `shard-01` | 5 | `8be06a1fc82617f2ba9d43f6389073d2dadafb49f4ef23aabd0119f32844a63d` | `supplemental_string_search_success` |
| 44 | `A40_PEACE_CONFERENCE` | `20260720T023151Z-w3-a40-peace-xref-followup-afb44169` | `complete` | `export_functions` | `shard-09` | 5 | `d06f303fbd2c772fbdc307ca4ca68045f4b08c670d76a47a1aa127d0139a61ae` | `supplemental_function_success` |
| 45 | `A28_COLLECTION_CONTROLLERS` | `20260720T023401Z-w3-a28-bonus-depth1-9bfbdc73` | `complete` | `export_functions` | `shard-14` | 4 | `9b916ba2650226ed2924e7004e752f98df6aa19a8a16dcf0644a4c24e82f9e92` | `authoritative_success` |
| 46 | `A28_COLLECTION_CONTROLLERS` | `20260720T023401Z-w3-a28-history-depth1-b1f16258` | `complete` | `export_functions` | `shard-13` | 5 | `d674531df84a69b03309b7d9e623f22ea8dd5fbab863c64e56f7030dc9cdad5d` | `authoritative_success` |
| 47 | `A28_COLLECTION_CONTROLLERS` | `20260720T023401Z-w3-a28-mio-root-depth1-13de3fff` | `failed` | `export_functions` | `shard-17` | 5 | `none` | `failed_superseded_by_single_seed_depth1_attempts` |
| 48 | `A28_COLLECTION_CONTROLLERS` | `20260720T023402Z-w3-a28-air-depth1-c99e90dc` | `complete` | `export_functions` | `shard-09` | 9 | `6e59f9026a87df480cb90fd3363b83942053efc92952deaa2ee485c4a90d0900` | `authoritative_success` |
| 49 | `A28_COLLECTION_CONTROLLERS` | `20260720T023531Z-w3-a28-mio-1-depth1-a808b2a9` | `failed` | `export_functions` | `shard-19` | 1 | `none` | `failed_retained_unresolved_seed_no_artifact_no_promotion` |
| 50 | `A28_COLLECTION_CONTROLLERS` | `20260720T023531Z-w3-a28-mio-2-depth1-30e5260f` | `complete` | `export_functions` | `shard-18` | 1 | `38813763635e4fda16fb1e529f2429083361fddc94fd81c2fe44bea3ffb4b943` | `authoritative_success` |
| 51 | `A28_COLLECTION_CONTROLLERS` | `20260720T023531Z-w3-a28-mio-3-depth1-0635f2f2` | `complete` | `export_functions` | `shard-13` | 1 | `477ed373c5224e7814f16ba155930fb1f772299e1327fdcd10b366b63bcae0d3` | `authoritative_success` |
| 52 | `A28_COLLECTION_CONTROLLERS` | `20260720T023531Z-w3-a28-mio-4-depth1-aab3061f` | `complete` | `export_functions` | `shard-17` | 1 | `fde4bfdda86c7125aa1bd1121a3f60e40ff90e748aa3028cf7a82a1e4141c827` | `authoritative_success` |
| 53 | `A28_COLLECTION_CONTROLLERS` | `20260720T023531Z-w3-a28-mio-5-depth1-e995a502` | `complete` | `export_functions` | `shard-05` | 1 | `e9be94e7b701a6f860ebed2ee0764cb486c25d34a6790c0080a2d5593c0dc10e` | `authoritative_success` |
| 54 | `A40_PEACE_CONFERENCE` | `20260720T024001Z-w3-a40-peace-breakdown-1d57fded` | `complete` | `export_functions` | `shard-01` | 4 | `f15141f08fdb990c597d2cf6dd6af3a1a0cd2df5f98bae8e3ee7a0835bf2cc7d` | `supplemental_function_success` |
| 55 | `A29_TECHNOLOGY_CANVAS` | `20260720T024434Z-w3-a29-callback-supplement-37432e72` | `failed` | `export_functions` | `shard-05` | 3 | `none` | `failed_non_function_seeds_superseded_by_exact_byte_dump` |
| 56 | `A29_TECHNOLOGY_CANVAS` | `20260720T024555Z-w3-a29-callback-bytes-c223e1a2` | `complete` | `dump_data` | `shard-05` | 3 | `6f98be3d567c6f81a118f4bb95a2d79a055b9d1a8591152d42ef2b4eb2b790f7` | `supplemental_data_success` |

Every row in the JSON ledger preserves the request array in original order and records the job JSON hash, main manifest/artifact hash, exact request sources, literal repository consumers, retry/supplement relationships, and pool receipt hash.
