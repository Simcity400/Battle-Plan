//! Complete authenticated materialization census for retained container sprites.
//! Display paths locate this candidate's elements; semantic scopes and local
//! authored paths identify the objects retained across candidate generations.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::assets::AssetResolver;
use crate::container_animation_records::{
    AdmittedContainerAnimations, NativeAnimationParseContext,
};
use crate::container_sprite_lifetime::{
    ContainerSpriteIdentity, ContainerSpriteLifetimes, ContainerSpriteWidgetFactory,
    StagedContainerSpriteLifetimes,
};
use crate::gfx::{SpriteDefinition, SpriteRegistry};
use crate::gui::GuiElement;
use portable_content_manifest::UsedContentKey;
use snapshot::sha256::Sha256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ContainerSpriteConstructor {
    Generic,
    MaskedShield,
    /// An unsupported factory is not evidence of an absent generic object.
    Unproven(String),
}

#[derive(Clone, Debug)]
pub(crate) struct MaterializedContainerSprite {
    pub element_name: String,
    pub display_child_path: Vec<u32>,
    pub constructor: ContainerSpriteConstructor,
    pub widget_factory: ContainerSpriteWidgetFactory,
    pub identity: Option<ContainerSpriteIdentity>,
    pub animations: Result<AdmittedContainerAnimations, String>,
    /// Exact consumed source lease, shared by elements using the same winner.
    pub definition_payload: Option<Arc<[u8]>>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ContainerSpriteMaterialization {
    pub elements: Vec<MaterializedContainerSprite>,
    pub refusals: Vec<String>,
}

impl ContainerSpriteMaterialization {
    /// Never supply a filtered census to the lifetime owner. Failure leaves the
    /// presented table unchanged; the caller retains every per-element refusal.
    pub(crate) fn stage_lifetimes(
        &self,
        presented: &ContainerSpriteLifetimes,
    ) -> Result<StagedContainerSpriteLifetimes, String> {
        if !self.refusals.is_empty() {
            return Err(self.refusals.join("; "));
        }
        let mut census = Vec::new();
        for element in &self.elements {
            if element.constructor == ContainerSpriteConstructor::MaskedShield {
                // Proven CShieldObject construction, outside the CSprite census.
                continue;
            }
            let animations = element.animations.as_ref().map_err(|reason| {
                format!("container sprite {:?}: {reason}", element.element_name)
            })?;
            let identity = element.identity.as_ref().ok_or_else(|| {
                format!(
                    "container sprite {:?}: constructor identity unavailable",
                    element.element_name
                )
            })?;
            if element.constructor != ContainerSpriteConstructor::Generic {
                return Err(format!(
                    "container sprite {:?}: factory unavailable",
                    element.element_name
                ));
            }
            census.push((
                identity.clone(),
                animations.records.len(),
                element.widget_factory.clone(),
            ));
        }
        presented
            .stage_materialization(census)
            .map_err(|reason| format!("container sprite census: {reason:?}"))
    }
}

/// The caller supplies the complete authenticated materialized tree, current
/// compositor registry, and semantic scopes from the actual template owners.
/// Unknown source parse controls stay None; they are not replaced by host modes.
pub(crate) fn collect_container_sprite_materialization(
    root: &GuiElement,
    root_owner_key: &[String],
    owner_scopes: &BTreeMap<String, Vec<String>>,
    sprite_name_overrides: &BTreeMap<String, String>,
    registry: &SpriteRegistry,
    resolver: &AssetResolver,
    parse_context: Option<NativeAnimationParseContext>,
) -> ContainerSpriteMaterialization {
    let mut result = ContainerSpriteMaterialization::default();
    if root_owner_key.is_empty() || root_owner_key.iter().any(String::is_empty) {
        result
            .refusals
            .push("container sprite root semantic owner unavailable".into());
    }
    let mut sources = BTreeMap::new();
    collect_node(
        root,
        &[],
        &[],
        root_owner_key,
        owner_scopes,
        sprite_name_overrides,
        registry,
        resolver,
        parse_context,
        &mut sources,
        &mut result,
    );
    result
}

type SourceLease = Result<(UsedContentKey, Arc<[u8]>, crate::paradox_text::Document), String>;

fn widget_factory(element: &GuiElement) -> ContainerSpriteWidgetFactory {
    let admit = || -> Result<(), String> {
        if !element.element_type.eq_ignore_ascii_case("iconType") {
            return Err(format!("owning widget factory {:?}", element.element_type));
        }
        let fields = element
            .raw
            .as_object()
            .ok_or("authored icon block unavailable")?;
        // CIconType142358490 initializes +13c=1. The position reader142358910
        // case4c keeps it 1; position3D caseaf selects the other factory branch.
        for field in fields {
            if let Some(key) = field.key.as_deref() {
                if key.eq_ignore_ascii_case("position3d") {
                    return Err("ordinary CIcon 2D factory: position3D branch unavailable".into());
                }
                if key.eq_ignore_ascii_case("position") {
                    if field.key_is_quoted {
                        return Err("ordinary CIcon position token unavailable".into());
                    }
                    parse_icon_fixed_pair(&field.value)?;
                }
            }
        }
        Ok(())
    };
    match admit() {
        Ok(()) => ContainerSpriteWidgetFactory::OrdinaryIcon2d,
        Err(reason) => ContainerSpriteWidgetFactory::Unproven(reason),
    }
}

/// The fixed pair consumed by CIconType, before its i32 -> f32 storage and
/// CIcon's later CVTTSS2SI. This identity names the original authenticated GUI
/// definition, independently of the materialized display path or GFX source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AdmittedStaticIconPosition {
    pub gui_source: UsedContentKey,
    pub authored_child_path: Vec<u32>,
    pub fixed_position: [i32; 2],
}

