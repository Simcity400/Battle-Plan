# the game Installed content Atlas

Vanilla root: `<local-path> of Iron IV`. Generated from static shipped files only; `hoi4.exe` was not launched. Binary formats are described from headers, file sizes, and ASCII signatures only.

This atlas maps replay-renderer visuals to shipped file sources: map rasters, shader laws, model/entity binding, UI `.gui`/`.gfx`, colors, fonts, flags, DLC extensions, and localisation where it feeds map text. Explicit non-graphics exclusions are listed so they are not silent.

## 1. Inventory

Tree walk measured `2021` directories and `58272` files. Files outside explicit non-graphics exclusions: `54836`; excluded non-graphics files: `3436`.
| Top-level path | Files recursive | Dirs recursive | Files immediate | Dirs immediate | Extensions recursive |
| --- | --- | --- | --- | --- | --- |
| assets | 7 | 0 | 7 | 0 | .png 5, .json 1, .ttf 1 |
| browser | 1 | 0 | 1 | 0 | .js 1 |
| buildings_nudger_markers.txt | 1 | 0 | 1 | 0 | .txt 1 |
| cef | 12 | 4 | 0 | 2 | .pak 12 |
| changelog.txt | 1 | 0 | 1 | 0 | .txt 1 |
| checksum_manifest.txt | 1 | 0 | 1 | 0 | .txt 1 |
| clausewitz_branch.txt | 1 | 0 | 1 | 0 | .txt 1 |
| clausewitz_rev.txt | 1 | 0 | 1 | 0 | .txt 1 |
| common | 2260 | 125 | 13 | 81 | .txt 2217, .md 39, .lua 3, .json 1 |
| console_history.txt | 1 | 0 | 1 | 0 | .txt 1 |
| country_metadata | 1 | 0 | 1 | 0 | .txt 1 |
| crash_reporter | 2 | 1 | 0 | 1 | .exe 1, .manifest 1 |
| dlc | 12953 | 483 | 0 | 27 | .dds 8077, .wav 2444, .anim 1083, .mesh 826, .asset 194, .gfx 127, .ogg 91, .txt 43, .png 40, .dlc 24, .gui 2, ._advisordds 1, .zip 1 |
| dlc_metadata | 1 | 1 | 0 | 1 | .txt 1 |
| documentation | 22 | 0 | 22 | 0 | .html 11, .md 11 |
| dowser.exe | 1 | 0 | 1 | 0 | .exe 1 |
| EmptySteamDepot | 0 | 0 | 0 | 0 | - |
| events | 128 | 0 | 128 | 0 | .txt 128 |
| fake.txt | 1 | 0 | 1 | 0 | .txt 1 |
| fake2.txt | 1 | 0 | 1 | 0 | .txt 1 |
| fakegfx.txt | 1 | 0 | 1 | 0 | .txt 1 |
| fakegfx2.txt | 1 | 0 | 1 | 0 | .txt 1 |
| gfx | 30414 | 359 | 4 | 23 | .dds 21839, .tga 6721, .anim 535, .mesh 436, .fnt 379, .asset 213, .png 96, .cur 49, .shader 46, .gfx 36, .txt 27, .fxh 13, .ani 10, .ico 10, ... +4 |
| history | 2058 | 4 | 0 | 4 | .txt 2058 |
| hoi4_branch.txt | 1 | 0 | 1 | 0 | .txt 1 |
| hoi4_rev.txt | 1 | 0 | 1 | 0 | .txt 1 |
| hoi4.exe | 1 | 0 | 1 | 0 | .exe 1 |
| hoi4.exe.manifest | 1 | 0 | 1 | 0 | .manifest 1 |
| icudtl.dat | 1 | 0 | 1 | 0 | .dat 1 |
| imgui.ini | 1 | 0 | 1 | 0 | .ini 1 |
| integrated_dlc | 3587 | 124 | 0 | 4 | .dds 1661, .wav 1470, .anim 194, .mesh 135, .asset 51, .gfx 46, .ogg 13, .txt 8, .dlc 4, .png 4, .gui 1 |
| interface | 620 | 17 | 278 | 14 | .gui 453, .gfx 162, .txt 2, .dds 1, .info 1, .sfx 1 |
| launcher-installer-windows_2024.14.msi | 1 | 0 | 1 | 0 | .msi 1 |
| launcher-settings.json | 1 | 0 | 1 | 0 | .json 1 |
| libcef.so | 1 | 0 | 1 | 0 | .so 1 |
| licenses.txt | 1 | 0 | 1 | 0 | .txt 1 |
| localisation | 2074 | 10 | 2 | 10 | .yml 2073, .txt 1 |
| map | 391 | 3 | 22 | 3 | .txt 317, .dds 62, .bmp 9, .csv 2, .map 1 |
| music | 67 | 2 | 28 | 2 | .ogg 61, .asset 3, .txt 3 |
| pdx_launcher | 185 | 16 | 1 | 2 | .dds 98, .fnt 21, .yml 21, .tga 16, .gfx 13, .gui 9, .asset 3, .shader 2, .txt 2 |
| pdx_online_assets | 33 | 5 | 0 | 2 | .dds 16, .fnt 6, .gfx 4, .gui 3, .shader 2, .tga 2 |
| PDXSDK.dll | 1 | 0 | 1 | 0 | .dll 1 |
| portraits | 2 | 0 | 2 | 0 | .txt 2 |
| previewer_assets | 47 | 7 | 1 | 3 | .dds 21, .tga 7, .gfx 6, .fnt 4, .gui 3, .cur 1, .png 1, .sfx 1, .ttc 1, .ttf 1, .txt 1 |
| railway_mapicon_bg.dds | 1 | 0 | 1 | 0 | .dds 1 |
| run_hoi4 | 1 | 0 | 1 | 0 | [no extension] 1 |
| script | 5 | 0 | 5 | 0 | .lua 5 |
| settings-layout.json | 1 | 0 | 1 | 0 | .json 1 |
| sound | 992 | 13 | 8 | 8 | .wav 982, .asset 10 |
| steam_api64.dll | 1 | 0 | 1 | 0 | .dll 1 |
| steam_appid.txt | 1 | 0 | 1 | 0 | .txt 1 |
| tbb_debug.dll | 1 | 0 | 1 | 0 | .dll 1 |
| tbb.dll | 1 | 0 | 1 | 0 | .dll 1 |
| tests | 35 | 0 | 35 | 0 | .txt 34, .info 1 |
| tools | 9 | 2 | 3 | 2 | .png 3, .html 2, .js 2, .json 1, .settings 1 |
| tut_supply_mapmode_img.png | 1 | 0 | 1 | 0 | .png 1 |
| tutorial | 1 | 0 | 1 | 0 | .txt 1 |
| tweakergui_assets | 38 | 5 | 0 | 2 | .dds 15, .tga 13, .fnt 5, .gfx 2, .shader 2, .gui 1 |
| unit_nudger_markers.txt | 1 | 0 | 1 | 0 | .txt 1 |
| unit_test.txt | 1 | 0 | 1 | 0 | .txt 1 |
| wiki | 2295 | 810 | 679 | 46 | .html 1314, .png 830, .jpeg 62, .jpg 38, .css 8, .gif@346d8 1, .gif@53eb5 1, .gif@57f34 1, .gif@ca65b 1, .ico 1, .php 1, .php@debug=false&lang=en&modules=startup&only=scripts&skin=vector 1, .png@0c329 1, .png@27d60 1, ... +34 |

### Explicit Graphics-Irrelevant Exclusions

| Excluded root | Files | Dirs | Extensions | Reason |
| --- | --- | --- | --- | --- |
| music | 67 | 2 | .ogg 61, .asset 3, .txt 3 | audio/music payloads; not map/UI rendering inputs |
| sound | 992 | 13 | .wav 982, .asset 10 | audio payloads; not map/UI rendering inputs |
| tutorial | 1 | 0 | .txt 1 | tutorial save/data text, not graphics law |
| wiki | 2295 | 810 | .html 1314, .png 830, .jpeg 62, .jpg 38, .css 8, .gif@346d8 1, .gif@53eb5 1, .gif@57f34 1, .gif@ca65b 1, .ico 1, .php 1, .php@debug=false&lang=en&modules=startup&only=scripts&skin=vector 1, .png@0c329 1, .png@27d60 1, ... +34 | launcher/manual web content, not shipped renderer inputs |
| documentation | 22 | 0 | .html 11, .md 11 | script docs/reference HTML, not renderer inputs |
| tests | 35 | 0 | .txt 34, .info 1 | test fixtures, not runtime graphics sources |
| crash_reporter | 2 | 1 | .exe 1, .manifest 1 | crash reporter executable/manifest |
| browser | 1 | 0 | .js 1 | embedded browser script, not the game renderer input |
| cef | 12 | 4 | .pak 12 | CEF runtime resources |
| tools | 9 | 2 | .png 3, .html 2, .js 2, .json 1, .settings 1 | launcher/tooling files |
| EmptySteamDepot | 0 | 0 | - | empty depot placeholder |

One-level inventory for every top-level directory is in the appendix.

## 2. gfx/FX Shader Atlas

