//! Explicitly unshipped backend for the historical inline-hook evidence graph.
//!
//! The production recorder never compiles or calls this module.

use super::*;
use minhook::{MinHook, MH_STATUS};

pub(super) type HookSpec = (usize, *mut c_void, &'static AtomicUsize);

unsafe fn create_hook(target: usize, detour: *mut c_void) -> Result<usize, String> {
    MinHook::create_hook(target as *mut c_void, detour)
        .map(|original| original as usize)
        .map_err(|status| status.to_string())
}

unsafe fn enable_hook(target: usize) -> Result<(), String> {
    MinHook::enable_hook(target as *mut c_void).map_err(|status| status.to_string())
}

unsafe fn disable_hook_if_present(target: usize) -> Result<(), String> {
    match MinHook::disable_hook(target as *mut c_void) {
        Ok(()) | Err(MH_STATUS::MH_ERROR_DISABLED) => Ok(()),
        Err(status) => Err(status.to_string()),
    }
}

unsafe fn remove_hook_if_present(target: usize) -> Result<(), String> {
    match MinHook::remove_hook(target as *mut c_void) {
        Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => Ok(()),
        Err(status) => Err(status.to_string()),
    }
}

fn disable_staged_hooks(targets: &[usize]) -> Result<(), String> {
    let failures = targets
        .iter()
        .copied()
        .filter_map(|target| {
            unsafe { disable_hook_if_present(target) }
                .err()
                .map(|status| format!("{target:#x} staged hook disable failed: {status}"))
        })
        .collect::<Vec<_>>();
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join(", "))
    }
}

fn remove_staged_hooks(targets: &[usize]) -> Result<(), String> {
    let failures = targets
        .iter()
        .copied()
        .filter_map(|target| {
            unsafe { remove_hook_if_present(target) }
                .err()
                .map(|status| format!("{target:#x} staged hook removal failed: {status}"))
        })
        .collect::<Vec<_>>();
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join(", "))
    }
}

pub(super) fn begin_staged_hook_teardown(targets: &[usize]) {
    if !targets.is_empty() {
        begin_com_detour_teardown();
    }
}

pub(super) fn terminalize_staged_hooks(targets: &[usize]) -> Result<(), String> {
    disable_staged_hooks(targets)?;
    if !targets.is_empty() {
        if let Err(error) = wait_for_com_detour_quiescence() {
            let mut message = error;
            if let Err(teardown_error) = crate::teardown_gui_paint_instrumentation() {
                message.push_str(&format!(
                    "; GUI paint instrumentation teardown failed: {teardown_error}"
                ));
            }
            return Err(message);
        }
    }
    if let Err(error) = remove_staged_hooks(targets) {
        let mut failures = vec![error];
        if let Err(teardown_error) = crate::teardown_gui_paint_instrumentation() {
            failures.push(format!(
                "GUI paint instrumentation teardown failed: {teardown_error}"
            ));
        }
        return Err(failures.join(", "));
    }
    Ok(())
}