impl AdmittedStaticIconPosition {
    /// Consume the original authored element, before viewer placement setters.
    /// This admits the position operand only. The owning materializer must
    /// separately establish upper_left, no centering, unit scale, zero rotation,
    /// the physical graphics extent and the actual parent before staging it.
    pub(crate) fn parse(
        element: &GuiElement,
        gui_source: &UsedContentKey,
        authored_child_path: &[u32],
    ) -> Result<Self, String> {
        let fail = |reason: &str| {
            format!(
                "static icon {:?} at {:?}: {reason}",
                element.name, authored_child_path,
            )
        };
        gui_source.validate().map_err(|reason| fail(&reason))?;
        if !gui_source
            .normalized_game_relative_path
            .to_ascii_lowercase()
            .ends_with(".gui")
        {
            return Err(fail("authenticated GUI source identity required"));
        }
        if !element.element_type.eq_ignore_ascii_case("iconType") {
            return Err(fail(
                "ordinary iconType position reader is not established for this widget family",
            ));
        }
        let fields = element
            .raw
            .as_object()
            .ok_or_else(|| fail("authored icon block unavailable"))?;
        let mut position = [0; 2]; // 142358490 constructor, absence only.
        for entry in fields {
            let Some(key) = entry.key.as_deref() else {
                continue;
            };
            if key.eq_ignore_ascii_case("position3d") {
                return Err(fail("3D icon position is outside the fixed integer branch"));
            }
            if !key.eq_ignore_ascii_case("position") {
                continue;
            }
            if entry.key_is_quoted {
                return Err(fail("quoted position key has no native token admission"));
            }
            position = parse_icon_fixed_pair(&entry.value).map_err(|reason| fail(&reason))?;
        }
        Ok(Self {
            gui_source: gui_source.clone(),
            authored_child_path: authored_child_path.to_vec(),
            fixed_position: position,
        })
    }

    /// Call only after the owner admits the static upper_left, uncentered,
    /// unit-scale, zero-rotation branch. No Rect is converted back into a source
    /// operand here; preserve the native float storage/conversion in the helper.
    pub(crate) fn stage_position_for_admitted_static_branch(
        &self,
        stored_physical_extent: &[u32; 2],
        native_integer_size: &[i32; 2],
    ) -> crate::container_sprite_geometry::NativeSpritePositionWrite {
        crate::container_sprite_geometry::NativeSpritePositionWrite::stage_static_icon(
            &self.fixed_position,
            stored_physical_extent,
            native_integer_size,
        )
    }
}

fn parse_icon_fixed_pair(value: &crate::paradox_text::Value) -> Result<[i32; 2], String> {
    // 140a3d660 requires open, x, '=', integer-token, y, '=', integer-token,
    // close in that order. Partial/recovered blocks do not prove its output.
    let fields = value
        .as_object()
        .ok_or("position requires the fixed x/y block")?;
    if fields.len() != 2 {
        return Err("position requires exactly the ordered x and y assignments".into());
    }
    let mut pair = [0; 2];
    for (axis, field) in fields.iter().enumerate() {
        if field.key_is_quoted
            || !field
                .key
                .as_deref()
                .is_some_and(|key| key.eq_ignore_ascii_case(["x", "y"][axis]))
        {
            return Err(format!(
                "position axis {axis} requires native {} token",
                ["x", "y"][axis]
            ));
        }
        pair[axis] = parse_icon_integer_token(&field.value)
            .map_err(|reason| format!("position {}: {reason}", ["x", "y"][axis]))?;
    }
    Ok(pair)
}

