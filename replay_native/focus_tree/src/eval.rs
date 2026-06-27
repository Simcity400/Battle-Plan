//! The deliberately-small OFFSET-TRIGGER EVALUATOR (plan §E).
//!
//! Scope is bounded by an explicit SUPPORTED-SUBSET list. It evaluates only the
//! cheap, data-backed triggers that key off state we actually record:
//!
//!   * `tag` / `original_tag`            — the observed country tag
//!   * `has_completed_focus = ID`        — membership in the recorded completed set
//!   * `has_game_rule = { rule option }` — the recorded game-rule selections
//!   * `has_dlc = "Name"`                — the recorded DLC set
//!   * `has_global_flag = flag`          — the recorded global-flag set
//!   * `always = yes/no`
//!   * `AND` / `OR` / `NOT` / `NAND` / `NOR` nesting (NAND/NOR via NOT(AND/OR))
//!
//! ANYTHING ELSE (counts, comparisons, live predicates, unrecorded flags) makes
//! the WHOLE trigger UNSUPPORTED: we do NOT guess. An unsupported trigger yields
//! [`Eval::Unsupported`] with the offending key recorded, the caller falls back
//! to the base x/y and BADGES the node `approximate`. We never fabricate a TRUE
//! (which would move a node as if a hidden condition held) nor a FALSE.

use std::collections::HashSet;

use crate::pdx::{Op, Value};

/// The recorded replay context the evaluator reads. Everything is "what we have
/// in the snapshot/replay", never a live computation. Fields that the snapshot
/// does not carry are simply empty/None — and the evaluator treats a trigger
/// that *needs* an absent capability as UNSUPPORTED, not false (no fabrication).
#[derive(Debug, Clone, Default)]
pub struct EvalContext {
    pub tag: String,
    pub original_tag: Option<String>,
    pub completed_focuses: HashSet<String>,
    /// rule -> option (only those we recorded). If `game_rules_known` is false,
    /// any `has_game_rule` is treated as UNSUPPORTED rather than false.
    pub game_rules: std::collections::HashMap<String, String>,
    pub game_rules_known: bool,
    /// Recorded DLC names. If `dlc_known` is false, `has_dlc` is UNSUPPORTED.
    pub dlc: HashSet<String>,
    pub dlc_known: bool,
    /// Recorded global flags. If `global_flags_known` is false, `has_global_flag`
    /// is UNSUPPORTED.
    pub global_flags: HashSet<String>,
    pub global_flags_known: bool,
}

/// The result of evaluating a trigger block.
#[derive(Debug, Clone, PartialEq)]
pub enum Eval {
    True,
    False,
    /// The trigger uses something outside the supported subset (or needs a
    /// capability we did not record). Carries the offending key for logging +
    /// badging. The caller MUST fall back to base position + mark approximate.
    Unsupported {
        reason: String,
    },
}

impl Eval {
    pub fn is_unsupported(&self) -> bool {
        matches!(self, Eval::Unsupported { .. })
    }
}

fn truthy(s: &str) -> bool {
    matches!(s, "yes" | "YES" | "Yes" | "true" | "1")
}

/// Evaluate a `trigger = { … }` block's INNER content (the block value passed in
/// is the trigger block itself). An empty trigger is vacuously TRUE.
pub fn eval_trigger(block: &Value, ctx: &EvalContext) -> Eval {
    let entries = match block.as_block() {
        Some(e) => e,
        // a scalar trigger is not something we model -> unsupported
        None => {
            return Eval::Unsupported {
                reason: "non-block trigger".into(),
            }
        }
    };
    // implicit AND over all entries
    let mut all_true = true;
    for e in entries {
        // Only `=`/`==` are used by the supported subset. A comparison operator
        // (`>`,`<`,`>=`,`<=`) implies a numeric live predicate -> unsupported.
        if !matches!(e.op, Op::Eq | Op::EqEq) {
            return Eval::Unsupported {
                reason: format!("{}{}", e.key, op_str(e.op)),
            };
        }
        match eval_entry(&e.key, &e.value, ctx) {
            Eval::True => {}
            Eval::False => all_true = false,
            u @ Eval::Unsupported { .. } => return u,
        }
    }
    if all_true {
        Eval::True
    } else {
        Eval::False
    }
}