`gfx/FX` contains `58` shader/include files (.shader and .fxh). Each row is one shipped file with effects, passes, constant buffers, texture slots/declarations, and includes.
| File | Purpose | Lines | Effects | Passes | Constant buffers | Texture slots/declarations | Includes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| gfx/FX/arrow.shader | Defines ArrowEffect | 200 | ArrowEffect (gfx/FX/arrow.shader:193) | - | ConstantBuffer( 1, 32 ) (59) | - | - |
| gfx/FX/bloom.shader | Defines bloom, bloom_alpha | 140 | bloom (gfx/FX/bloom.shader:128), bloom_alpha (gfx/FX/bloom.shader:134) | - | ConstantBuffer( 2, 39 ) (40) | - | - |
| gfx/FX/border.shader | Defines border | 169 | border (gfx/FX/border.shader:163) | - | ConstantBuffer( 2, 48 ) (72) | - | - |
| gfx/FX/buttonstate_blendframes.shader | Defines Up, Down, Disable | 130 | Up (gfx/FX/buttonstate_blendframes.shader:106), Down (gfx/FX/buttonstate_blendframes.shader:112), Disable (gfx/FX/buttonstate_blendframes.shader:118), Over (gfx/FX/buttonstate_blendframes.shader:124) | - | - | - | - |
| gfx/FX/buttonstate_fade_frames_to_black.shader | Defines Up, Down, Disable | 133 | Up (gfx/FX/buttonstate_fade_frames_to_black.shader:109), Down (gfx/FX/buttonstate_fade_frames_to_black.shader:115), Disable (gfx/FX/buttonstate_fade_frames_to_black.shader:121), Over (gfx/FX/buttonstate_fade_frames_to_black.shader:127) | - | - | - | - |
| gfx/FX/buttonstate_linear.shader | Defines Up, Down, Disable | 156 | Up (gfx/FX/buttonstate_linear.shader:132), Down (gfx/FX/buttonstate_linear.shader:138), Disable (gfx/FX/buttonstate_linear.shader:144), Over (gfx/FX/buttonstate_linear.shader:150) | - | ConstantBuffer( 0, 0 ) (34) | - | - |
| gfx/FX/buttonstate_nodowneffect.shader | Defines Up, Down, Disable | 155 | Up (gfx/FX/buttonstate_nodowneffect.shader:131), Down (gfx/FX/buttonstate_nodowneffect.shader:137), Disable (gfx/FX/buttonstate_nodowneffect.shader:143), Over (gfx/FX/buttonstate_nodowneffect.shader:149) | - | ConstantBuffer( 0, 0 ) (34) | - | - |
| gfx/FX/buttonstate_nodownordisableeffect.shader | Defines Up, Down, Disable | 123 | Up (gfx/FX/buttonstate_nodownordisableeffect.shader:99), Down (gfx/FX/buttonstate_nodownordisableeffect.shader:105), Disable (gfx/FX/buttonstate_nodownordisableeffect.shader:111), Over (gfx/FX/buttonstate_nodownordisableeffect.shader:117) | - | ConstantBuffer( 0, 0 ) (34) | - | - |
| gfx/FX/buttonstate_nontransparent.shader | Defines Up, Down, Disable | 133 | Up (gfx/FX/buttonstate_nontransparent.shader:109), Down (gfx/FX/buttonstate_nontransparent.shader:115), Disable (gfx/FX/buttonstate_nontransparent.shader:121), Over (gfx/FX/buttonstate_nontransparent.shader:127) | - | - | - | - |
| gfx/FX/buttonstate_onlydisable.shader | Defines Up, Down, Disable | 153 | Up (gfx/FX/buttonstate_onlydisable.shader:129), Down (gfx/FX/buttonstate_onlydisable.shader:135), Disable (gfx/FX/buttonstate_onlydisable.shader:141), Over (gfx/FX/buttonstate_onlydisable.shader:147) | - | - | - | - |
| gfx/FX/buttonstate_rendertarget.shader | Defines Up, Down, Disable | 98 | Up (gfx/FX/buttonstate_rendertarget.shader:74), Down (gfx/FX/buttonstate_rendertarget.shader:80), Disable (gfx/FX/buttonstate_rendertarget.shader:86), Over (gfx/FX/buttonstate_rendertarget.shader:92) | - | - | - | - |
| gfx/FX/buttonstate.fxh | Shared include/constants/functions | 16 | - | - | ConstantBuffer( 0, 0 ) (7) | - | - |
| gfx/FX/buttonstate.shader | Defines Up, Down, Disable | 248 | Up (gfx/FX/buttonstate.shader:224), Down (gfx/FX/buttonstate.shader:230), Disable (gfx/FX/buttonstate.shader:236), Over (gfx/FX/buttonstate.shader:242) | - | - | - | - |
| gfx/FX/circularprogressbar.shader | Defines CircularProgressBar | 92 | CircularProgressBar (gfx/FX/circularprogressbar.shader:88) | - | ConstantBuffer( 0, 0 ) (43) | - | - |
| gfx/FX/coa_shield.shader | Defines Standard, Color, Seal | 241 | Standard (gfx/FX/coa_shield.shader:217), Color (gfx/FX/coa_shield.shader:223), Seal (gfx/FX/coa_shield.shader:229), Frame (gfx/FX/coa_shield.shader:235) | - | ConstantBuffer( 0, 0 ) (70) | - | - |
| gfx/FX/color.shader | Defines Color | 61 | Color (gfx/FX/color.shader:55) | - | ConstantBuffer( 0, 0 ) (18) | - | - |
| gfx/FX/constants.fxh | Shared include/constants/functions | 249 | - | - | - | - | - |
| gfx/FX/DebugLines.shader | Defines DebugLines | 64 | DebugLines (gfx/FX/DebugLines.shader:58) | - | ConstantBuffer( 0, 0 ) (15) | - | - |
| gfx/FX/DebugTexture.shader | Defines DebugTexture | 82 | DebugTexture (gfx/FX/DebugTexture.shader:76) | - | ConstantBuffer( 0, 0 ) (31) | - | - |
| gfx/FX/defines_glsl.fxh | Shared include/constants/functions | 135 | - | - | - | tex (22) | - |
| gfx/FX/defines_hlsl_dx11.fxh | Shared include/constants/functions | 65 | - | - | - | _Texture (25), Texture (28), STextureSampler2D (48) | - |
| gfx/FX/defines_hlsl.fxh | Shared include/constants/functions | 30 | - | - | - | - | - |
| gfx/FX/downsample_luminance.shader | Defines LuminanceDownsample, LuminanceGather | 153 | LuminanceDownsample (gfx/FX/downsample_luminance.shader:141), LuminanceGather (gfx/FX/downsample_luminance.shader:147) | - | ConstantBuffer( 2, 39 ) (44) | - | - |
| gfx/FX/downsample.shader | Defines downsample | 90 | downsample (gfx/FX/downsample.shader:84) | - | - | - | - |
| gfx/FX/fade_frames_to_black_constants.fxh | Shared include/constants/functions | 8 | - | - | - | - | - |
| gfx/FX/fow.fxh | Shared include/constants/functions | 93 | - | - | - | IntelTex (25), ExtraFOW (30), NoiseTex (43), FOWTexture (85) | - |
| gfx/FX/linechart.shader | Defines linechart | 79 | linechart (gfx/FX/linechart.shader:73) | - | ConstantBuffer( 0, 0 ) (16) | - | - |
| gfx/FX/lut_blender.shader | Defines LutBlend1, LutBlend2 | 134 | LutBlend1 (gfx/FX/lut_blender.shader:122), LutBlend2 (gfx/FX/lut_blender.shader:128) | - | ConstantBuffer( 0, 0 ) (60) | - | - |
| gfx/FX/maparrow.shader | Defines MapArrowDefault, MapArrowDefaultWithDepth, MapArrowDefaultWithDepthTestOnly | 658 | MapArrowDefault (gfx/FX/maparrow.shader:589), MapArrowDefaultWithDepth (gfx/FX/maparrow.shader:596), MapArrowDefaultWithDepthTestOnly (gfx/FX/maparrow.shader:603), MapArrowNoHeadWidthDepth (gfx/FX/maparrow.shader:610), MapArrowNoHeadWidthDepthSeethrough (gfx/FX/maparrow.shader:618), MapArrowNoHead (gfx/FX/maparrow.shader:627), MapArrowNoHeadSeethrough (gfx/FX/maparrow.shader:635), MapSymbolDefault (gfx/FX/maparrow.shader:644), MapSymbolDefaultAdd (gfx/FX/maparrow.shader:651) | - | ConstantBuffer( 3, 32 ) # For arrow shader (153), ConstantBuffer( 4, 32 ) # For symbol shader (171) | - | - |
| gfx/FX/mapname.shader | Defines mapname | 117 | mapname (gfx/FX/mapname.shader:110) | - | ConstantBuffer( 1, 32 ) (24) | - | - |
| gfx/FX/maskedflag.shader | Defines Up, Down, Disable | 160 | Up (gfx/FX/maskedflag.shader:136), Down (gfx/FX/maskedflag.shader:142), Disable (gfx/FX/maskedflag.shader:148), Over (gfx/FX/maskedflag.shader:154) | - | ConstantBuffer( 0, 0 ) (43) | - | - |
| gfx/FX/particle.shader | Defines ParticleAlphaBlend, ParticlePreAlphaBlend, ParticleAdditive | 285 | ParticleAlphaBlend (gfx/FX/particle.shader:239), ParticlePreAlphaBlend (gfx/FX/particle.shader:246), ParticleAdditive (gfx/FX/particle.shader:254), ParticleAlphaBlendNoBillboard (gfx/FX/particle.shader:261), ParticlePreAlphaBlendNoBillboard (gfx/FX/particle.shader:269), ParticleAdditiveNoBillboard (gfx/FX/particle.shader:278) | - | ConstantBuffer( 1, 32 ) (71), ConstantBuffer( 2, 36 ) (76), ConstantBuffer( 3, 40 ) (81) | - | - |
| gfx/FX/pdxmap.fxh | Shared include/constants/functions | 9 | - | - | ConstantBuffer( 1, 32 ) (2) | - | - |
| gfx/FX/pdxmap.shader | Defines terrain_low_gfx, terrain, underwater | 507 | terrain_low_gfx (gfx/FX/pdxmap.shader:481), terrain (gfx/FX/pdxmap.shader:488), underwater (gfx/FX/pdxmap.shader:495), terrainunlit (gfx/FX/pdxmap.shader:501) | - | - | - | - |
| gfx/FX/pdxmesh.shader | Defines PdxMeshStandard, PdxMeshStandardSkinned, PdxMeshStandardShadow | 994 | PdxMeshStandard (gfx/FX/pdxmesh.shader:747), PdxMeshStandardSkinned (gfx/FX/pdxmesh.shader:754), PdxMeshStandardShadow (gfx/FX/pdxmesh.shader:761), PdxMeshStandardSkinnedShadow (gfx/FX/pdxmesh.shader:767), PdxMeshStandardSnow (gfx/FX/pdxmesh.shader:774), PdxMeshStandardSnowShadow (gfx/FX/pdxmesh.shader:780), PdxMeshAdvanced (gfx/FX/pdxmesh.shader:787), PdxMeshAdvancedSkinned (gfx/FX/pdxmesh.shader:795), PdxMeshAdvancedShadow (gfx/FX/pdxmesh.shader:803), PdxMeshAdvancedSkinnedShadow (gfx/FX/pdxmesh.shader:809), PdxMeshAdvancedSnow (gfx/FX/pdxmesh.shader:816), PdxMeshAdvancedSnowSkinned (gfx/FX/pdxmesh.shader:824), PdxMeshAdvancedSnowShadow (gfx/FX/pdxmesh.shader:831), PdxMeshAdvancedSnowSkinnedShadow (gfx/FX/pdxmesh.shader:837), PdxMeshAdvancedSkinnedAlphaTest (gfx/FX/pdxmesh.shader:844), PdxMeshAdvancedSkinnedAlphaTestShadow (gfx/FX/pdxmesh.shader:851), PdxMeshAdvancedAnimSkinned (gfx/FX/pdxmesh.shader:859), PdxMeshAdvancedAnimSkinnedShadow (gfx/FX/pdxmesh.shader:866), PdxMeshAlphaBlend (gfx/FX/pdxmesh.shader:873), PdxMeshAlphaBlendSkinned (gfx/FX/pdxmesh.shader:880), PdxMeshAlphaBlendShadow (gfx/FX/pdxmesh.shader:887), PdxMeshAlphaBlendSkinnedShadow (gfx/FX/pdxmesh.shader:893), PdxMeshSnow (gfx/FX/pdxmesh.shader:900), PdxMeshSnowSkinned (gfx/FX/pdxmesh.shader:907), PdxMeshSnowShadow (gfx/FX/pdxmesh.shader:914), PdxMeshSnowSkinnedShadow (gfx/FX/pdxmesh.shader:920), PdxMeshBorder (gfx/FX/pdxmesh.shader:927), PdxMeshBorderShadow (gfx/FX/pdxmesh.shader:934), DebugNormal (gfx/FX/pdxmesh.shader:941), DebugNormalSkinned (gfx/FX/pdxmesh.shader:947), PdxMeshStandard_NoFoW_NoTISkinned (gfx/FX/pdxmesh.shader:953), PdxMeshStandard_NoFoW_NoTISkinnedShadow (gfx/FX/pdxmesh.shader:959), PdxMeshTrain (gfx/FX/pdxmesh.shader:965), PdxMeshTrainShadow (gfx/FX/pdxmesh.shader:973), PdxMeshTrainSkinned (gfx/FX/pdxmesh.shader:980), PdxMeshTrainSkinnedShadow (gfx/FX/pdxmesh.shader:988) | - | ConstantBuffer( 1, 28 ) (216), ConstantBuffer( 2, 41 ) (234) | - | - |
| gfx/FX/pdxwater.shader | Defines water_low_gfx, water_no_refractions, water | 450 | water_low_gfx (gfx/FX/pdxwater.shader:429), water_no_refractions (gfx/FX/pdxwater.shader:436), water (gfx/FX/pdxwater.shader:443) | - | ConstantBuffer( 3, 48 ) (182) | TexCh (257) | - |
| gfx/FX/portrait.shader | Defines Standard, Eyes, Hair | 148 | Standard (gfx/FX/portrait.shader:130), Eyes (gfx/FX/portrait.shader:136), Hair (gfx/FX/portrait.shader:142) | - | ConstantBuffer( 0, 0 ) (34), ConstantBuffer( 1, 32 ) (44) | - | - |
| gfx/FX/posteffect_base.fxh | Shared include/constants/functions | 12 | - | - | ConstantBuffer( 1, 32 ) (1) | - | - |
| gfx/FX/progress_minmax.shader | Defines Color, Texture | 118 | Color (gfx/FX/progress_minmax.shader:107), Texture (gfx/FX/progress_minmax.shader:113) | - | ConstantBuffer( 0, 0 ) (42) | - | - |
| gfx/FX/progress_radial.shader | Defines Color, Texture | 122 | Color (gfx/FX/progress_radial.shader:110), Texture (gfx/FX/progress_radial.shader:116) | - | ConstantBuffer( 0, 0 ) (43) | - | - |
| gfx/FX/progress_reverse.shader | Defines Color, Texture | 139 | Color (gfx/FX/progress_reverse.shader:127), Texture (gfx/FX/progress_reverse.shader:133) | - | ConstantBuffer( 0, 0 ) (43) | - | - |
| gfx/FX/progress_startend.shader | Defines Color, Texture | 139 | Color (gfx/FX/progress_startend.shader:127), Texture (gfx/FX/progress_startend.shader:133) | - | ConstantBuffer( 0, 0 ) (43) | - | - |
| gfx/FX/progress.shader | Defines Color, Texture | 120 | Color (gfx/FX/progress.shader:108), Texture (gfx/FX/progress.shader:114) | - | ConstantBuffer( 0, 0 ) (43) | - | - |
| gfx/FX/restorescene.shader | Defines RestoreBloom, Restore | 249 | RestoreBloom (gfx/FX/restorescene.shader:236), Restore (gfx/FX/restorescene.shader:243) | - | - | - | - |
| gfx/FX/river.shader | Defines river_low_gfx, river | 439 | river_low_gfx (gfx/FX/river.shader:427), river (gfx/FX/river.shader:434) | - | ConstantBuffer( 1, 32 ) (181) | SnowNoise (242) | - |
| gfx/FX/saturation_slider.shader | Defines Up, Down, Disable | 242 | Up (gfx/FX/saturation_slider.shader:218), Down (gfx/FX/saturation_slider.shader:224), Disable (gfx/FX/saturation_slider.shader:230), Over (gfx/FX/saturation_slider.shader:236) | - | - | - | - |
| gfx/FX/shadow.fxh | Shared include/constants/functions | 57 | - | - | - | - | - |
| gfx/FX/shadowblur.shader | Defines ShadowBlurHorizontal, ShadowBlurVertical | 137 | ShadowBlurHorizontal (gfx/FX/shadowblur.shader:125), ShadowBlurVertical (gfx/FX/shadowblur.shader:131) | - | ConstantBuffer( 0, 0 ) (33) | - | - |
| gfx/FX/simple.shader | Defines Simple, Simple3D | 114 | Simple (gfx/FX/simple.shader:102), Simple3D (gfx/FX/simple.shader:108) | - | ConstantBuffer( 0, 0 ) (36) | - | - |
| gfx/FX/sky.shader | Defines sky | 87 | sky (gfx/FX/sky.shader:81) | - | - | - | - |
| gfx/FX/sprite_animation.fxh | Shared include/constants/functions | 161 | - | - | ConstantBuffer( 1, 7 ) (1) | MaskTextureSampler (119), MaskTexture2Sampler (143) | - |
| gfx/FX/standardfuncsgfx.fxh | Shared include/constants/functions | 1253 | - | - | ConstantBuffer( 0, 0 ) (2) | TexFoW (189), MudSnowTexture (231), MudDiffuseGlossSampler (238), SnowTextureSampler (264), SnowNoise (265), NormalTex (303), TexCh (791), gbTex (840), TexCh1 (887), TexMaskSampler (979) | - |
| gfx/FX/static_button.shader | Defines Up, Down, Disable | 114 | Up (gfx/FX/static_button.shader:90), Down (gfx/FX/static_button.shader:96), Disable (gfx/FX/static_button.shader:102), Over (gfx/FX/static_button.shader:108) | - | ConstantBuffer( 0, 0 ) (34) | - | - |
| gfx/FX/strait.shader | Defines strait | 125 | strait (gfx/FX/strait.shader:119) | - | - | - | - |
| gfx/FX/text.shader | Defines Text, Text3D | 118 | Text (gfx/FX/text.shader:106), Text3D (gfx/FX/text.shader:112) | - | ConstantBuffer( 0, 0 ) (36) | - | - |
| gfx/FX/tiled_pointlights.fxh | Shared include/constants/functions | 53 | - | - | ConstantBuffer( 2, 62 ) # after shadowmap constants (3) | LightDataSampler (23) | - |
| gfx/FX/traderoute.shader | Defines TradeRoute, TradeRouteTrade | 193 | TradeRoute (gfx/FX/traderoute.shader:181), TradeRouteTrade (gfx/FX/traderoute.shader:187) | - | ConstantBuffer( 1, 32 ) (66) | - | - |
| gfx/FX/tree.shader | Defines tree_low_gfx, tree, treeshadow | 485 | tree_low_gfx (gfx/FX/tree.shader:460), tree (gfx/FX/tree.shader:467), treeshadow (gfx/FX/tree.shader:473), treeunlit (gfx/FX/tree.shader:479) | - | ConstantBuffer( 1, 32 ) (187) | TreeMaskTextureSampler (303) | - |

