//! Selected rejection demand in 140DE95A0 / 140DF7D20: unit vf17 (+0x88)
//! runs only while the shared rejection buffer is empty. Its return value
//! does not gate the next member. Class-wrapper gates run before common
//! BEADD0; missing route/transport producer operands refuse when reached.

use super::{Class, Tooltip, line, refuse};
use crate::map::division_eta::{CounterQueries, CounterUnit};

fn first_rejection(
    members: &[u64],
    mut query: impl FnMut(u64, &mut String) -> Result<bool, String>,
) -> Result<String, String> {
    let mut text = String::new();
    for &handle in members {
        if !text.is_empty() {
            break;
        }
        let _ = query(handle, &mut text)?;
    }
    Ok(text)
}

pub(super) fn append(
    out: &mut Tooltip,
    class: Class,
    members: &[u64],
    target: u32,
    queries: &CounterQueries<'_>,
) -> Result<bool, String> {
    append_queried(out, class, members, |unit, text| {
        queries.selected_move_message(unit, target, text)
    })
}

fn append_queried(
    out: &mut Tooltip,
    class: Class,
    members: &[u64],
    mut query: impl FnMut(CounterUnit, &mut String) -> Result<bool, String>,
) -> Result<bool, String> {
    let message = first_rejection(members, |handle, text| {
        query(
            match class {
                Class::Army => CounterUnit::Division(handle),
                Class::Railway => CounterUnit::Railway(handle),
            },
            text,
        )
    });
    match message {
        // 140DE95A0 / 140DF7D20 own the outer red/reset bytes, even when
        // the effective localisation includes its own colour controls.
        Ok(text) => {
            let nonempty = !text.is_empty();
            if nonempty {
                line(
                    out,
                    "move rejection",
                    1,
                    Ok(format!("\u{11}R{text}\u{11}!")),
                );
            }
            Ok(nonempty)
        }
        Err(reason) => {
            refuse(out, "move rejection", reason.clone());
            Err(reason)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_rejections_wrap_both_classes_and_keep_authored_colour_controls() {
        for class in [Class::Army, Class::Railway] {
            for text in ["native rejection", "before \u{11}Yauthored\u{11}! after"] {
                let mut calls = Vec::new();
                let mut out = Tooltip {
                    title: "Selection".into(),
                    ..Default::default()
                };
                assert_eq!(
                    append_queried(&mut out, class, &[7, 9], |unit, buffer| {
                        calls.push(unit);
                        buffer.push_str(text);
                        Ok(false)
                    }),
                    Ok(true)
                );
                assert_eq!(
                    calls,
                    [match class {
                        Class::Army => CounterUnit::Division(7),
                        Class::Railway => CounterUnit::Railway(7),
                    }]
                );
                assert_eq!(out.title, format!("Selection\n    \u{11}R{text}\u{11}!"));
                assert!(out.refused.is_empty());
            }
        }
    }

    #[test]
    fn empty_and_refused_selected_rejections_do_not_emit_colour_controls() {
        let mut out = Tooltip::default();
        assert_eq!(
            append_queried(&mut out, Class::Army, &[7], |_, _| Ok(false)),
            Ok(false)
        );
        assert!(out.title.is_empty());
        assert!(
            append_queried(&mut out, Class::Railway, &[7], |_, _| {
                Err("wrapper input unavailable".into())
            })
            .is_err()
        );
        assert!(out.title.is_empty());
        assert_eq!(
            out.refused["province selected move rejection: wrapper input unavailable"],
            1
        );
    }

    #[test]
    fn false_empty_retries_next_occurrence_and_first_text_stops_demand() {
        let mut calls = Vec::new();
        let text = first_rejection(&[7, 7, 9], |handle, text| {
            calls.push(handle);
            if calls.len() == 2 {
                text.push_str("native rejection");
            }
            Ok(false)
        })
        .unwrap();
        assert_eq!(calls, [7, 7]);
        assert_eq!(text, "native rejection");
    }

    #[test]
    fn true_empty_and_empty_selection_do_not_invent_text() {
        let mut calls = Vec::new();
        assert_eq!(
            first_rejection(&[7, 9], |handle, _| {
                calls.push(handle);
                Ok(true)
            }),
            Ok(String::new())
        );
        assert_eq!(calls, [7, 9]);
        assert_eq!(
            first_rejection(&[], |_, _| panic!("empty selection queried")),
            Ok(String::new())
        );
    }

    #[test]
    fn unknown_empty_result_stops_before_a_later_message() {
        let mut calls = Vec::new();
        assert_eq!(
            first_rejection(&[7, 9], |handle, _| {
                calls.push(handle);
                Err("wrapper input unavailable".into())
            }),
            Err("wrapper input unavailable".into())
        );
        assert_eq!(calls, [7]);
    }
}