pub(super) fn clear_staged_hook_state() {
    let had_staged_hooks = STAGED_HOOK_TARGETS.get().is_some_and(|targets| {
        !targets
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty()
    });
    for value in [
        &ORIGINAL_VFS_OPEN_CONSTRUCTOR,
        &ORIGINAL_WINDOW_BACKBUFFER_RTV_PRODUCER,
        &ORIGINAL_OPAQUE_TEXTURE_TARGET_PRODUCER,
        &ORIGINAL_TEXTURE_CACHE_LOAD_ROOT,
        &ORIGINAL_ENVELOPE,
        &ORIGINAL_OBJECT_CTOR,
        &ORIGINAL_BUCKET,
        &ORIGINAL_CREATE,
        &ORIGINAL_DESTROY,
        &ORIGINAL_BUILDER,
        &ORIGINAL_PARTICLE_RENDER_SUBMIT,
        &ORIGINAL_PARTICLE_FINAL_DRAW,
        &ORIGINAL_ENTITY_FACTORY,
        &ORIGINAL_ENTITY_NAMED_FACTORY,
        &ORIGINAL_ENTITY_DEFINITION_LOOKUP,
        &ORIGINAL_ENTITY_DESTROY,
        &rail_construction_arrow::ORIGINAL_PRODUCER,
        &rail_supply_arrow::ORIGINAL_ROW_PRODUCER,
        &rail_supply_arrow::ORIGINAL_SLOT_SELECTOR,
        &s1_pdxmesh_caster::ORIGINAL_PRODUCER,
        &s1_tree_caster::ORIGINAL_PRODUCER,
        &ORIGINAL_ENTITY_RENDER,
        &ORIGINAL_ENTITY_FINAL_DRAW,
        &ORIGINAL_INDEXED_INSTANCED_FINAL_DRAW,
        &ORIGINAL_INSTANCED_FINAL_DRAW,
        &ORIGINAL_RENDER_TARGET_CLEAR,
        &ORIGINAL_SHADER_PROGRAM_BINDER,
        &ORIGINAL_ENGINE_SHADER_EFFECT_LOOKUP,
        &ORIGINAL_MAP_EFFECT_REGISTRATION_ROOT,
        &ORIGINAL_RAILROAD_EFFECT_REGISTRATION_ROOT,
        &ORIGINAL_MAP_NAME_EFFECT_REGISTRATION_ROOT,
        &ORIGINAL_RESTORE_BLOOM_EFFECT_REGISTRATION_ROOT,
        &ORIGINAL_LUT_BLEND_EFFECT_REGISTRATION_ROOT,
        &ORIGINAL_WEATHER_NORMAL_UPDATE_OWNER,
        &ORIGINAL_WEATHER_EDITOR_REFRESH_OWNER,
        &ORIGINAL_WEATHER,
        &ORIGINAL_WEATHER_ACTIVE_SETTER,
        &ORIGINAL_WEATHER_ALPHA_SETTER,
        &ORIGINAL_WEATHER_PLAYBACK_SETTER,
        &ORIGINAL_SEED,
        &ORIGINAL_MAP_SCENE_RENDER_ROOT,
        &ORIGINAL_POST_PROCESS_RENDER_ROOT,
        &ORIGINAL_PRESENT_RENDER_ROOT,
        &ORIGINAL_SNOW_MUD_PRODUCER,
        &ORIGINAL_SNOW_MUD_UPLOAD,
        &ORIGINAL_CAMERA_WHEEL_UPDATE,
        &ORIGINAL_CAMERA_VERTICAL_EVENT_PRODUCER,
        &ORIGINAL_CAMERA_MAP_PICK,
        &o05_building_capture::ORIGINAL_UPDATE_PROVINCES,
        &o05_building_capture::ORIGINAL_BUILDING_CONSTRUCTOR,
        &o05_building_capture::ORIGINAL_BUILDING_VECTOR_UPDATE,
        &o05_building_capture::ORIGINAL_ENTITY_TRANSFORM_SETTER,
        &ORIGINAL_D3D_CREATE_INPUT_LAYOUT,
        &ORIGINAL_D3D_CREATE_VERTEX_SHADER,
        &ORIGINAL_D3D_CREATE_PIXEL_SHADER,
        &ORIGINAL_D3D_CREATE_GEOMETRY_SHADER,
        &ORIGINAL_D3D_CREATE_HULL_SHADER,
        &ORIGINAL_D3D_CREATE_DOMAIN_SHADER,
        &ORIGINAL_D3D_CREATE_COMPUTE_SHADER,
        &ORIGINAL_D3D_CREATE_BLEND_STATE,
        &ORIGINAL_D3D_CREATE_DEPTH_STENCIL_STATE,
        &ORIGINAL_D3D_CREATE_RASTERIZER_STATE,
        &ORIGINAL_D3D_CREATE_SAMPLER_STATE,
        &ORIGINAL_D3D_CREATE_BUFFER,
        &ORIGINAL_D3D_CREATE_TEXTURE1D,
        &ORIGINAL_D3D_CREATE_TEXTURE2D,
        &ORIGINAL_D3D_CREATE_TEXTURE3D,
        &ORIGINAL_D3D_CREATE_SRV,
        &ORIGINAL_D3D_CREATE_RTV,
        &ORIGINAL_D3D_CREATE_DSV,
        &ORIGINAL_D3D_SO_SET_TARGETS,
    ] {
        value.store(0, Ordering::Release);
    }
    if had_staged_hooks {
        finish_com_detour_teardown();
    }
    if let Some(targets) = STAGED_HOOK_TARGETS.get() {
        targets
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }
}