### constants.fxh High-Impact Constants

`gfx/FX/constants.fxh` encodes zoom laws, political fill/outline thresholds, snow/ice/water/city-light constants, day/night ambient ramps, FOW removal, tree seasonal parameters, and map-arrow dimensions. Full constant table is in the appendix.
| Constant | Type | Value | Comment | Citation |
| --- | --- | --- | --- | --- |
| NIGHT_AMBIENT_BOOST | float | 3.0f | can just be baked into the below later ye? | gfx/FX/constants.fxh:14 |
| CITY_LIGHTS_TILING | float | 0.09103 |  | gfx/FX/constants.fxh:49 |
| CITY_LIGHTS_INTENSITY | float | 5.5 |  | gfx/FX/constants.fxh:50 |
| SNOW_CAM_MIN | float | 50.0f |  | gfx/FX/constants.fxh:67 |
| SNOW_CAM_MAX | float | 300.0f |  | gfx/FX/constants.fxh:68 |
| ICE_CAM_MIN | float | 100.0f |  | gfx/FX/constants.fxh:71 |
| ICE_CAM_MAX | float | 350.0f |  | gfx/FX/constants.fxh:72 |
| SNOW_COLOR | float3 | float3( 0.46, 0.48, 0.69 ) |  | gfx/FX/constants.fxh:78 |
| ICE_COLOR | float3 | float3( 0.5f, 0.6f, 0.9f ) |  | gfx/FX/constants.fxh:87 |
| TREE_SEASON_MIN | float | 0.5f |  | gfx/FX/constants.fxh:113 |
| WATER_TIME_SCALE | float | 1.0f / 50.0f |  | gfx/FX/constants.fxh:132 |
| WATER_HEIGHT | float | 9.5f |  | gfx/FX/constants.fxh:133 |
| GB_CAM_MIN | float | 100.0f |  | gfx/FX/constants.fxh:195 |
| GB_CAM_MAX | float | 350.0f |  | gfx/FX/constants.fxh:196 |
| GB_CAM_MAX_FILLING_CLAMP | float | 0.8f | 0 to 1 value for clamping the fill when camera is at max distance | gfx/FX/constants.fxh:197 |
| GB_OUTLINE_CUTOFF_SEA | float | 0.990f | Magic number to balance cutoff on edges without neighbor (over Sea) | gfx/FX/constants.fxh:201 |
| GB_OPACITY_NEAR | float | 1.0f | Transparency when camera is near | gfx/FX/constants.fxh:202 |
| GB_OPACITY_FAR | float | 0.85f | Transparency when camera is far | gfx/FX/constants.fxh:203 |
| BORDER_MAP_TILE | float | 18000.0f |  | gfx/FX/constants.fxh:210 |
| SEC_MAP_TILE | float | 6000.0f |  | gfx/FX/constants.fxh:216 |