fn parse_icon_integer_token(value: &crate::paradox_text::Value) -> Result<i32, String> {
    use crate::paradox_text::{Scalar, Value};
    use map_viewer::authored_fixed::{assign_native_i32, NativeI32Assignment};
    let Value::Scalar(Scalar::Bare(text)) = value else {
        return Err("native integer token required; string/block tokens are unavailable".into());
    };
    if text.starts_with('@') {
        return Err("native macro-expanded integer token unavailable".into());
    }
    let mut assigned = 0;
    match assign_native_i32(text, &mut assigned) {
        NativeI32Assignment::EndOfInput => {
            return Err("EOF assignment leaves native temporary unavailable".into())
        }
        NativeI32Assignment::MatchingFailure => {
            return Err("native integer assignment failed".into())
        }
        NativeI32Assignment::Match => {}
    }
    // Admit the bounded bare signed-decimal token spelling. A successful %i
    // prefix scan alone does not prove native integer token type: percentages,
    // fractions, suffixes, hex spellings and embedded NUL stay unavailable.
    // Leading zeroes are retained for the shared scanner's native octal rule.
    let digits = text.strip_prefix('-').unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("native integer token type unavailable for this spelling".into());
    }
    Ok(assigned)
}

#[allow(clippy::too_many_arguments)]
fn collect_node(
    node: &GuiElement,
    display_path: &[u32],
    local_path: &[u32],
    inherited_owner: &[String],
    owner_scopes: &BTreeMap<String, Vec<String>>,
    overrides: &BTreeMap<String, String>,
    registry: &SpriteRegistry,
    resolver: &AssetResolver,
    context: Option<NativeAnimationParseContext>,
    sources: &mut BTreeMap<String, SourceLease>,
    result: &mut ContainerSpriteMaterialization,
) {
    let mut owner = inherited_owner.to_vec();
    let scope = owner_scopes.get(&node.name);
    // Each map entry is the caller's explicit declaration of a materialized
    // semantic owner root. An empty scope names an unavailable owner identity.
    let dynamic_root = scope.is_some();
    if let Some(scope) = scope {
        if scope.is_empty() || scope.iter().any(String::is_empty) {
            result.refusals.push(format!(
                "container sprite {:?}: semantic owner scope unavailable",
                node.name
            ));
        } else if !(display_path.is_empty() && scope == inherited_owner) {
            owner.extend_from_slice(scope);
        }
    }
    let local_path = if dynamic_root { &[][..] } else { local_path };
    let sprite_name = overrides
        .get(&node.name)
        .map(String::as_str)
        .or_else(|| crate::compose::primary_sprite_ref(node));
    if let Some(sprite_name) = sprite_name {
        let mut row = MaterializedContainerSprite {
            element_name: node.name.clone(),
            display_child_path: display_path.to_vec(),
            constructor: ContainerSpriteConstructor::Unproven("definition unavailable".into()),
            widget_factory: widget_factory(node),
            identity: None,
            animations: Err(format!("definition {sprite_name:?} unavailable")),
            definition_payload: None,
        };
        if let Some(sprite) = registry.get(sprite_name) {
            let lease = sources
                .entry(sprite.source_file.clone())
                .or_insert_with(|| read_source(resolver, &sprite.source_file));
            row.animations = (|| {
                let (source, payload, document) = lease.as_ref().map_err(Clone::clone)?;
                verify_registry_definition(sprite, document)?;
                row.identity = Some(ContainerSpriteIdentity {
                    owner_key: owner.clone(),
                    authored_child_path: local_path.to_vec(),
                    definition_source: source.clone(),
                    sprite_name: sprite.name.clone(),
                });
                row.definition_payload = Some(payload.clone());
                row.constructor = constructor(sprite);
                match &row.constructor {
                    ContainerSpriteConstructor::Generic => {
                        AdmittedContainerAnimations::parse(&sprite.raw, source.clone(), context)
                            .map_err(|reason| reason.to_string())
                    }
                    ContainerSpriteConstructor::MaskedShield => Err("maskedShieldType constructs CShieldObject; generic CSprite animations do not apply".into()),
                    ContainerSpriteConstructor::Unproven(kind) => Err(format!("native constructor for sprite family {kind:?} unavailable")),
                }
            })();
        }
        result.elements.push(row);
    }
    for (ordinal, child) in node.children.iter().enumerate() {
        let Ok(ordinal) = u32::try_from(ordinal) else {
            result.refusals.push(format!(
                "container sprite {:?}: child ordinal exceeds u32",
                node.name
            ));
            break;
        };
        let mut display = display_path.to_vec();
        display.push(ordinal);
        let mut local = local_path.to_vec();
        local.push(ordinal);
        collect_node(
            child,
            &display,
            &local,
            &owner,
            owner_scopes,
            overrides,
            registry,
            resolver,
            context,
            sources,
            result,
        );
    }
}