fn actual_hardware_d3d_hook_specs(device: usize, context: usize) -> Result<Vec<HookSpec>, String> {
    if device == 0 || context == 0 {
        return Err("successful game D3D creation returned a null device or context".into());
    }
    let device_method = |slot, label| {
        com_method(device, slot)
            .ok_or_else(|| format!("actual hardware {label} vtable slot is null"))
    };
    let context_method = |slot, label| {
        com_method(context, slot)
            .ok_or_else(|| format!("actual hardware {label} vtable slot is null"))
    };
    let specs = vec![
        (
            device_method(11, "CreateInputLayout")?,
            hook_d3d_create_input_layout as *mut c_void,
            &ORIGINAL_D3D_CREATE_INPUT_LAYOUT,
        ),
        (
            device_method(12, "CreateVertexShader")?,
            hook_d3d_create_vertex_shader as *mut c_void,
            &ORIGINAL_D3D_CREATE_VERTEX_SHADER,
        ),
        (
            device_method(15, "CreatePixelShader")?,
            hook_d3d_create_pixel_shader as *mut c_void,
            &ORIGINAL_D3D_CREATE_PIXEL_SHADER,
        ),
        (
            device_method(13, "CreateGeometryShader")?,
            hook_d3d_create_geometry_shader as *mut c_void,
            &ORIGINAL_D3D_CREATE_GEOMETRY_SHADER,
        ),
        (
            device_method(16, "CreateHullShader")?,
            hook_d3d_create_hull_shader as *mut c_void,
            &ORIGINAL_D3D_CREATE_HULL_SHADER,
        ),
        (
            device_method(17, "CreateDomainShader")?,
            hook_d3d_create_domain_shader as *mut c_void,
            &ORIGINAL_D3D_CREATE_DOMAIN_SHADER,
        ),
        (
            device_method(18, "CreateComputeShader")?,
            hook_d3d_create_compute_shader as *mut c_void,
            &ORIGINAL_D3D_CREATE_COMPUTE_SHADER,
        ),
        (
            device_method(20, "CreateBlendState")?,
            hook_d3d_create_blend_state as *mut c_void,
            &ORIGINAL_D3D_CREATE_BLEND_STATE,
        ),
        (
            device_method(21, "CreateDepthStencilState")?,
            hook_d3d_create_depth_stencil_state as *mut c_void,
            &ORIGINAL_D3D_CREATE_DEPTH_STENCIL_STATE,
        ),
        (
            device_method(22, "CreateRasterizerState")?,
            hook_d3d_create_rasterizer_state as *mut c_void,
            &ORIGINAL_D3D_CREATE_RASTERIZER_STATE,
        ),
        (
            device_method(23, "CreateSamplerState")?,
            hook_d3d_create_sampler_state as *mut c_void,
            &ORIGINAL_D3D_CREATE_SAMPLER_STATE,
        ),
        (
            device_method(3, "CreateBuffer")?,
            hook_d3d_create_buffer as *mut c_void,
            &ORIGINAL_D3D_CREATE_BUFFER,
        ),
        (
            device_method(4, "CreateTexture1D")?,
            hook_d3d_create_texture1d as *mut c_void,
            &ORIGINAL_D3D_CREATE_TEXTURE1D,
        ),
        (
            device_method(5, "CreateTexture2D")?,
            hook_d3d_create_texture2d as *mut c_void,
            &ORIGINAL_D3D_CREATE_TEXTURE2D,
        ),
        (
            device_method(6, "CreateTexture3D")?,
            hook_d3d_create_texture3d as *mut c_void,
            &ORIGINAL_D3D_CREATE_TEXTURE3D,
        ),
        (
            device_method(7, "CreateShaderResourceView")?,
            hook_d3d_create_srv as *mut c_void,
            &ORIGINAL_D3D_CREATE_SRV,
        ),
        (
            device_method(9, "CreateRenderTargetView")?,
            hook_d3d_create_rtv as *mut c_void,
            &ORIGINAL_D3D_CREATE_RTV,
        ),
        (
            device_method(10, "CreateDepthStencilView")?,
            hook_d3d_create_dsv as *mut c_void,
            &ORIGINAL_D3D_CREATE_DSV,
        ),
        (
            context_method(37, "SOSetTargets")?,
            hook_d3d_so_set_targets as *mut c_void,
            &ORIGINAL_D3D_SO_SET_TARGETS,
        ),
    ];
    debug_assert_eq!(specs.len(), HARDWARE_D3D_HOOK_COUNT);
    Ok(specs)
}