fn op_str(op: Op) -> &'static str {
    match op {
        Op::Gt => " >",
        Op::Lt => " <",
        Op::Ge => " >=",
        Op::Le => " <=",
        _ => " =",
    }
}

fn eval_entry(key: &str, value: &Value, ctx: &EvalContext) -> Eval {
    match key {
        "always" => match value.as_scalar() {
            // `always = yes/no` is the only valid form; a non-scalar (`always = {…}`)
            // is unparseable -> UNSUPPORTED, not a fabricated False (Codex FIX-C
            // consistency: same shape-class as the scalar predicates below).
            Some(s) => boolean(truthy(s)),
            None => Eval::Unsupported {
                reason: "always (non-scalar value)".into(),
            },
        },
        // Scalar-expecting predicates: a NON-scalar value (a block) is an
        // unparseable shape -> UNSUPPORTED, never a fabricated definite FALSE
        // (Codex FIX-C: `None == Some(tag)` used to collapse to Eval::False, which
        // could wrongly suppress a node).
        "tag" => match value.as_scalar() {
            Some(want) => boolean(want == ctx.tag.as_str()),
            None => Eval::Unsupported {
                reason: "tag (non-scalar value)".into(),
            },
        },
        "original_tag" => match value.as_scalar() {
            Some(want) => {
                // original_tag defaults to the current tag when no tag-change was
                // recorded (engine semantics): None means "never changed".
                let have = ctx.original_tag.as_deref().unwrap_or(ctx.tag.as_str());
                boolean(want == have)
            }
            None => Eval::Unsupported {
                reason: "original_tag (non-scalar value)".into(),
            },
        },
        "has_completed_focus" => match value.as_scalar() {
            Some(id) => boolean(ctx.completed_focuses.contains(id)),
            None => Eval::Unsupported {
                reason: "has_completed_focus (non-scalar value)".into(),
            },
        },
        "has_game_rule" => {
            if !ctx.game_rules_known {
                return Eval::Unsupported {
                    reason: "has_game_rule (rules not recorded)".into(),
                };
            }
            // form: has_game_rule = { rule = R option = O }
            let rule = value.get_scalar("rule");
            let option = value.get_scalar("option");
            match (rule, option) {
                (Some(r), Some(o)) => {
                    boolean(ctx.game_rules.get(r).map(|v| v == o).unwrap_or(false))
                }
                // scalar form `has_game_rule = R` is not a thing for options
                _ => Eval::Unsupported {
                    reason: "has_game_rule (unrecognized form)".into(),
                },
            }
        }
        "has_dlc" => {
            if !ctx.dlc_known {
                return Eval::Unsupported {
                    reason: "has_dlc (dlc not recorded)".into(),
                };
            }
            match value.as_scalar() {
                Some(name) => boolean(ctx.dlc.contains(name)),
                None => Eval::Unsupported {
                    reason: "has_dlc (non-scalar value)".into(),
                },
            }
        }
        "has_global_flag" => {
            if !ctx.global_flags_known {
                return Eval::Unsupported {
                    reason: "has_global_flag (flags not recorded)".into(),
                };
            }
            // form: has_global_flag = flag  OR  has_global_flag = { flag = F … }
            let flag = value
                .as_scalar()
                .map(|s| s.to_string())
                .or_else(|| value.get_scalar("flag").map(|s| s.to_string()));
            match flag {
                Some(f) => boolean(ctx.global_flags.contains(&f)),
                None => Eval::Unsupported {
                    reason: "has_global_flag (unrecognized form)".into(),
                },
            }
        }
        "AND" => eval_trigger(value, ctx),
        "NAND" => negate(eval_trigger(value, ctx)),
        "OR" => eval_or(value, ctx),
        "NOR" => negate(eval_or(value, ctx)),
        "NOT" => {
            // Clausewitz `NOT` is UNARY: `NOT = { x }` negates the single child.
            // A MULTI-child `NOT = { a b c }` is engine-warned-against and its
            // intended meaning is ambiguous (none-of vs not-all) — under the
            // no-fabrication mandate (Codex-confirmed) we MUST NOT assume `NOR`;
            // treat it as UNSUPPORTED (badge + fall back) rather than guess.
            // `NOR`/`NAND` are the explicit none-of / not-all forms above.
            match value.as_block() {
                Some(entries) if entries.len() == 1 => negate(eval_trigger(value, ctx)),
                Some(_) => Eval::Unsupported {
                    reason: "NOT with multiple children (ambiguous; use NOR/NAND)".into(),
                },
                None => Eval::Unsupported {
                    reason: "non-block NOT".into(),
                },
            }
        }
        other => Eval::Unsupported {
            reason: other.to_string(),
        },
    }
}