fn constructor(sprite: &SpriteDefinition) -> ContainerSpriteConstructor {
    // a729 1422264c0 asks the game factory first, then constructs the generic
    // type. 1422228f0 invokes the selected type's +b8 object factory. The
    // masked route is 140b31fb0 -> 14162f1f0 -> 14162fc50 -> 141630340.
    // No suffix matching: bordered, custom, and masked definitions do not
    // establish the ordinary CSprite constructor's state.
    if sprite.kind.eq_ignore_ascii_case("spriteType") {
        ContainerSpriteConstructor::Generic
    } else if sprite.kind.eq_ignore_ascii_case("maskedShieldType") {
        ContainerSpriteConstructor::MaskedShield
    } else {
        ContainerSpriteConstructor::Unproven(sprite.kind.clone())
    }
}

fn read_source(resolver: &AssetResolver, path: &str) -> SourceLease {
    if !resolver.has_portable_content_admission() {
        return Err("recording-authenticated sprite definition required".into());
    }
    let source = resolver
        .admitted_content_key(path)
        .ok_or_else(|| format!("authenticated definition source {path:?} unavailable"))?
        .clone();
    source.validate()?;
    let file = resolver
        .resolve_file(path)
        .map_err(|reason| reason.to_string())?;
    let payload = resolver
        .read_file(&file)
        .map_err(|reason| reason.to_string())?;
    if !file
        .rel_path
        .eq_ignore_ascii_case(&source.normalized_game_relative_path)
        || payload.len() as u64 != source.payload_bytes
        || Sha256::hex(&payload) != source.payload_sha256
    {
        return Err(format!("authenticated definition source {path:?} changed"));
    }
    if crate::paradox_text::destructive_event_count(&crate::paradox_text::diagnose_text_decode(
        &payload,
    )) != 0
    {
        return Err(format!(
            "authenticated definition source {path:?} has decode loss"
        ));
    }
    let document = crate::paradox_text::parse_bytes(&payload);
    if !document.recoveries.is_empty() {
        return Err(format!(
            "authenticated definition source {path:?} has parse recoveries"
        ));
    }
    Ok((source, payload.into(), document))
}