fn rollback_created_hooks(specs: &[HookSpec], created: usize) -> Result<(), String> {
    let mut failures = Vec::new();
    let includes_com = specs
        .iter()
        .take(created)
        .any(|(_, _, original)| is_com_original(original));
    let includes_a25_gui_bridge = specs
        .iter()
        .take(created)
        .any(|(_, _, original)| a25_gui_bridge::owns_original_slot(original));
    if includes_com {
        begin_com_detour_teardown();
    }
    if includes_a25_gui_bridge {
        a25_gui_bridge::begin_teardown();
    }
    let mut disable_failed = false;
    for (target, _, _) in specs.iter().take(created).rev() {
        if let Err(status) = unsafe { disable_hook_if_present(*target) } {
            disable_failed = true;
            failures.push(format!("disable {target:#x}: {status}"));
        }
    }
    let mut quiescence_failed = false;
    if includes_com {
        if let Err(error) = wait_for_com_detour_quiescence() {
            quiescence_failed = true;
            failures.push(error);
        }
    }
    if includes_a25_gui_bridge {
        if let Err(error) = a25_gui_bridge::wait_for_quiescence() {
            quiescence_failed = true;
            failures.push(error);
        }
    }
    if disable_failed || quiescence_failed {
        return Err(format!("hook rollback failed: {}", failures.join(", ")));
    }
    let mut removal_failed = false;
    for (target, _, _) in specs.iter().take(created).rev() {
        if let Err(status) = unsafe { remove_hook_if_present(*target) } {
            removal_failed = true;
            failures.push(format!("remove {target:#x}: {status}"));
        }
    }
    if !removal_failed && !quiescence_failed {
        clear_created_originals(specs, created);
        if includes_com {
            finish_com_detour_teardown();
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!("hook rollback failed: {}", failures.join(", ")))
    }
}

pub(super) fn clear_created_originals(specs: &[HookSpec], created: usize) {
    for (_, _, original) in specs.iter().take(created) {
        original.store(0, Ordering::Release);
    }
}

fn create_and_enable_hooks(specs: &[HookSpec], label: &str) -> Result<Vec<usize>, String> {
    let mut created = 0usize;
    for (target, detour, original) in specs {
        match unsafe { create_hook(*target, *detour) } {
            Ok(value) => {
                original.store(value, Ordering::Release);
                created += 1;
            }
            Err(status) => {
                let rollback = rollback_created_hooks(specs, created)
                    .err()
                    .map(|error| format!("; {error}"))
                    .unwrap_or_default();
                return Err(format!(
                    "{label} hook {target:#x} creation failed: {status}{rollback}"
                ));
            }
        }
    }
    for (index, (target, _, _)) in specs.iter().enumerate() {
        if let Err(status) = unsafe { enable_hook(*target) } {
            let rollback = rollback_created_hooks(specs, created)
                .err()
                .map(|error| format!("; {error}"))
                .unwrap_or_default();
            return Err(format!(
                "{label} hook {target:#x} enable failed at staged index {index}: {status}{rollback}"
            ));
        }
    }
    Ok(specs.iter().map(|(target, _, _)| *target).collect())
}