fn eval_or(block: &Value, ctx: &EvalContext) -> Eval {
    let entries = match block.as_block() {
        Some(e) => e,
        None => {
            return Eval::Unsupported {
                reason: "non-block OR".into(),
            }
        }
    };
    let mut any_true = false;
    let mut any_unsupported: Option<String> = None;
    for e in entries {
        // A comparison operator inside an OR arm is itself unsupported, but it
        // must NOT short-circuit the whole OR: a sibling arm may be definitively
        // TRUE. Defer it like any other unsupported arm.
        if !matches!(e.op, Op::Eq | Op::EqEq) {
            any_unsupported.get_or_insert(format!("{}{}", e.key, op_str(e.op)));
            continue;
        }
        match eval_entry(&e.key, &e.value, ctx) {
            Eval::True => any_true = true,
            Eval::False => {}
            Eval::Unsupported { reason } => {
                // an OR whose other arms are all FALSE but has an unsupported arm
                // cannot be safely resolved -> remember and bail unsupported
                // unless some arm is already definitively true.
                any_unsupported.get_or_insert(reason);
            }
        }
    }
    if any_true {
        Eval::True
    } else if let Some(r) = any_unsupported {
        Eval::Unsupported { reason: r }
    } else {
        Eval::False
    }
}

fn negate(e: Eval) -> Eval {
    match e {
        Eval::True => Eval::False,
        Eval::False => Eval::True,
        u => u,
    }
}