## 3. map/ Deep Dive

`map` contains `22` immediate files and `3` immediate subdirectories (map/strategicregions, map/supplyareas, map/terrain). `map/default.map` binds definitions/provinces/positions/terrain/rivers/heightmap/trees/continent/adjacency rules/adjacencies/ambient objects/seasons at `map/default.map:1`-`map/default.map:13`; tree palette indices are at `map/default.map:16`.
| Citation | Binding |
| --- | --- |
| map/default.map:1 | definitions = "definition.csv" |
| map/default.map:2 | provinces = "provinces.bmp" |
| map/default.map:3 | positions = "positions.txt" |
| map/default.map:4 | terrain = "terrain.bmp" |
| map/default.map:5 | rivers = "rivers.bmp" |
| map/default.map:6 | heightmap = "heightmap.bmp" |
| map/default.map:7 | tree_definition = "trees.bmp" |
| map/default.map:8 | continent = "continent.txt" |
| map/default.map:9 | adjacency_rules = "adjacency_rules.txt" |
| map/default.map:10 | adjacencies = "adjacencies.csv" |
| map/default.map:12 | ambient_object = "ambient_object.txt" |
| map/default.map:13 | seasons = "seasons.txt" |
| map/default.map:16 | tree = { 3 4 7 10 } |