pub(super) fn engine_hook_specs(a: InstalledAddresses) -> [HookSpec; ENGINE_HOOK_COUNT] {
    let detours = [
        hook_vfs_open_constructor as *mut c_void,
        hook_window_backbuffer_rtv_producer as *mut c_void,
        hook_opaque_texture_target_producer as *mut c_void,
        hook_texture_cache_load_root as *mut c_void,
        hook_envelope as *mut c_void,
        hook_object_ctor as *mut c_void,
        hook_bucket as *mut c_void,
        hook_create as *mut c_void,
        hook_destroy as *mut c_void,
        hook_builder as *mut c_void,
        hook_particle_render_submit as *mut c_void,
        hook_particle_final_draw as *mut c_void,
        hook_entity_factory as *mut c_void,
        hook_entity_named_factory as *mut c_void,
        hook_entity_definition_lookup as *mut c_void,
        hook_entity_destroy as *mut c_void,
        rail_construction_arrow::detour(),
        rail_supply_arrow::row_producer_detour(),
        rail_supply_arrow::slot_selector_detour(),
        s1_pdxmesh_caster::detour(),
        s1_tree_caster::detour(),
        hook_entity_render as *mut c_void,
        hook_entity_final_draw as *mut c_void,
        hook_indexed_instanced_final_draw as *mut c_void,
        hook_instanced_final_draw as *mut c_void,
        hook_render_target_clear as *mut c_void,
        hook_shader_program_binder as *mut c_void,
        hook_engine_shader_effect_lookup as *mut c_void,
        hook_map_effect_registration_root as *mut c_void,
        hook_railroad_effect_registration_root as *mut c_void,
        hook_map_name_effect_registration_root as *mut c_void,
        hook_restore_bloom_effect_registration_root as *mut c_void,
        hook_lut_blend_effect_registration_root as *mut c_void,
        hook_weather_normal_update_owner as *mut c_void,
        hook_weather_editor_refresh_owner as *mut c_void,
        hook_weather as *mut c_void,
        hook_weather_active_setter as *mut c_void,
        hook_weather_alpha_setter as *mut c_void,
        hook_weather_playback_setter as *mut c_void,
        hook_seed as *mut c_void,
        hook_map_scene_render_root as *mut c_void,
        hook_post_process_render_root as *mut c_void,
        hook_present_render_root as *mut c_void,
        hook_snow_mud_producer as *mut c_void,
        hook_snow_mud_upload as *mut c_void,
        hook_camera_wheel_update as *mut c_void,
        hook_camera_vertical_event_producer as *mut c_void,
        hook_camera_map_pick as *mut c_void,
        o05_building_capture::hook_update_provinces as *mut c_void,
        o05_building_capture::hook_building_constructor as *mut c_void,
        o05_building_capture::hook_building_vector_update as *mut c_void,
        o05_building_capture::hook_entity_transform_setter as *mut c_void,
    ];
    let originals = [
        &ORIGINAL_VFS_OPEN_CONSTRUCTOR,
        &ORIGINAL_WINDOW_BACKBUFFER_RTV_PRODUCER,
        &ORIGINAL_OPAQUE_TEXTURE_TARGET_PRODUCER,
        &ORIGINAL_TEXTURE_CACHE_LOAD_ROOT,
        &ORIGINAL_ENVELOPE,
        &ORIGINAL_OBJECT_CTOR,
        &ORIGINAL_BUCKET,
        &ORIGINAL_CREATE,
        &ORIGINAL_DESTROY,
        &ORIGINAL_BUILDER,
        &ORIGINAL_PARTICLE_RENDER_SUBMIT,
        &ORIGINAL_PARTICLE_FINAL_DRAW,
        &ORIGINAL_ENTITY_FACTORY,
        &ORIGINAL_ENTITY_NAMED_FACTORY,
        &ORIGINAL_ENTITY_DEFINITION_LOOKUP,
        &ORIGINAL_ENTITY_DESTROY,
        &rail_construction_arrow::ORIGINAL_PRODUCER,
        &rail_supply_arrow::ORIGINAL_ROW_PRODUCER,
        &rail_supply_arrow::ORIGINAL_SLOT_SELECTOR,
        &s1_pdxmesh_caster::ORIGINAL_PRODUCER,
        &s1_tree_caster::ORIGINAL_PRODUCER,
        &ORIGINAL_ENTITY_RENDER,
        &ORIGINAL_ENTITY_FINAL_DRAW,
        &ORIGINAL_INDEXED_INSTANCED_FINAL_DRAW,
        &ORIGINAL_INSTANCED_FINAL_DRAW,
        &ORIGINAL_RENDER_TARGET_CLEAR,
        &ORIGINAL_SHADER_PROGRAM_BINDER,
        &ORIGINAL_ENGINE_SHADER_EFFECT_LOOKUP,
        &ORIGINAL_MAP_EFFECT_REGISTRATION_ROOT,
        &ORIGINAL_RAILROAD_EFFECT_REGISTRATION_ROOT,
        &ORIGINAL_MAP_NAME_EFFECT_REGISTRATION_ROOT,
        &ORIGINAL_RESTORE_BLOOM_EFFECT_REGISTRATION_ROOT,
        &ORIGINAL_LUT_BLEND_EFFECT_REGISTRATION_ROOT,
        &ORIGINAL_WEATHER_NORMAL_UPDATE_OWNER,
        &ORIGINAL_WEATHER_EDITOR_REFRESH_OWNER,
        &ORIGINAL_WEATHER,
        &ORIGINAL_WEATHER_ACTIVE_SETTER,
        &ORIGINAL_WEATHER_ALPHA_SETTER,
        &ORIGINAL_WEATHER_PLAYBACK_SETTER,
        &ORIGINAL_SEED,
        &ORIGINAL_MAP_SCENE_RENDER_ROOT,
        &ORIGINAL_POST_PROCESS_RENDER_ROOT,
        &ORIGINAL_PRESENT_RENDER_ROOT,
        &ORIGINAL_SNOW_MUD_PRODUCER,
        &ORIGINAL_SNOW_MUD_UPLOAD,
        &ORIGINAL_CAMERA_WHEEL_UPDATE,
        &ORIGINAL_CAMERA_VERTICAL_EVENT_PRODUCER,
        &ORIGINAL_CAMERA_MAP_PICK,
        &o05_building_capture::ORIGINAL_UPDATE_PROVINCES,
        &o05_building_capture::ORIGINAL_BUILDING_CONSTRUCTOR,
        &o05_building_capture::ORIGINAL_BUILDING_VECTOR_UPDATE,
        &o05_building_capture::ORIGINAL_ENTITY_TRANSFORM_SETTER,
    ];
    let targets = targets(a);
    std::array::from_fn(|index| (targets[index], detours[index], originals[index]))
}