fn boolean(b: bool) -> Eval {
    if b {
        Eval::True
    } else {
        Eval::False
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdx::parse_file;

    fn ctx() -> EvalContext {
        let mut c = EvalContext {
            tag: "ENG".into(),
            game_rules_known: true,
            ..Default::default()
        };
        c.completed_focuses.insert("ENG_a_change_in_course".into());
        c.game_rules
            .insert("obsolete_focus_branches_visibility".into(), "HIDE".into());
        c
    }

    fn trig(src: &str) -> Value {
        // src like: trigger = { ... }
        let top = parse_file(src);
        top.into_iter().next().unwrap().value
    }

    #[test]
    fn supported_real_offset_trigger_true() {
        // the real abdacom_shared_branch ENG offset trigger
        let t = trig(
            r#"trigger = {
                tag = ENG
                has_game_rule = { rule = obsolete_focus_branches_visibility option = HIDE }
                has_completed_focus = ENG_a_change_in_course
            }"#,
        );
        assert_eq!(eval_trigger(&t, &ctx()), Eval::True);
    }

    #[test]
    fn supported_offset_trigger_false_wrong_tag() {
        let mut c = ctx();
        c.tag = "USA".into();
        let t = trig(r#"trigger = { tag = ENG has_completed_focus = ENG_a_change_in_course }"#);
        assert_eq!(eval_trigger(&t, &c), Eval::False);
    }

    #[test]
    fn missing_completed_focus_is_false_not_unsupported() {
        let mut c = ctx();
        c.completed_focuses.clear();
        let t = trig(r#"trigger = { tag = ENG has_completed_focus = ENG_a_change_in_course }"#);
        assert_eq!(eval_trigger(&t, &c), Eval::False);
    }

    #[test]
    fn unsupported_trigger_badges_not_fabricates() {
        let t = trig(r#"trigger = { num_of_factories > 5 }"#);
        let r = eval_trigger(&t, &ctx());
        assert!(r.is_unsupported(), "got {r:?}");
    }

    #[test]
    fn unsupported_unknown_key() {
        let t = trig(r#"trigger = { has_war_with = SOV }"#);
        assert!(eval_trigger(&t, &ctx()).is_unsupported());
    }

    #[test]
    fn game_rule_unsupported_when_not_recorded() {
        let mut c = ctx();
        c.game_rules_known = false;
        let t = trig(r#"trigger = { has_game_rule = { rule = r option = o } }"#);
        assert!(eval_trigger(&t, &c).is_unsupported());
    }

    #[test]
    fn multi_child_not_is_unsupported_not_assumed_nor() {
        // Codex-confirmed: a multi-child NOT is ambiguous; never assume NOR.
        let c = ctx();
        let r = eval_trigger(&trig(r#"trigger = { NOT = { tag = USA tag = SOV } }"#), &c);
        assert!(
            r.is_unsupported(),
            "multi-child NOT must badge, not guess: {r:?}"
        );
        // the explicit NOR form IS supported (none-of)
        assert_eq!(
            eval_trigger(&trig(r#"trigger = { NOR = { tag = USA tag = SOV } }"#), &c),
            Eval::True
        );
    }

    #[test]
    fn not_and_or_nesting() {
        let c = ctx();
        // unary NOT: NOT={ tag=USA } -> true (USA not the tag)
        assert_eq!(
            eval_trigger(&trig(r#"trigger = { NOT = { tag = USA } }"#), &c),
            Eval::True
        );
        // OR with one true arm
        assert_eq!(
            eval_trigger(&trig(r#"trigger = { OR = { tag = USA tag = ENG } }"#), &c),
            Eval::True
        );
        // AND both true
        assert_eq!(
            eval_trigger(
                &trig(
                    r#"trigger = { AND = { tag = ENG has_completed_focus = ENG_a_change_in_course } }"#
                ),
                &c
            ),
            Eval::True
        );
    }

    #[test]
    fn non_scalar_predicate_is_unsupported_not_false() {
        // BUG (Codex FIX-C): a scalar-expecting predicate given a BLOCK value used to
        // fall through `as_scalar()==Some(..)` as `None==Some(..)` -> Eval::False,
        // FABRICATING a known-false from an unparseable shape (which could wrongly
        // suppress a node). It must be Unsupported (badge + fall back to base x/y),
        // never a definite False.
        let c = ctx();
        for src in [
            r#"trigger = { tag = { foo = bar } }"#,
            r#"trigger = { original_tag = { x = y } }"#,
            r#"trigger = { has_completed_focus = { a = b } }"#,
            r#"trigger = { always = { a = b } }"#,
        ] {
            let r = eval_trigger(&trig(src), &c);
            assert!(
                r.is_unsupported(),
                "non-scalar predicate must badge, got {r:?} for {src}"
            );
        }
        // a non-scalar has_dlc (after dlc_known) is also unsupported, not false.
        let mut c2 = ctx();
        c2.dlc_known = true;
        assert!(
            eval_trigger(&trig(r#"trigger = { has_dlc = { n = m } }"#), &c2).is_unsupported(),
            "non-scalar has_dlc must badge, not fabricate false"
        );
    }

    #[test]
    fn or_with_unsupported_arm_and_no_true_is_unsupported() {
        let c = ctx();
        let t = trig(r#"trigger = { OR = { tag = USA num_of_factories > 5 } }"#);
        assert!(eval_trigger(&t, &c).is_unsupported());
    }

    #[test]
    fn or_with_true_arm_ignores_unsupported() {
        let c = ctx();
        let t = trig(r#"trigger = { OR = { tag = ENG num_of_factories > 5 } }"#);
        assert_eq!(eval_trigger(&t, &c), Eval::True);
    }
}