### Map Immediate Files
| File | Measured format / role evidence |
| --- | --- |
| map/adjacencies.csv | Text 257 lines; first non-empty: map/adjacencies.csv:1 From;To;Type;Through;start_x;start_y;stop_x;stop_y;adjacency_rule_name;Comment |
| map/adjacency_rules.txt | Text 412 lines; first non-empty: map/adjacency_rules.txt:1 # Adjacency rules determine if an army, navy or trade can pass through the adjacency (e.g. a canal or strait). |
| map/ambient_object.txt | Text 207 lines; first non-empty: map/ambient_object.txt:1 type={ |
| map/buildings.txt | Text 66664 lines; first non-empty: map/buildings.txt:1 1;arms_factory;2946.00;11.63;1364.00;0.45;0 |
| map/cities.bmp | BMP 5632x2048, 8-bit, offset 1074, DIB 40, palette 255, compression 0, bytes 11535412 |
| map/cities.txt | Text 136 lines; first non-empty: map/cities.txt:1 types_source = "map/cities.bmp" |
| map/colors.txt | Text 201 lines; first non-empty: map/colors.txt:1 color = { 4 144 178 } |
| map/continent.txt | Text 10 lines; first non-empty: map/continent.txt:1 continents = { |
| map/default.map | Text 17 lines; first non-empty: map/default.map:1 definitions = "definition.csv" |
| map/definition.csv | Text 13414 lines; first non-empty: map/definition.csv:1 0;0;0;0;land;false;unknown;0 |
| map/heightmap.bmp | BMP 5632x2048, 8-bit, offset 1078, DIB 40, palette 256, compression 0, bytes 11535416 |
| map/positions.txt | Text 1 lines; first non-empty: empty |
| map/provinces.bmp | BMP 5632x2048, 24-bit, offset 54, DIB 40, palette 0, compression 0, bytes 34603062 |
| map/railways.txt | Text 910 lines; first non-empty: map/railways.txt:1 3 4 6521 11444 3312 6282 |
| map/rivers.bmp | BMP 5632x2048, 8-bit, offset 1078, DIB 40, palette 256, compression 0, bytes 11535416 |
| map/seasons.txt | Text 156 lines; first non-empty: map/seasons.txt:1 # winter = { |
| map/supply_nodes.txt | Text 728 lines; first non-empty: map/supply_nodes.txt:1 1 67 |
| map/terrain.bmp | BMP 5632x2048, 8-bit, offset 1158, DIB 124, palette 255, compression 0, bytes 11535494 |
| map/trees.bmp | BMP 1650x600, 8-bit, offset 1162, DIB 124, palette 256, compression 0, bytes 992362 |
| map/unitstacks.txt | Text 265356 lines; first non-empty: map/unitstacks.txt:1 2;0;3429.00;9.85;1699.00;0.00;0.30 |
| map/weatherpositions.txt | Text 589 lines; first non-empty: map/weatherpositions.txt:1 1;2781.24;9.90;1571.49;small |
| map/world_normal.bmp | BMP 2816x1024, 24-bit, offset 54, DIB 40, palette 0, compression 0, bytes 8650808 |

### Map Subdirectories
| Subdirectory | Files | Dirs | Extensions | Role |
| --- | --- | --- | --- | --- |
| map/strategicregions | 304 | 0 | .txt 304 | map graphics/law data |
| map/supplyareas | 1 | 0 | .txt 1 | map graphics/law data |
| map/terrain | 64 | 0 | .dds 62, .bmp 2 | map graphics/law data |

### Critical Map File Schemas and Laws

- `definition.csv` is headerless; row order is province id/R/G/B/type/coastal/terrain/continent inferred from records such as `map/definition.csv:1`.
- `adjacencies.csv` declares `From;To;Type;Through;start_x;start_y;stop_x;stop_y;adjacency_rule_name;Comment` at `map/adjacencies.csv:1`.
- `adjacency_rules.txt` documents its schema at `map/adjacency_rules.txt:1`-`map/adjacency_rules.txt:12`; Suez starts at `map/adjacency_rules.txt:15`, with `required_provinces` at `map/adjacency_rules.txt:48` and icon/offset at `map/adjacency_rules.txt:59`-`map/adjacency_rules.txt:60`.
- `buildings.txt` ships per-state building model placement rows; `map/buildings.txt:1` proves state/building/map-coordinate/rotation records.
- `unitstacks.txt` ships per-province unit stack placement rows beginning at `map/unitstacks.txt:1`.
- `railways.txt` and `supply_nodes.txt` are headerless graph files beginning at `map/railways.txt:1` and `map/supply_nodes.txt:1`.
- `airports.txt` and `rocketsites.txt` are map placement files; their first records are cited in the immediate-file table.
- `seasons.txt` carries seasonal color grading: winter `map/seasons.txt:63`, spring `map/seasons.txt:80`, summer `map/seasons.txt:95`, autumn `map/seasons.txt:110`, tree season remaps `map/seasons.txt:125`-`map/seasons.txt:156`.
- `strategicregions/*.txt` bind region ids, province lists, and weather. Example: id `map/strategicregions/1-Southern England.txt:3`, provinces `map/strategicregions/1-Southern England.txt:5`, weather `map/strategicregions/1-Southern England.txt:8`.
- `cities.txt` points at `map/cities.bmp` at `map/cities.txt:1`; pixel steps are `map/cities.txt:2`-`map/cities.txt:3`; mesh groups start at `map/cities.txt:6`.
- `ambient_object.txt` defines ambient map entities; `ambient_wind_entity` starts `map/ambient_object.txt:1`, `ambient_water` starts `map/ambient_object.txt:15`.
- `weatherpositions.txt` begins semicolon-delimited weather anchor rows at `map/weatherpositions.txt:1`.
- `heightmap.bmp` and `world_normal.bmp` dimensions/bit depth are measured in the immediate-file table; their numeric height/normal scaling is not declared in text and is listed as a gap.

### Terrain Palette -> common/terrain Mapping

`common/terrain/00_terrain.txt:320` states `color = index in bitmap color table (see terrain.bmp)`. The palette-index mapping is:
| Terrain key | Type | Palette index | Texture | Spawn city | Permanent snow | Citation |
| --- | --- | --- | --- | --- | --- | --- |
| terrain_0 | plains | 0 | 1 |  |  | common/terrain/00_terrain.txt:324 |
| terrain_1 | forest | 1 | 4 |  |  | common/terrain/00_terrain.txt:325 |
| desert_mountain | hills | 2 | 3 |  |  | common/terrain/00_terrain.txt:326 |
| desert | desert | 3 | 9 |  |  | common/terrain/00_terrain.txt:327 |
| terrain_4 | forest | 4 | 5 |  |  | common/terrain/00_terrain.txt:329 |
| terrain_5 | plains | 5 | 0 |  |  | common/terrain/00_terrain.txt:330 |
| terrain_6 | mountain | 6 | 11 |  |  | common/terrain/00_terrain.txt:331 |
| terrain_7 | desert | 7 | 12 |  |  | common/terrain/00_terrain.txt:332 |
| desert_hills | desert | 8 | 14 |  |  | common/terrain/00_terrain.txt:333 |
| terrain_9 | marsh | 9 | 6 |  |  | common/terrain/00_terrain.txt:334 |
| terrain_10 | mountain | 10 | 13 |  |  | common/terrain/00_terrain.txt:335 |
| desert | mountain | 11 | 11 |  |  | common/terrain/00_terrain.txt:337 |
| desert | desert | 12 | 8 |  |  | common/terrain/00_terrain.txt:338 |
| forest_13 | urban | 13 | 10 | yes |  | common/terrain/00_terrain.txt:340 |
| forest_14 | lakes | 14 | 255 |  |  | common/terrain/00_terrain.txt:341 |
| ocean_15 | ocean | 15 | 9 |  |  | common/terrain/00_terrain.txt:342 |
| snow_16 | mountain | 16 | 11 |  | yes | common/terrain/00_terrain.txt:344 |
| plains_17 | plains | 19 | 0 |  | yes | common/terrain/00_terrain.txt:345 |
| hills_blend | hills | 17 | 2 |  |  | common/terrain/00_terrain.txt:347 |
| mountain_variation_sand | mountain | 18 | 7 |  |  | common/terrain/00_terrain.txt:348 |
| mountain_variation_grass | mountain | 20 | 7 |  |  | common/terrain/00_terrain.txt:349 |
| jungle_18 | jungle | 21 | 4 |  |  | common/terrain/00_terrain.txt:350 |
| jungle_blend_18 | jungle | 22 | 5 |  |  | common/terrain/00_terrain.txt:351 |
| jungle_blend_18 | mountain | 27 | 7 |  |  | common/terrain/00_terrain.txt:353 |
| desert_mountain_tops | mountain | 31 | 15 |  |  | common/terrain/00_terrain.txt:354 |

## 4. common/ Graphics Sources

### common/defines

`common/defines` Lua files: `common/defines/00_defines.lua`, `common/defines/00_graphics.lua`, `common/defines/01_career_profile.lua`. `00_graphics.lua` starts `NDefines_Graphics` at `common/defines/00_graphics.lua:1` and `NMapMode` at `common/defines/00_graphics.lua:8`.
| Citation | Graphics constant evidence |
| --- | --- |
| common/defines/00_graphics.lua:8 | NMapMode = { |
| common/defines/00_graphics.lua:14 | CONSTRUCTION_MAP_MODE_BUILDING_DEFAULT_COLOR = { 0.43, 0.22, 0.22, 0.5 }, -- Color of states/provinces that can't be built on |
| common/defines/00_graphics.lua:25 | MAP_MODE_IDEOLOGY_COLOR_TRANSPARENCY = 1, -- In the Ideology map mode, the colors on the map are taken from the scriptable file 00_ideologies.txt for each group. We use them in the interfaces (pie-charts) where transparency is not used, but 100% opaque looks bad on the map. This is a variable to control the transparency of the color. |
| common/defines/00_graphics.lua:34 | PLAYER_MAPMODE_NOT_SELECTED_COUNTRY_TRANSPARENCY = 0.15, -- How much is the country colors faded out, for countries that are not occupied by the any player. |
| common/defines/00_graphics.lua:35 | SELECTED_COUNTRY_HIGHLIGHT_THICKNESS_MULT = 1.5, -- When a country is selected (blinking/highlighted) it's borders becomes a bit thicker, to make stand out even more. 1.0 is default thickness. |
| common/defines/00_graphics.lua:47 | AIR_RANGE_INDICATOR_DEFAULT_COLOR = { 1.0, 1.0, 0, 1 }, -- On map circle indicating the air wings range. |
| common/defines/00_graphics.lua:92 | NAVAL_HEADQUARTER_OFFSET = {1.5, 0, 1.5}, -- Offset from world position CVector3f (x, y, z). So it does not overlap with a port map icon or facility |
| common/defines/00_graphics.lua:989 | MAPICON_GROUP_PASSES = 20, -- how many mapicons get processed per frame for grouping. more = quicker response, fewer = better performance |
| common/defines/00_graphics.lua:59 | NAVAL_REGION_TRANSPARENCY_RANGE = 2000.0, -- The range of naval dominance to be rendered on the strategic navy region map |
| common/defines/00_graphics.lua:71 | RAILWAY_GUN_RANGE_INDICATOR_DEFAULT_COLOR = { 1.0, 1.0, 1.0, 1.0 }, -- On map circle indicating the railway gun bombardment range. |

The appendix fully tabulates 1238 assignments parsed from `00_graphics.lua` and all graphics-relevant matches in the other define files.

### Country Color Pipeline

`common/country_tags` files: `common/country_tags/00_countries.txt`, `common/country_tags/zz_dynamic_countries.txt`. `common/country_tags/00_countries.txt:1` maps GER to `countries/Germany.txt`. Germany declares graphical culture and base color at `common/countries/Germany.txt:3`-`common/countries/Germany.txt:6`.
`common/countries/colors.txt` contains 301 explicit color blocks; it begins with `#reload countrycolors` at `common/countries/colors.txt:1`. GER starts at `common/countries/colors.txt:3` and has map/UI color lines at `common/countries/colors.txt:4`-`common/countries/colors.txt:5`. No cosmetic-tag mapping file exists in `common/country_tags` in this install; color precedence is a gap.
| Color block | Block citation | Map color line | UI color line |
| --- | --- | --- | --- |
| GER | common/countries/colors.txt:3 | common/countries/colors.txt:4 | - |
| ENG | common/countries/colors.txt:7 | common/countries/colors.txt:8 | - |
| SOV | common/countries/colors.txt:11 | common/countries/colors.txt:12 | - |
| SWE | common/countries/colors.txt:15 | common/countries/colors.txt:16 | - |
| FRA | common/countries/colors.txt:19 | common/countries/colors.txt:20 | - |
| LUX | common/countries/colors.txt:23 | common/countries/colors.txt:24 | - |
| POL | common/countries/colors.txt:36 | common/countries/colors.txt:37 | - |
| AUS | common/countries/colors.txt:40 | common/countries/colors.txt:41 | - |
| LIT | common/countries/colors.txt:44 | common/countries/colors.txt:45 | - |
| EST | common/countries/colors.txt:48 | common/countries/colors.txt:49 | - |
| LAT | common/countries/colors.txt:52 | common/countries/colors.txt:53 | - |
| SPR | common/countries/colors.txt:56 | common/countries/colors.txt:57 | - |
| ITA | common/countries/colors.txt:60 | common/countries/colors.txt:61 | - |
| ROM | common/countries/colors.txt:64 | common/countries/colors.txt:65 | - |
| YUG | common/countries/colors.txt:68 | common/countries/colors.txt:69 | - |
| SWI | common/countries/colors.txt:72 | common/countries/colors.txt:73 | - |
| TUR | common/countries/colors.txt:76 | common/countries/colors.txt:77 | - |
| GRE | common/countries/colors.txt:80 | common/countries/colors.txt:81 | - |
| ALB | common/countries/colors.txt:84 | common/countries/colors.txt:85 | - |
| NOR | common/countries/colors.txt:88 | common/countries/colors.txt:89 | - |
| DEN | common/countries/colors.txt:92 | common/countries/colors.txt:93 | - |
| BUL | common/countries/colors.txt:96 | common/countries/colors.txt:97 | - |
| POR | common/countries/colors.txt:100 | common/countries/colors.txt:101 | - |
| FIN | common/countries/colors.txt:104 | common/countries/colors.txt:105 | - |
| IRE | common/countries/colors.txt:108 | common/countries/colors.txt:109 | - |
| HUN | common/countries/colors.txt:112 | common/countries/colors.txt:113 | - |
| AFG | common/countries/colors.txt:116 | common/countries/colors.txt:117 | - |
| ARG | common/countries/colors.txt:120 | common/countries/colors.txt:121 | - |
| AST | common/countries/colors.txt:124 | common/countries/colors.txt:125 | - |
| BHU | common/countries/colors.txt:128 | common/countries/colors.txt:129 | - |

### Buildings, Units, Ideologies, Focus Art

`common/buildings/00_buildings.txt` documents `show_on_map` at `common/buildings/00_buildings.txt:3`-`common/buildings/00_buildings.txt:5`. Parsed renderable building rows: 29.
| Building | show_on_map | show_on_map_meshes | show_on_map cite | mesh-count cite |
| --- | --- | --- | --- | --- |
| arms_factory | 6 | - | common/buildings/00_buildings.txt:56 | - |
| industrial_complex | 6 | - | common/buildings/00_buildings.txt:71 | - |
| air_base | 1 | - | common/buildings/00_buildings.txt:85 | - |
| supply_node | 1 | - | common/buildings/00_buildings.txt:99 | - |
| naval_facility | 1 | - | common/buildings/00_buildings.txt:127 | - |
| naval_base | 1 | 3 | common/buildings/00_buildings.txt:151 | common/buildings/00_buildings.txt:152 |
| bunker | 1 | - | common/buildings/00_buildings.txt:170 | - |
| coastal_bunker | 1 | - | common/buildings/00_buildings.txt:185 | - |
| dlc_allowed | 1 | - | common/buildings/00_buildings.txt:202 | - |
| dockyard | 1 | 3 | common/buildings/00_buildings.txt:226 | common/buildings/00_buildings.txt:227 |
| anti_air_building | 3 | - | common/buildings/00_buildings.txt:244 | - |
| synthetic_refinery | 1 | - | common/buildings/00_buildings.txt:259 | - |
| fuel_silo | 1 | - | common/buildings/00_buildings.txt:283 | - |
| radar_station | 1 | - | common/buildings/00_buildings.txt:303 | - |
| dlc_allowed | 1 | - | common/buildings/00_buildings.txt:320 | - |
| rocket_site | 1 | - | common/buildings/00_buildings.txt:339 | - |
| naval_supply_hub | 1 | - | common/buildings/00_buildings.txt:357 | - |
| dlc_allowed | 1 | - | common/buildings/00_buildings.txt:378 | - |
| missing_tech_loc | 1 | - | common/buildings/00_buildings.txt:409 | - |
| missing_tech_loc | 1 | - | common/buildings/00_buildings.txt:440 | - |
| missing_tech_loc | 1 | - | common/buildings/00_buildings.txt:471 | - |
| nuclear_facility | 1 | - | common/buildings/00_buildings.txt:501 | - |
| air_facility | 1 | - | common/buildings/00_buildings.txt:534 | - |
| land_facility | 1 | - | common/buildings/00_buildings.txt:557 | - |
| dam | 1 | - | common/buildings/00_buildings.txt:580 | - |
| dam_mountain | 1 | - | common/buildings/00_buildings.txt:608 | - |
| cataract_dam_mountain | 1 | - | common/buildings/00_buildings.txt:636 | - |
| canal_kiel | 1 | - | common/buildings/00_buildings.txt:664 | - |
| canal_panama | 1 | - | common/buildings/00_buildings.txt:687 | - |
`common/units` provides 204 `sprite = ...` lines for UI/counter keys. Examples: `common/units/air.txt:5` = `light_plane`; `common/units/air.txt:15` = `light_plane`; `common/units/air.txt:28` = `light_plane`; `common/units/air.txt:38` = `light_plane`; `common/units/air.txt:51` = `light_plane`; `common/units/air.txt:61` = `light_plane`.
`common/ideologies` contains ideology map colors; first parsed color line is `common/ideologies/00_ideologies.txt:30`. `common/defines/00_graphics.lua:25` says ideology map colors come from `00_ideologies.txt`.
`common/national_focus` contains 11089 `icon = GFX_*` UI art references. Examples: `common/national_focus/TSR_lingguang_incident_joint_branch.txt:3` = `GFX_focus_PER_fascist_reach_out_to_japan`; `common/national_focus/TSR_lingguang_incident_joint_branch.txt:65` = `GFX_focus_generic_support_the_left_right`; `common/national_focus/TSR_lingguang_incident_joint_branch.txt:123` = `GFX_focus_PRC_arms_recovery_efforts`; `common/national_focus/TSR_lingguang_incident_joint_branch.txt:185` = `GFX_focus_AST_war_japan`; `common/national_focus/TSR_lingguang_incident_joint_branch.txt:243` = `GFX_goal_generic_propaganda`; `common/national_focus/TSR_lingguang_incident_joint_branch.txt:300` = `GFX_focus_PRC_reorganize_divisions`.

## 5. gfx/ Outside FX

| gfx path | Files | Dirs | Extensions | Replay relevance |
| --- | --- | --- | --- | --- |
| gfx/3dviewenv | 13 | 0 | .txt 13 | graphics assets |
| gfx/FX | 58 | 0 | .shader 45, .fxh 13 | graphics assets |
| gfx/aces | 85 | 0 | .dds 85 | graphics assets |
| gfx/achievements | 882 | 0 | .dds 882 | graphics assets |
| gfx/army_icons | 13 | 0 | .dds 12, .txt 1 | graphics assets |
| gfx/cursors | 126 | 0 | .png 57, .cur 49, .ani 10, .ico 10 | graphics assets |
| gfx/entities | 53 | 0 | .gfx 27, .asset 26 | entity -> pdxmesh/state binding |
| gfx/event_pictures | 882 | 0 | .dds 882 | graphics assets |
| gfx/flags | 6617 | 2 | .tga 6617 | country flag TGA sets |
| gfx/fonts | 786 | 3 | .fnt 379, .dds 366, .tga 36, .txt 5 | bitmap fonts and pages |
| gfx/game_rules | 17 | 0 | .dds 17 | graphics assets |
| gfx/interface | 16962 | 180 | .dds 16878, .tga 40, .png 38, .txt 3, .info 1, .pdn 1, [no extension] 1 | UI/map icon art |
| gfx/keyicons | 70 | 0 | .dds 70 | graphics assets |
| gfx/leaders | 1130 | 124 | .dds 1129, .png 1 | graphics assets |
| gfx/loadingscreens | 19 | 0 | .dds 19 | excluded unless reproducing load UI |
| gfx/maparrows | 89 | 0 | .dds 88, .txt 1 | battle-plan/map-arrow textures/config |
| gfx/mapitems | 14 | 0 | .dds 12, .tga 2 | map item textures |
| gfx/minimap | 13 | 0 | .dds 13 | graphics assets |
| gfx/models | 2010 | 24 | .dds 984, .anim 535, .mesh 436, .asset 46, .gfx 8, .shader 1 | mesh/anim binaries plus material .asset binding |
| gfx/particles | 213 | 3 | .asset 141, .dds 70, .gfx 1, .txt 1 | particle assets/textures |
| gfx/texticons | 331 | 0 | .dds 331 | graphics assets |
| gfx/train_gfx_database | 1 | 0 | .txt 1 | graphics assets |
| gfx/world | 26 | 0 | .tga 26 | graphics assets |

### Text .asset Schema and Binary Model Headers

Parsed `213` .asset files under `gfx`. Entity assets use text `entity/name/pdxmesh/state` blocks; model assets use `pdxmesh` blocks with .mesh files and texture/material keys.
| Citation | Asset schema evidence |
| --- | --- |
| gfx/entities/buildings.asset:6 | entity = { |
| gfx/entities/buildings.asset:7 | name = "westerngfx_house_1_1" |
| gfx/entities/buildings.asset:8 | pdxmesh = "westerngfx_house_1_1" |
| gfx/entities/buildings.asset:28 | state = { name = "idle" |
| gfx/entities/particle_entities.asset:2 | entity = { |
| gfx/entities/particle_entities.asset:3 | name = "overrun_effect_2_entity" |
| gfx/entities/particle_entities.asset:70 | pdxmesh = "french_city_4_02" |
| gfx/entities/particle_entities.asset:6 | state = { name = "idle" state_time = 3 looping = no next_state = "idle" |
Mesh binaries: 436 .mesh; animation binaries: 535 .anim. Sample mesh `gfx/models/anim_blend_test/anim_blend_test_char.mesh`: first bytes `40 40 74 40 70 64 78 61 73 73 65 74 20 69 20 32`, ASCII `@@t@pdxasset i 2 1 0.[object]..[`, `pdxasset` marker offset 4. Sample anim `gfx/models/anim_blend_test/anim_blend_test_char_arm_out.anim`: first bytes `40 40 74 40 70 64 78 61 73 73 65 74 20 69 20 32`, ASCII `@@t@pdxasset i 2 1 0.[info]..fps`, `pdxasset` marker offset 4. Full decoding is a gap.

### Map Arrows, Particles, Flags, Fonts, Interface Art

`gfx/maparrows/maparrows.txt` binds arrows with `effect=` at `gfx/maparrows/maparrows.txt:3`; shipped FX source is `gfx/FX/maparrow.shader`. Arrow constants are `gfx/FX/constants.fxh:223`-`gfx/FX/constants.fxh:226`.
`gfx/particles` contains 213 files; `gfx/entities/particle_entities.asset` binds particle entities and `gfx/FX/particle.shader` renders them.
Flag naming is tag-sized `.tga` sets:
| Flag directory | TGA files | Sample names |
| --- | --- | --- |
| gfx/flags root | 2198 | ABK_communism.tga, ABK_democratic.tga, ABK_fascism.tga, ABK_neutrality.tga, ADU.tga |
| gfx/flags/medium | 2198 | ABK_communism.tga, ABK_democratic.tga, ABK_fascism.tga, ABK_neutrality.tga, ADU.tga |
| gfx/flags/small | 2221 | ABK_communism.tga, ABK_democratic.tga, ABK_fascism.tga, ABK_neutrality.tga, ADU.tga |

Parsed 379 .fnt bitmap-font files under `gfx/fonts`. Samples:
| Citation | info line | common/page line |
| --- | --- | --- |
| gfx/fonts/Arial12.fnt:1 | info face="Arial" size=14 bold=0 italic=0 charset="" unicode=1 stretchH=100 smooth=1 aa=1 padding=0,0,0,0 spacing=2,2 outline=0 | common lineHeight=14 base=11 scaleW=256 scaleH=128 pages=1 packed=0 alphaChnl=1 redChnl=0 greenChnl=0 blueChnl=0 |
| gfx/fonts/cg_16b.fnt:1 | info face="Century Gothic" size=16 bold=1 italic=0 charset="" unicode=1 stretchH=100 smooth=1 aa=2 padding=3,3,3,3 spacing=1,1 outline=0 | common lineHeight=16 base=13 scaleW=512 scaleH=256 pages=1 packed=0 alphaChnl=1 redChnl=0 greenChnl=0 blueChnl=0 |
| gfx/fonts/cg_18b.fnt:1 | info face="Century Gothic" size=18 bold=1 italic=0 charset="" unicode=1 stretchH=100 smooth=1 aa=2 padding=3,3,3,3 spacing=1,1 outline=0 | common lineHeight=18 base=15 scaleW=512 scaleH=512 pages=1 packed=0 alphaChnl=1 redChnl=0 greenChnl=0 blueChnl=0 |
| gfx/fonts/chinese/Arial12_1of4.fnt:1 | info face="Arial12_1of4" size=12 bold=0 italic=0 charset="" unicode=1 stretchH=100 smooth=1 aa=1 padding=1,1,1,1 spacing=1,1 | common lineHeight=16 base=14 scaleW=1400 scaleH=1024 pages=1 packed=0 |
| gfx/fonts/chinese/Arial12_2of4.fnt:1 | info face="Arial12_2of4" size=12 bold=0 italic=0 charset="" unicode=1 stretchH=100 smooth=1 aa=1 padding=1,1,1,1 spacing=1,1 | common lineHeight=16 base=14 scaleW=1600 scaleH=1024 pages=1 packed=0 |
| gfx/fonts/chinese/Arial12_3of4.fnt:1 | info face="Arial12_3of4" size=12 bold=0 italic=0 charset="" unicode=1 stretchH=100 smooth=1 aa=1 padding=1,1,1,1 spacing=1,1 | common lineHeight=16 base=14 scaleW=1600 scaleH=1024 pages=1 packed=0 |
| gfx/fonts/chinese/Arial12_4of4.fnt:1 | info face="Arial12_4of4" size=12 bold=0 italic=0 charset="" unicode=1 stretchH=100 smooth=1 aa=1 padding=1,1,1,1 spacing=1,1 | common lineHeight=16 base=14 scaleW=1400 scaleH=1024 pages=1 packed=0 |
| gfx/fonts/chinese/cg_16b_1of4.fnt:1 | info face="cg_16b_1of4" size=12 bold=0 italic=0 charset="" unicode=1 stretchH=100 smooth=1 aa=1 padding=1,1,1,1 spacing=1,1 | common lineHeight=12 base=14 scaleW=2048 scaleH=1600 pages=1 packed=0 |
| gfx/fonts/chinese/cg_16b_2of4.fnt:1 | info face="cg_16b_2of4" size=12 bold=0 italic=0 charset="" unicode=1 stretchH=100 smooth=1 aa=1 padding=1,1,1,1 spacing=1,1 | common lineHeight=12 base=14 scaleW=2048 scaleH=1600 pages=1 packed=0 |
| gfx/fonts/chinese/cg_16b_3of4.fnt:1 | info face="cg_16b_3of4" size=12 bold=0 italic=0 charset="" unicode=1 stretchH=100 smooth=1 aa=1 padding=1,1,1,1 spacing=1,1 | common lineHeight=12 base=14 scaleW=2048 scaleH=1800 pages=1 packed=0 |
| gfx/fonts/chinese/cg_16b_4of4.fnt:1 | info face="cg_16b_4of4" size=12 bold=0 italic=0 charset="" unicode=1 stretchH=100 smooth=1 aa=1 padding=1,1,1,1 spacing=1,1 | common lineHeight=12 base=14 scaleW=2048 scaleH=1600 pages=1 packed=0 |
| gfx/fonts/chinese/cg_18b_1of4.fnt:1 | info face="cg_18b_1of4" size=15 bold=0 italic=0 charset="" unicode=1 stretchH=100 smooth=1 aa=1 padding=1,1,1,1 spacing=1,1 | common lineHeight=15 base=17 scaleW=2048 scaleH=1040 pages=1 packed=0 |

`gfx/interface` is the major UI art tree; map-related subdirectories include `gfx/interface/counters`, `gfx/interface/mapicons`, `gfx/interface/mapmode`, and `gfx/interface/unitcontrol`.

## 6. interface/ Map UI Atlas

`interface` contains 162 .gfx files and 453 .gui files. These are the authoritative UI chrome/control layer beside `gfx/interface` art.
| Interface file | Map/UI evidence |
| --- | --- |
| interface/battleplantools.gfx | interface/battleplantools.gfx:1 spriteTypes = { |
| interface/battleplantools.gui | interface/battleplantools.gui:19 spriteType = "GFX_BPT_bg" |
| interface/core.gfx | interface/core.gfx:713 animatedmaptext = { |
| interface/core_chinese.gfx | interface/core_chinese.gfx:15 name = "AnimatedMapText" |
| interface/core_korean.gfx | interface/core_korean.gfx:12 name = "AnimatedMapText" |
| interface/countryarmyview.gui | interface/countryarmyview.gui:11 spriteType = "GFX_overview_filter_entry" |
| interface/countrystateview.gui | interface/countrystateview.gui:81 spriteType = "GFX_victorypoint_stateview_bg" |
| interface/mapicons.gfx | interface/mapicons.gfx:2 spriteTypes = { |
| interface/mapicons.gui | interface/mapicons.gui:31 spriteType = "GFX_UV_group_marker" |
| interface/mapmodes_interface.gfx | interface/mapmodes_interface.gfx:1 spriteTypes = { |
| interface/mapmodes_interface.gui | interface/mapmodes_interface.gui:27 spriteType = "GFX_socialview_icon" |
| interface/minimap.gui | interface/minimap.gui:5 spriteType = "GFX_minimap_ping_offensive" |
| interface/strategicairview.gfx | interface/strategicairview.gfx:1 spriteTypes = { |
| interface/strategicairview.gui | interface/strategicairview.gui:10 spriteType = "GFX_damage_types_strip" |
| interface/subuniticons.gfx | interface/subuniticons.gfx:1 spriteTypes = { |
| interface/topbar.gui | interface/topbar.gui:10 spriteType = "GFX_topbar_background_extended" |
| interface/unitview.gfx | interface/unitview.gfx:1 spriteTypes = { |
| interface/unitview.gui | interface/unitview.gui:57 spriteType = "GFX_hue_slider_background" |

Map labels: `interface/core.gfx:713` starts animated map text declarations; `interface/core.gfx:1680` points to `gfx/fonts/hoi_mapfont4`. Localisation supplies country/state/victory-point strings.

## 7. DLC, integrated_dlc, checksum, localisation

`dlc` and `integrated_dlc` were scanned as extension/override sources. Pack rows are in the appendix; content classes include interface art/gui, entities, models, particles, flags, fonts, common text/lua, map, localisation, archives, and audio depending on pack.

| Citation | Directory | Subdirectories | Extension |
| --- | --- | --- | --- |
| checksum_manifest.txt:2 | common | yes | .txt |
| checksum_manifest.txt:7 | common | yes | .lua |
| checksum_manifest.txt:12 | events | yes | .txt |
| checksum_manifest.txt:17 | history | yes | .txt |
| checksum_manifest.txt:22 | map | yes | .txt |
| checksum_manifest.txt:27 | map | yes | .map |
| checksum_manifest.txt:32 | map | yes | .bmp |
| checksum_manifest.txt:37 | map | yes | .csv |

Localisation was scanned only where it feeds map text. Candidate files: 80. Examples: `localisation/braz_por/countries_cosmetic_l_braz_por.yml`, `localisation/braz_por/countries_l_braz_por.yml`, `localisation/braz_por/state_l_braz_por.yml`, `localisation/braz_por/state_names_l_braz_por.yml`, `localisation/braz_por/strategic_region_names_l_braz_por.yml`, `localisation/braz_por/terrain_l_braz_por.yml`, `localisation/braz_por/togetherforvictory_music_l_braz_por.yml`, `localisation/braz_por/victory_points_l_braz_por.yml`, `localisation/english/countries_cosmetic_l_english.yml`, `localisation/english/countries_l_english.yml`, `localisation/english/state_l_english.yml`, `localisation/english/state_names_l_english.yml`.

## 8. Replay Feature -> Authoritative Shipped Files

| Replay feature | Authoritative shipped files |
| --- | --- |
| Terrain colors/atlas | `map/terrain.bmp`; `common/terrain/00_terrain.txt:320`; `map/terrain/atlas*.dds`; `map/terrain/atlas_normal*.dds`; `gfx/FX/pdxmap.shader`; `gfx/FX/constants.fxh:53`-`gfx/FX/constants.fxh:59` |
| Water | `gfx/FX/pdxwater.shader`; `gfx/FX/standardfuncsgfx.fxh` water functions; `gfx/FX/constants.fxh:128`-`gfx/FX/constants.fxh:135`; `map/terrain/colormap_water_*.dds`; `fow_rgb_waterspec_a.dds`; `lean1.dds`; `lean2.dds`; `reflection.dds` |
| Rivers and straits | `map/rivers.bmp`; `gfx/FX/river.shader`; `gfx/FX/strait.shader`; `map/terrain/RiverSurface*.dds`; `map/adjacencies.csv:1`; `map/adjacency_rules.txt:1`-`map/adjacency_rules.txt:12` |
| Trees | `map/trees.bmp`; `map/default.map:16`; `map/terrain/Tree_season.bmp`; `map/terrain/Tree_tint.bmp`; `gfx/FX/tree.shader`; `gfx/FX/constants.fxh:113`-`gfx/FX/constants.fxh:125` |
| Borders | `map/provinces.bmp`; `map/definition.csv:1`; `gfx/FX/border.shader`; `map/terrain/border_*.dds`; `gfx/FX/standardfuncsgfx.fxh`; `gfx/FX/constants.fxh:195`-`gfx/FX/constants.fxh:210` |
| Political fill + zoom law | `common/countries/*.txt`; `common/countries/colors.txt:1` and `common/countries/colors.txt:3`; `gfx/FX/constants.fxh:195`-`gfx/FX/constants.fxh:210`; `gfx/FX/standardfuncsgfx.fxh` gradient-border functions |
| Snow/ice/mud/weather | `map/seasons.txt:63`/`map/seasons.txt:80`/`map/seasons.txt:95`/`map/seasons.txt:110`; `map/strategicregions/*.txt`; `map/weatherpositions.txt:1`; `gfx/FX/constants.fxh:61`-`gfx/FX/constants.fxh:88`; snow/ice/mud DDS files |
| Day-night and city lights | `gfx/FX/constants.fxh:14`-`gfx/FX/constants.fxh:37`; `gfx/FX/constants.fxh:49`-`gfx/FX/constants.fxh:51`; `map/terrain/citylights_rgb_snowmask_a_*.dds`; `map/terrain/colormap_rgb_cityemissivemask_a.dds`; `map/cities.bmp`; `map/cities.txt:1` |
| Buildings on map | `common/buildings/00_buildings.txt:3`-`common/buildings/00_buildings.txt:5`; `map/buildings.txt:1`; `gfx/entities/buildings.asset`; `gfx/models/buildings/*.asset`; `.mesh`; `map/cities.txt:6` |
| Units on map | `map/unitstacks.txt:1`; `common/units/*.txt` sprite refs; `gfx/entities/units*.asset`; `gfx/models/units`; `gfx/FX/pdxmesh.shader`; `interface/unitview.*`; `gfx/interface/counters` |
| Railways/supply | `map/railways.txt:1`; `map/supply_nodes.txt:1`; `map/supplyareas/*.txt`; `common/defines/00_graphics.lua:94`-`common/defines/00_graphics.lua:120`; `interface/mapicons.*`; root `railway_mapicon_bg.dds` |
| Map text labels | localisation `.yml`; `interface/core.gfx` animated map text; `gfx/fonts/hoi_mapfont4`; `gfx/FX/mapname.shader` |
| Unit counters | `common/units/*.txt`; `interface/subuniticons.gfx`; `interface/unitview.gfx`; `gfx/interface/counters` |
| Flags | `gfx/flags/*.tga`; `gfx/flags/medium/*.tga`; `gfx/flags/small/*.tga`; `common/country_tags/00_countries.txt:1` |
| Fonts | `gfx/fonts/*.fnt`; `gfx/fonts/*.dds/.tga`; `interface/core.gfx` font registrations |
| UI chrome | `interface/*.gui`; `interface/*.gfx`; `gfx/interface/**`; `gfx/FX/buttonstate*.shader`; `gfx/FX/text.shader` |
| Particles/effects | `gfx/particles/**`; `gfx/entities/particle_entities.asset`; `gfx/FX/particle.shader`; `posteffect_base.fxh`; `bloom.shader`; `lut_blender.shader` |
| Arrows and battle plans | `gfx/maparrows/maparrows.txt`; `gfx/maparrows/*.dds`; `gfx/FX/maparrow.shader`; `gfx/FX/arrow.shader`; `gfx/FX/constants.fxh:223`-`gfx/FX/constants.fxh:226`; `interface/battleplantools.*`; `common/defines/00_graphics.lua:84`-`common/defines/00_graphics.lua:86` |

## 9. Static-File Gaps

| Gap | Best static conclusion / runtime evidence needed |
| --- | --- |
| Country color precedence | Static files show per-country `color` and `common/countries/colors.txt`, but no cosmetic tag mapping file in `common/country_tags`; runtime evidence is needed for exact precedence. |
| Headerless map schemas | Several map files contain data without in-file column declarations; exact final-column names require engine/modding docs or nudger/runtime validation. |
| Rivers.bmp palette semantics | BMP header and binding are static; not every palette index meaning is named in shipped text. Runtime comparison or official docs are needed. |
| Heightmap/world-normal scaling | Headers give dimensions and bit depth, but numeric world elevation scaling and normal-map reconstruction are not declared in text files. Runtime/shader inspection beyond headers would settle it. |
| .mesh/.anim decoding | Headers and `pdxasset` markers are measured; full chunk decoding needs a dedicated decoder or engine tooling. |
| Map arrow effect extension | `maparrows.txt` names `gfx/FX/maparrow.lua` while shipped source is `.shader`; resolver alias is engine behavior. |
| Map label curving/layout | Static files show fonts, animated map text, mapname shader, and localisation; precise label pathing/collision appears engine-side. |
| GUI data binding | `.gui`/`.gfx` define sprites/layout; runtime game-state binding remains engine behavior. |
| DLC/mod override order | Static packs are visible; active DLC/mod resolution needs launcher/runtime active content state. |

Appendix: `GAME_FILES_ATLAS_APPENDIX.md` contains one-level inventory, full `constants.fxh` table, full `00_graphics.lua` assignment table, shader-function samples, map/terrain image headers, DLC content-class rows, and additional counts.