pub(super) fn engine_originals_are_clear(a: InstalledAddresses) -> bool {
    engine_hook_specs(a)
        .iter()
        .all(|(_, _, original)| original.load(Ordering::Acquire) == 0)
}

pub(super) fn stage_hooks_after_game_device(
    device: usize,
    context: usize,
    creation_qpc: u64,
) -> Result<(), String> {
    let mut stage = HOOK_STAGE
        .get_or_init(|| Mutex::new(HookStage::Bootstrap))
        .lock()
        .map_err(|_| "hook stage state poisoned")?;
    if *stage == HookStage::Ready {
        return Err("a second successful game-device creation was rejected".into());
    }
    stage.begin_staging().map_err(String::from)?;

    let result = (|| {
        if !INSTALLED.load(Ordering::Acquire) {
            return Err("bootstrap hooks are not installed at game-device staging".into());
        }
        let base = MODULE_BASE.load(Ordering::Acquire);
        if base == 0 {
            return Err("module base is absent at game-device staging".into());
        }
        let addresses = addresses(base);
        if !engine_originals_are_clear(addresses) {
            return Err("an engine hook was patched before successful game-device creation".into());
        }
        let mut staged_targets = STAGED_HOOK_TARGETS
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .map_err(|_| "staged hook target state poisoned")?;
        if !staged_targets.is_empty() {
            return Err("staged hook target list was non-empty before game-device staging".into());
        }

        let selection = staged_hook_selection(active_hook_install_profile()?)?;
        let mut all_specs = actual_hardware_d3d_hook_specs(device, context)?;
        all_specs.extend(engine_hook_specs(addresses));
        let all_labels = staged_hook_identity_labels();
        let all_groups = staged_hook_group_bits();
        let selected: Vec<_> = all_specs
            .into_iter()
            .zip(all_labels)
            .zip(all_groups)
            .enumerate()
            .filter_map(|(ordinal, item)| {
                staged_hook_selected(ordinal, item.1, selection).then_some(item)
            })
            .collect();
        let specs: Vec<_> = selected.iter().map(|((spec, _), _)| *spec).collect();
        let labels: Vec<_> = selected.iter().map(|((_, label), _)| *label).collect();
        let selection_wire_mask = selection
            .to_wire_mask()
            .ok_or("selected staged-hook inventory has no canonical wire mask")?;
        if staged_hook_count_for_group_mask(selection_wire_mask) != Some(specs.len() as u32) {
            return Err("selected staged-hook inventory differs from its group mask".into());
        }
        let targets = create_and_enable_hooks(&specs, "post-device staged")?;

        let epoch_result: Result<Option<u64>, String> = EARLY_CAPTURE_EPOCH
            .get()
            .ok_or("early capture epoch is absent at game-device staging")?
            .lock()
            .map_err(|_| "early capture epoch state poisoned".to_owned())
            .and_then(|mut epoch| {
                if creation_qpc == 0
                    || epoch.observer_installed_qpc == 0
                    || creation_qpc < epoch.observer_installed_qpc
                {
                    return Err("game-device creation QPC predates bootstrap readiness".into());
                }
                let generation = 1;
                let receipt = staged_hook_set_receipt_sha256(
                    epoch
                        .process
                        .as_ref()
                        .ok_or("early capture process epoch is absent during staging")?
                        .process_id,
                    epoch
                        .process
                        .as_ref()
                        .ok_or("early capture process epoch is absent during staging")?
                        .process_start_nonce,
                    epoch.hook_epoch_nonce,
                    &epoch.hook_set_receipt_sha256,
                    device,
                    context,
                    generation,
                    selection.is_diagnostic(),
                    selection_wire_mask,
                    &labels,
                    &targets,
                )?;
                let diagnostic_staging_complete_sequence = if selection
                    == EarlyStagedHookSelection::Empty
                    || (selection.is_diagnostic() && selection.com_selector() != 0)
                {
                    Some(
                        next_catalog_observation_sequence()
                            .ok_or("diagnostic COM-selector staging sequence exhausted")?,
                    )
                } else {
                    None
                };
                epoch.game_device_count = epoch.game_device_count.saturating_add(1);
                if epoch.d3d_device_created_qpc == 0 {
                    epoch.d3d_device_created_qpc = creation_qpc;
                }
                epoch.staged_hook_generation = generation;
                epoch.staged_hook_count = targets.len() as u32;
                epoch.staged_hook_set_sha256 = receipt;
                epoch.diagnostic_stage_probe = selection.is_diagnostic();
                epoch.staged_hook_group_mask = selection_wire_mask;
                epoch.game_device_ptr = device;
                epoch.immediate_context_ptr = context;
                Ok(diagnostic_staging_complete_sequence)
            });
        let diagnostic_staging_complete_sequence = match epoch_result {
            Ok(sequence) => sequence,
            Err(error) => {
                let rollback = rollback_created_hooks(&specs, specs.len())
                    .err()
                    .map(|rollback| format!("; {rollback}"))
                    .unwrap_or_default();
                return Err(format!("{error}{rollback}"));
            }
        };
        *staged_targets = targets;
        Ok(diagnostic_staging_complete_sequence)
    })();

    match result {
        Ok(diagnostic_staging_complete_sequence) => {
            stage.finish_staging(true);
            drop(stage);
            let pending = PENDING_STAGE_FIRST_CREATION.swap(0, Ordering::AcqRel);
            if let Some(sequence) =
                staged_first_sequence(pending, diagnostic_staging_complete_sequence)
            {
                mark_first_creation_sequence(sequence);
            }
            Ok(())
        }
        Err(error) => {
            stage.finish_staging(false);
            PENDING_STAGE_FIRST_CREATION.store(0, Ordering::Release);
            Err(error)
        }
    }
}

/// Install the broad staged render-observation graph for an explicitly
/// unshipped evidence run.
pub fn install_unshipped_evidence_render_hooks() -> Result<InstalledAddresses, String> {
    let authority = match std::env::var(UNSHIPPED_EVIDENCE_HOOKS_ENV) {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err("unshipped evidence render-hook authority is not Unicode".into())
        }
    };
    if !unshipped_evidence_authority_is_explicit(authority.as_deref()) {
        return Err(format!(
            "unshipped evidence render hooks require {UNSHIPPED_EVIDENCE_HOOKS_ENV}={UNSHIPPED_EVIDENCE_HOOKS_AUTHORITY}"
        ));
    }
    Ok(install_with_profile(HookInstallProfile::UnshippedEvidenceStaged)?.addresses)
}