/// Verify the existing registry winner against its consumed source. This does
/// not construct a second registry or use a different winner for rendering.
fn verify_registry_definition(
    sprite: &SpriteDefinition,
    document: &crate::paradox_text::Document,
) -> Result<(), String> {
    let mut winner = None;
    for container_name in ["spriteTypes", "bitmapfonts"] {
        for container in document.root.values_ci(container_name) {
            for entry in container.fields() {
                let Some(kind) = entry.key.as_deref() else {
                    continue;
                };
                if container_name == "bitmapfonts"
                    && !kind.to_ascii_lowercase().contains("spritetype")
                    && !kind.eq_ignore_ascii_case("progressbartype")
                {
                    continue;
                }
                if entry
                    .value
                    .first_scalar_ci("name")
                    .is_some_and(|name| name.eq_ignore_ascii_case(&sprite.name))
                {
                    winner = crate::gfx::parse_sprite(kind, &sprite.source_file, &entry.value);
                }
            }
        }
    }
    if winner.as_ref() != Some(sprite) {
        return Err(format!(
            "registry definition {:?} differs from authenticated source winner",
            sprite.name
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container_sprite_inputs::NativeButtonEffect;
    use crate::container_sprite_interaction::{IconObserverEvent, IconOperation};
    use crate::container_sprite_lifetime::NativeGuiClock;
    use crate::container_sprite_pass::{ContainerSpritePass, ContainerSpritePolicy};
    use crate::runtime_graphics::{ButtonVisualState, ClipRect};
    use portable_content_manifest::{paradox_text::parse_str, ContentLayerKind};

    fn icon_pass(
        census: ContainerSpriteMaterialization,
        presented: &ContainerSpriteLifetimes,
        seconds: i64,
    ) -> ContainerSpritePass {
        ContainerSpritePass::begin(
            census,
            presented,
            Some(NativeGuiClock::from_application_elapsed_nanoseconds(
                seconds * 1_000_000_000,
            )),
            ContainerSpritePolicy {
                texture_normalization: None,
                native_mxcsr: None,
            },
        )
        .unwrap()
    }

    fn clip() -> Option<ClipRect> {
        Some(ClipRect {
            left: 0,
            top: 0,
            right: 16,
            bottom: 16,
        })
    }

    fn ordinary_icon() -> ContainerSpriteMaterialization {
        collect("guiTypes = { iconType = { name = icon spriteType = GFX_test position = { x = 2 y = 3 } } }",
            "spriteTypes = { spriteType = { name = GFX_test } }", &BTreeMap::new())
    }

    #[test]
    fn materialized_icon_operations_preserve_temporary_disable_latch_and_rollback() {
        let empty = ContainerSpriteLifetimes::default();
        let mut first = icon_pass(ordinary_icon(), &empty, 3);
        let fresh = first.reached(&[], "GFX_test", None, clip()).unwrap();
        assert_eq!(fresh.effect, NativeButtonEffect::Unassigned);
        assert_eq!(fresh.draw.draw_clock.to_bits(), 0);
        first
            .icon_operation(
                &[],
                "GFX_test",
                IconOperation::Enable {
                    disable_locked: false,
                },
                |_, _, _| Ok(()),
            )
            .unwrap();
        let enabled = first.reached(&[], "GFX_test", None, clip()).unwrap();
        assert_eq!(enabled.effect, NativeButtonEffect::Up);
        assert_eq!(enabled.draw.draw_clock, 3.0);
        let presented = first.lifetimes.clone();
        let mut next = icon_pass(ordinary_icon(), &presented, 7);
        next.icon_operation(
            &[],
            "GFX_test",
            IconOperation::ForceDisabledCallback(true),
            |_, _, _| Ok(()),
        )
        .unwrap();
        next.icon_operation(
            &[],
            "GFX_test",
            IconOperation::FinishInputUpdate,
            |_, _, _| panic!("forced state3 has no observer notification"),
        )
        .unwrap();
        let restored = next.reached(&[], "GFX_test", None, clip()).unwrap();
        assert_eq!(restored.effect, NativeButtonEffect::Up);
        assert_eq!(restored.draw.draw_clock, 7.0);
        let staged = next.lifetimes.clone();
        next.begin_raster_replay();
        assert!(next.reached(&[], "GFX_test", None, clip()).is_err());
        next.icon_operation(
            &[],
            "GFX_test",
            IconOperation::ForceDisabledCallback(true),
            |_, _, _| panic!("replay"),
        )
        .unwrap();
        next.icon_operation(
            &[],
            "GFX_test",
            IconOperation::FinishInputUpdate,
            |_, _, _| panic!("replay"),
        )
        .unwrap();
        assert_eq!(
            next.reached(&[], "GFX_test", None, clip())
                .unwrap()
                .draw
                .draw_clock,
            7.0
        );
        assert!(next.finish_raster_replay().is_ok());
        assert_eq!(next.lifetimes, staged);
        // A rejected presentation never installs either widget flags or clocks.
        let mut rollback = icon_pass(ordinary_icon(), &presented, 9);
        assert_eq!(
            rollback
                .reached(&[], "GFX_test", None, clip())
                .unwrap()
                .draw
                .draw_clock,
            3.0
        );
        assert_eq!(presented, first.lifetimes);
        assert_eq!(empty, ContainerSpriteLifetimes::default());
    }

    #[test]
    fn actual_input_callback_writes_are_transactional_and_replay_keeps_each_draw_effect() {
        let mut pass = icon_pass(ordinary_icon(), &ContainerSpriteLifetimes::default(), 4);
        pass.reached(&[], "GFX_test", None, clip()).unwrap();
        pass.icon_operation(
            &[],
            "GFX_test",
            IconOperation::WriteInputState(ButtonVisualState::Over),
            |_, _, _| Ok(()),
        )
        .unwrap();
        let before = pass.lifetimes.clone();
        assert!(pass
            .icon_operation(
                &[],
                "GFX_test",
                IconOperation::FinishInputUpdate,
                |event, icon, drawable| {
                    assert_eq!(event, IconObserverEvent::Over);
                    icon.disable(drawable);
                    Err("viewer observer unavailable after staged writes".into())
                }
            )
            .is_err());
        assert_eq!(pass.lifetimes, before);
        pass.icon_operation(
            &[],
            "GFX_test",
            IconOperation::FinishInputUpdate,
            |event, icon, drawable| {
                assert_eq!(event, IconObserverEvent::Over);
                icon.set_state(drawable, ButtonVisualState::Down);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            pass.reached(&[], "GFX_test", None, clip()).unwrap().effect,
            NativeButtonEffect::Down
        );
        let staged = pass.lifetimes.clone();
        pass.begin_raster_replay();
        // Final retained effect is Down; the first draw receipt still owns 0.
        assert_eq!(
            pass.reached(&[], "GFX_test", None, clip()).unwrap().effect,
            NativeButtonEffect::Unassigned
        );
        assert!(pass
            .icon_operation(&[], "GFX_test", IconOperation::Disable, |_, _, _| Ok(()))
            .is_err());
        pass.icon_operation(
            &[],
            "GFX_test",
            IconOperation::WriteInputState(ButtonVisualState::Over),
            |_, _, _| panic!("replay"),
        )
        .unwrap();
        pass.icon_operation(
            &[],
            "GFX_test",
            IconOperation::FinishInputUpdate,
            |_, _, _| panic!("replay"),
        )
        .unwrap();
        assert_eq!(
            pass.reached(&[], "GFX_test", None, clip()).unwrap().effect,
            NativeButtonEffect::Down
        );
        assert!(pass.finish_raster_replay().is_ok());
        assert_eq!(pass.lifetimes, staged);
    }

    #[test]
    fn generic_sprite_does_not_establish_other_widget_or_3d_ownership() {
        for (kind, fields) in [
            ("background", ""),
            ("buttonType", ""),
            ("customIconType", ""),
            ("iconType", "position3D = { x = 0 y = 0 z = 0 }"),
            ("iconType", "position = { x = 1 }"),
        ] {
            let census = collect(
                &format!(
                    "guiTypes = {{ {kind} = {{ name = icon spriteType = GFX_test {fields} }} }}"
                ),
                "spriteTypes = { spriteType = { name = GFX_test } }",
                &BTreeMap::new(),
            );
            assert!(matches!(
                census.elements[0].widget_factory,
                ContainerSpriteWidgetFactory::Unproven(_)
            ));
            let mut pass = icon_pass(census, &ContainerSpriteLifetimes::default(), 2);
            let before = pass.lifetimes.clone();
            assert!(pass.reached(&[], "GFX_test", None, clip()).is_err());
            assert!(pass
                .reached(&[], "GFX_test", Some(NativeButtonEffect::Disable), clip())
                .is_err());
            assert!(pass
                .icon_operation(&[], "GFX_test", IconOperation::Disable, |_, _, _| Ok(()))
                .is_err());
            assert_eq!(pass.lifetimes, before);
        }
    }

    #[test]
    fn retained_icon_follows_semantic_owner_across_display_order_and_recreation() {
        let gfx = "spriteTypes = { spriteType = { name = GFX_test } }";
        let scopes = BTreeMap::from([
            ("a".into(), vec!["participant".into(), "a".into()]),
            ("b".into(), vec!["participant".into(), "b".into()]),
        ]);
        let tree = |children: &str| {
            format!("guiTypes = {{ containerWindowType = {{ name = root {children} }} }}")
        };
        let a = "iconType = { name = a spriteType = GFX_test }";
        let b = "iconType = { name = b spriteType = GFX_test }";
        let first = collect(&tree(&format!("{a} {b}")), gfx, &scopes);
        let mut pass = icon_pass(first, &ContainerSpriteLifetimes::default(), 3);
        pass.icon_operation(&[0], "GFX_test", IconOperation::Disable, |_, _, _| Ok(()))
            .unwrap();
        pass.reached(&[0], "GFX_test", None, clip()).unwrap();
        let presented = pass.lifetimes.clone();
        let reordered = collect(&tree(&format!("{b} {a}")), gfx, &scopes);
        let mut next = icon_pass(reordered, &presented, 8);
        assert_eq!(
            next.reached(&[1], "GFX_test", None, clip())
                .unwrap()
                .draw
                .draw_clock,
            3.0
        );
        assert_eq!(
            next.reached(&[0], "GFX_test", None, clip()).unwrap().effect,
            NativeButtonEffect::Unassigned
        );
        let mut changed_scopes = scopes.clone();
        changed_scopes.insert("a".into(), vec!["participant".into(), "new".into()]);
        let recreated = collect(&tree(&format!("{b} {a}")), gfx, &changed_scopes);
        let mut fresh = icon_pass(recreated, &presented, 9);
        assert_eq!(
            fresh
                .reached(&[1], "GFX_test", None, clip())
                .unwrap()
                .effect,
            NativeButtonEffect::Unassigned
        );
        let changed_kind = collect(
            &tree(&format!(
                "buttonType = {{ name = a spriteType = GFX_test }} {b}"
            )),
            gfx,
            &scopes,
        );
        assert!(changed_kind
            .stage_lifetimes(&presented)
            .unwrap_err()
            .contains("WidgetFactoryChangedWithoutConstruction"));
        assert_eq!(presented, pass.lifetimes);
    }

    fn fixture(gfx: &str) -> (SpriteRegistry, BTreeMap<String, SourceLease>) {
        let path = "interface/test.gfx";
        let document = parse_str(gfx);
        assert!(document.recoveries.is_empty());
        let mut registry = SpriteRegistry::new();
        registry.add_document(path, &document);
        let hash = Sha256::hex(gfx.as_bytes());
        let content = UsedContentKey {
            effective_vfs_manifest_sha256: "a".repeat(64),
            stored_game_relative_path: path.into(),
            normalized_game_relative_path: path.into(),
            winning_layer_ordinal: 0,
            layer_kind: ContentLayerKind::Base,
            layer_id: "base".into(),
            source_sha256: hash.clone(),
            payload_sha256: hash,
            payload_bytes: gfx.len() as u64,
        };
        (
            registry,
            BTreeMap::from([(
                path.into(),
                Ok((content, Arc::from(gfx.as_bytes()), document)),
            )]),
        )
    }

    fn collect(
        gui: &str,
        gfx: &str,
        scopes: &BTreeMap<String, Vec<String>>,
    ) -> ContainerSpriteMaterialization {
        let root = crate::gui::parse_gui_document(&parse_str(gui)).remove(0);
        let (registry, mut sources) = fixture(gfx);
        let mut result = ContainerSpriteMaterialization::default();
        collect_node(
            &root,
            &[],
            &[],
            &["conference".into(), "42".into()],
            scopes,
            &BTreeMap::new(),
            &registry,
            &AssetResolver::new(vec![]),
            None,
            &mut sources,
            &mut result,
        );
        result
    }

    #[test]
    fn semantic_scope_survives_display_reordering_and_keeps_conference() {
        let gfx = "spriteTypes = { spriteType = { name = GFX_test } }";
        let scopes = BTreeMap::from([
            ("root".into(), vec!["conference".into(), "42".into()]),
            ("row_a".into(), vec!["participant".into(), "a".into()]),
            ("row_b".into(), vec!["participant".into(), "b".into()]),
        ]);
        let before = collect("guiTypes = { containerWindowType = { name = root containerWindowType = { name = row_a iconType = { name = icon_a spriteType = GFX_test } } containerWindowType = { name = row_b iconType = { name = icon_b spriteType = GFX_test } } } }", gfx, &scopes);
        let after = collect("guiTypes = { containerWindowType = { name = root containerWindowType = { name = row_b iconType = { name = icon_b spriteType = GFX_test } } containerWindowType = { name = row_a iconType = { name = icon_a spriteType = GFX_test } } } }", gfx, &scopes);
        let a = before.elements[0].identity.as_ref().unwrap();
        assert_eq!(a.owner_key, ["conference", "42", "participant", "a"]);
        assert_eq!(a.authored_child_path, [0]);
        assert_eq!(after.elements[1].identity.as_ref(), Some(a));
        assert_ne!(
            before.elements[0].display_child_path,
            after.elements[1].display_child_path
        );
        let original = before
            .stage_lifetimes(&ContainerSpriteLifetimes::default())
            .unwrap();
        assert_eq!(original.created, before.elements.iter()
            .map(|row| row.identity.clone().unwrap()).collect::<Vec<_>>());
        let presented = original.next
            .stage_icon_operation(
                a,
                crate::container_sprite_interaction::IconOperation::Disable,
                |_, _, _| Ok(()),
            )
            .unwrap()
            .stage_reached_draw(a, 7.0, NativeButtonEffect::Disable, false, &[])
            .unwrap()
            .next;
        let reordered = after.stage_lifetimes(&presented).unwrap();
        assert!(reordered.created.is_empty());
        assert_eq!(reordered.next, presented);
    }

    #[test]
    fn unknown_parse_control_retains_element_identity_and_blocks_census_replacement() {
        let census = collect("guiTypes = { iconType = { name = animated spriteType = GFX_test hide = yes } }",
            "spriteTypes = { spriteType = { name = GFX_test animation = { animationtype = pulsing animationtime = 3 } } }", &BTreeMap::new());
        assert_eq!(census.elements.len(), 1);
        let row = &census.elements[0];
        assert!(row.identity.is_some());
        assert!(row.definition_payload.is_some());
        assert!(row
            .animations
            .as_ref()
            .unwrap_err()
            .contains("native floating-point control unavailable"));
        assert!(census
            .stage_lifetimes(&ContainerSpriteLifetimes::default())
            .is_err());
    }

    #[test]
    fn masked_and_unproven_families_never_construct_generic_sprites() {
        let masked = collect(
            "guiTypes = { iconType = { name = flag spriteType = GFX_test } }",
            "spriteTypes = { maskedShieldType = { name = GFX_test } }",
            &BTreeMap::new(),
        );
        assert_eq!(
            masked.elements[0].constructor,
            ContainerSpriteConstructor::MaskedShield
        );
        assert!(masked.elements[0].animations.is_err());
        let staged = masked.stage_lifetimes(&ContainerSpriteLifetimes::default()).unwrap();
        assert_eq!(staged.next, ContainerSpriteLifetimes::default());
        assert!(staged.created.is_empty());
        let custom = collect(
            "guiTypes = { iconType = { name = custom spriteType = GFX_test } }",
            "spriteTypes = { customSpriteType = { name = GFX_test } }",
            &BTreeMap::new(),
        );
        assert!(custom
            .stage_lifetimes(&ContainerSpriteLifetimes::default())
            .is_err());
    }

    #[test]
    fn missing_definition_and_missing_dynamic_owner_are_not_measured_empty() {
        let missing = collect(
            "guiTypes = { iconType = { name = missing spriteType = GFX_missing } }",
            "spriteTypes = { spriteType = { name = GFX_test } }",
            &BTreeMap::from([("missing".into(), Vec::new())]),
        );
        assert_eq!(missing.elements.len(), 1);
        assert!(missing.elements[0].identity.is_none());
        assert_eq!(missing.refusals.len(), 1);
        assert!(missing
            .stage_lifetimes(&ContainerSpriteLifetimes::default())
            .is_err());
    }

    #[test]
    fn registry_winner_must_match_actual_consumed_source() {
        let (registry, sources) = fixture("spriteTypes = { spriteType = { name = GFX_test noOfFrames = 2 } spriteType = { name = GFX_test noOfFrames = 4 } }");
        let (_, _, document) = sources.values().next().unwrap().as_ref().unwrap();
        assert!(verify_registry_definition(registry.get("GFX_test").unwrap(), document).is_ok());
        assert!(verify_registry_definition(&registry.sprites()[0], document).is_err());
        let mut changed = registry.get("GFX_test").unwrap().clone();
        changed.no_of_frames = 9;
        assert!(verify_registry_definition(&changed, document).is_err());
    }

    fn icon_position(kind: &str, fields: &str) -> Result<AdmittedStaticIconPosition, String> {
        let raw = parse_str(&format!(
            "guiTypes = {{ {kind} = {{ name = test_icon {fields} }} }}"
        ));
        assert!(raw.recoveries.is_empty());
        let element = crate::gui::parse_gui_document(&raw).remove(0);
        let (_, sources) = fixture("spriteTypes = { spriteType = { name = GFX_test } }");
        let mut source = sources.values().next().unwrap().as_ref().unwrap().0.clone();
        source.normalized_game_relative_path = "interface/test.gui".into();
        source.stored_game_relative_path = "interface/test.gui".into();
        AdmittedStaticIconPosition::parse(&element, &source, &[3, 1])
    }

    #[test]
    fn icon_position_uses_native_integer_assignment_and_preserves_source_path() {
        let position = icon_position(
            "iconType",
            "position = { x = 010 y = -12 } position = { x = 16777217 y = 2147483647 }",
        )
        .unwrap();
        assert_eq!(position.fixed_position, [16_777_217, i32::MAX]);
        assert_eq!(position.authored_child_path, [3, 1]);
        assert_eq!(
            position.gui_source.normalized_game_relative_path,
            "interface/test.gui"
        );
        let write = position.stage_position_for_admitted_static_branch(&[100, 100], &[8, 4]);
        assert_eq!(
            write.requested_position_bits.map(f32::from_bits),
            [16_777_216.0, i32::MIN as f32]
        );
        assert_eq!(
            icon_position("iconType", "position = { x = 010 y = 4294967295 }")
                .unwrap()
                .fixed_position,
            [8, -1]
        );
    }

    #[test]
    fn icon_position_missing_is_constructor_zero_but_partial_or_reordered_is_unavailable() {
        assert_eq!(
            icon_position("iconType", "").unwrap().fixed_position,
            [0, 0]
        );
        for fields in [
            "position = {}",
            "position = { x = 7 }",
            "position = { y = 8 }",
            "position = { y = 8 x = 7 }",
            "position = { x = 7 x = 8 }",
            "position = { x = 7 y = 8 z = 9 }",
            "position = { 7 8 }",
            "position = { \"x\" = 7 y = 8 }",
            "position = 7",
        ] {
            assert!(icon_position("iconType", fields).is_err(), "{fields}");
        }
    }

    #[test]
    fn icon_position_rejects_scalar_types_and_prefixes_without_integer_token_admission() {
        for operand in ["\"7\"", "@x", "7%", "7.0", "7suffix", "0x20", "{ x = 7 }"] {
            let fields = format!("position = {{ x = {operand} y = 8 }}");
            assert!(icon_position("iconType", &fields).is_err(), "{fields}");
        }
    }

    #[test]
    fn icon_position_eof_never_borrows_constructor_zero_or_previous_axis() {
        use crate::paradox_text::{Scalar, Value};
        for text in ["", " ", "+", "-", "0x", "-0X", "\0ignored"] {
            let operand = Value::Scalar(Scalar::Bare(text.into()));
            assert!(
                parse_icon_integer_token(&operand)
                    .unwrap_err()
                    .contains("EOF assignment"),
                "{text:?}"
            );
        }
        assert!(icon_position("iconType", "position = { x = 7 y = \"\" }").is_err());
    }

    #[test]
    fn icon_position_does_not_admit_background_custom_or_3d_reader() {
        for kind in [
            "background",
            "buttonType",
            "customIconType",
            "containerWindowType",
        ] {
            assert!(
                icon_position(kind, "position = { x = 7 y = 8 }").is_err(),
                "{kind}"
            );
        }
        assert!(icon_position("iconType", "position3D = { x = 7 y = 8 z = 9 }").is_err());
    }
}
