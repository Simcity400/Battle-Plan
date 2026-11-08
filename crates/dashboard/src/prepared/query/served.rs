use super::*;
use std::{io::Write as _, sync::Arc};

/// An answer's data as the JSON text its replies serve, written once, with where a reply
/// changes it ([`Marks`]): a reply copies the rest of the text as it is, never parsing it.
pub(super) struct Answer {
    text: Arc<String>,
    marks: Marks,
}

/// Where a reply changes an answer's text, as byte ranges of it.
#[derive(Clone, Default, Serialize, Deserialize)]
struct Marks {
    /// The values the product's warning paths reach ([`content_warning_paths`]), in text order,
    /// none inside another.
    lists: Vec<(usize, usize)>,
    /// Which of `lists` takes the lines every reply adds: the first value the first path
    /// reaching any reaches.
    first: Option<usize>,
    /// Where the root's `served` field goes, for a reply that adds one.
    served: Option<Field>,
}

/// A root field's place: the range of text it replaces, empty where it is inserted, and the
/// commas it needs before and after.
#[derive(Clone, Copy, Serialize, Deserialize)]
struct Field {
    start: usize,
    end: usize,
    before: bool,
    after: bool,
    /// Whether the range is the field's value alone, its key kept.
    #[serde(default)]
    value_only: bool,
}

impl Answer {
    /// `data` as JSON within the reply budget, stopping once `cancelled`, marking the values
    /// `paths` reach and, when `served`, where a root `served` field goes.
    pub(super) fn new(
        data: Data,
        paths: &[&[&str]],
        served: bool,
        cancelled: &AtomicBool,
    ) -> io::Result<Self> {
        let (text, reached, served) = match data {
            Data::Tree(data) => {
                let mut writer = Writer {
                    text: Vec::new(),
                    cancelled,
                    reached: Vec::new(),
                    served: None,
                };
                let active: Vec<(usize, &[&str])> = paths.iter().copied().enumerate().collect();
                if let Err(error) = writer.value(&data, &active, served) {
                    // A cancelled write stops as every cancelled request does.
                    check_cancel(cancelled)?;
                    return Err(error);
                }
                let text = String::from_utf8(writer.text).map_err(io::Error::other)?;
                (text, writer.reached, writer.served)
            }
            Data::Built(built) => {
                let text = built.finish()?;
                if text.len() > MAX_REPLY_BYTES {
                    return Err(crate::invalid(
                        "Projected response exceeds the transport byte budget",
                    ));
                }
                check_cancel(cancelled)?;
                // The written text is scanned once for what replies change, never parsed whole.
                let mut reached: Vec<_> = dashboard_base::products::built::reach(&text, paths)?
                    .into_iter()
                    .map(|(path, range, list)| (path, range.start, range.end, list))
                    .collect();
                reached.sort_by_key(|&(_, start, ..)| start);
                let served = if served {
                    let field = dashboard_base::products::built::reach(&text, &[&["served"]])?
                        .first()
                        .map(|(_, range, _)| range.clone());
                    match field {
                        // The value is replaced; its key stays.
                        Some(range) => Some(Field {
                            start: range.start,
                            end: range.end,
                            before: false,
                            after: false,
                            value_only: true,
                        }),
                        None => dashboard_base::products::built::root_close(&text)?.map(
                            |(close, fields)| Field {
                                start: close,
                                end: close,
                                before: fields,
                                after: false,
                                value_only: false,
                            },
                        ),
                    }
                } else {
                    None
                };
                (text, reached, served)
            }
        };
        let first = (0..paths.len()).find_map(|path| {
            reached
                .iter()
                .filter(|reached| reached.0 == path)
                .map(|&(_, start, end, list)| (start, end, list))
                .min()
        });
        // Only a list is replaced.
        let mut lists: Vec<_> = reached
            .iter()
            .filter(|reached| reached.3)
            .map(|&(_, start, end, _)| (start, end))
            .collect();
        lists.sort_unstable_by_key(|&(start, end)| (start, std::cmp::Reverse(end)));
        lists.dedup();
        // A list inside another one is written as part of it.
        let mut after = 0;
        lists.retain(|&(start, end)| {
            let outside = start >= after;
            if outside {
                after = end;
            }
            outside
        });
        let first = first
            .filter(|&(_, _, list)| list)
            .and_then(|(start, end, _)| lists.iter().position(|&list| list == (start, end)));
        Ok(Self {
            text: Arc::new(text),
            marks: Marks {
                lists,
                first,
                served,
            },
        })
    }
}

/// Writes an answer's text within the reply budget, stopping once the request is cancelled, as
/// [`bounded_json`] does, noting where the warning paths reach and the root's `served` place.
struct Writer<'c> {
    text: Vec<u8>,
    cancelled: &'c AtomicBool,
    /// The index of a path with the range of a value it reaches, and whether that is a list.
    reached: Vec<(usize, usize, usize, bool)>,
    served: Option<Field>,
}

impl io::Write for Writer<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        // Never `Interrupted`, which `write_all` retries.
        if self.cancelled.load(std::sync::atomic::Ordering::Acquire) {
            return Err(io::Error::other("Dashboard request was cancelled"));
        }
        if self.text.len() + bytes.len() > MAX_REPLY_BYTES {
            return Err(crate::invalid(
                "Projected response exceeds the transport byte budget",
            ));
        }
        self.text
            .try_reserve(bytes.len())
            .map_err(|_| crate::invalid("Not enough memory for projected response"))?;
        self.text.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Writer<'_> {
    /// Writes `value`, which each path in `active` reached with what of it is left to follow,
    /// and, when `served`, notes where its `served` field goes.
    fn value<'p>(
        &mut self,
        value: &Value,
        active: &[(usize, &'p [&'p str])],
        served: bool,
    ) -> io::Result<()> {
        let start = self.text.len();
        let follows = |keyed: bool| {
            active
                .iter()
                .any(|(_, rest)| rest.first().is_some_and(|first| (*first != "*") == keyed))
        };
        match value {
            Value::Object(fields) if served || follows(true) => {
                self.write_all(b"{")?;
                for (index, (key, field)) in fields.iter().enumerate() {
                    if index > 0 {
                        self.write_all(b",")?;
                    }
                    let at = self.text.len();
                    if served && self.served.is_none() && key.as_str() > "served" {
                        self.served = Some(Field {
                            start: at,
                            end: at,
                            before: false,
                            after: true,
                            value_only: false,
                        });
                    }
                    serde_json::to_writer(&mut *self, key)?;
                    self.write_all(b":")?;
                    self.value(field, &following(active, key, false), false)?;
                    if served && self.served.is_none() && key == "served" {
                        self.served = Some(Field {
                            start: at,
                            end: self.text.len(),
                            before: false,
                            after: false,
                            value_only: false,
                        });
                    }
                }
                if served && self.served.is_none() {
                    let at = self.text.len();
                    self.served = Some(Field {
                        start: at,
                        end: at,
                        before: !fields.is_empty(),
                        after: false,
                        value_only: false,
                    });
                }
                self.write_all(b"}")?;
            }
            Value::Array(items) if follows(false) => {
                let next = following(active, "*", true);
                self.write_all(b"[")?;
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        self.write_all(b",")?;
                    }
                    self.value(item, &next, false)?;
                }
                self.write_all(b"]")?;
            }
            value => serde_json::to_writer(&mut *self, value)?,
        }
        let list = value.is_array();
        for &(path, rest) in active {
            if rest.is_empty() {
                self.reached.push((path, start, self.text.len(), list));
            }
        }
        Ok(())
    }
}

/// The paths in `active` whose next step is `segment`, a key, or every item when `wildcard`,
/// with what of each is left after it.
fn following<'p>(
    active: &[(usize, &'p [&'p str])],
    segment: &str,
    wildcard: bool,
) -> Vec<(usize, &'p [&'p str])> {
    active
        .iter()
        .filter_map(|&(path, rest)| match rest.split_first() {
            Some((first, rest)) if (*first == "*") == wildcard && *first == segment => {
                Some((path, rest))
            }
            _ => None,
        })
        .collect()
}

/// A reply's data: its answer's text with each warning list refreshed and the served lines added.
pub(super) struct Served<'a> {
    answer: &'a Answer,
    /// Each of the answer's lists as served, as JSON; none for one that is not a list.
    lists: Vec<Option<String>>,
    /// The root `served` field's value, as JSON.
    served: Option<String>,
}

impl<'a> Served<'a> {
    pub(super) fn new(
        source: &Source,
        answer: &'a Answer,
        content: Option<&ContentOptions>,
        fallbacks: &std::collections::BTreeSet<String>,
        diagnostics: &installed_content_core::content::diagnostics::Diagnostics,
    ) -> Self {
        let lines = served_lines(source, content, fallbacks);
        let mut reply = Self::decorate(answer, lines, |warnings| {
            diagnostics.append_to(warnings);
            source.append_changed_warnings(warnings, content);
        });
        if answer.marks.served.is_some() {
            reply.served = Some(
                Value::from(crate::products::division_equipment::history::served_lines(
                    source, content,
                ))
                .to_string(),
            );
        }
        reply
    }

    fn decorate(
        answer: &'a Answer,
        lines: Vec<String>,
        mut refresh: impl FnMut(&mut Vec<String>),
    ) -> Self {
        let mut lists: Vec<Option<Vec<Value>>> = answer
            .marks
            .lists
            .iter()
            .map(|&(start, end)| {
                let Ok(Value::Array(original)) = serde_json::from_str(&answer.text[start..end])
                else {
                    return None;
                };
                let strings: Option<Vec<String>> = original
                    .iter()
                    .map(|line| line.as_str().map(str::to_owned))
                    .collect();
                Some(match strings {
                    Some(mut strings) => {
                        refresh(&mut strings);
                        strings.into_iter().map(Value::String).collect()
                    }
                    None => original,
                })
            })
            .collect();
        if let Some(Some(warnings)) = answer.marks.first.and_then(|first| lists.get_mut(first)) {
            for line in lines {
                if !warnings.iter().any(|kept| kept.as_str() == Some(&line)) {
                    warnings.push(Value::String(line));
                }
            }
        }
        Self {
            answer,
            lists: lists
                .into_iter()
                .map(|list| list.map(|list| Value::Array(list).to_string()))
                .collect(),
            served: None,
        }
    }

    /// The served data's text, piece by piece.
    fn pieces(&self) -> Vec<&str> {
        let text = self.answer.text.as_str();
        let marks = &self.answer.marks;
        let mut splices: Vec<(usize, usize, [&str; 4])> = marks
            .lists
            .iter()
            .zip(&self.lists)
            .filter_map(|(&(start, end), list)| Some((start, end, [list.as_deref()?, "", "", ""])))
            .collect();
        if let (Some(field), Some(served)) = (marks.served, &self.served) {
            let comma = |needed: bool| if needed { "," } else { "" };
            let key = if field.value_only { "" } else { "\"served\":" };
            splices.push((
                field.start,
                field.end,
                [comma(field.before), key, served.as_str(), comma(field.after)],
            ));
            splices.sort_by_key(|&(start, ..)| start);
        }
        let mut pieces = Vec::with_capacity(splices.len() * 5 + 1);
        let mut at = 0;
        for (start, end, replacement) in splices {
            // Marks never overlap; a kept one that would is left as it is.
            if start < at {
                continue;
            }
            pieces.push(&text[at..start]);
            pieces.extend(replacement);
            at = end;
        }
        pieces.push(&text[at..]);
        pieces
    }

    /// The bytes [`Self::write`] appends.
    pub(super) fn len(&self) -> usize {
        self.pieces().iter().map(|piece| piece.len()).sum()
    }

    /// Appends the served data's text to `reply`, which has room for it.
    pub(super) fn write(&self, reply: &mut String) {
        for piece in self.pieces() {
            reply.push_str(piece);
        }
    }
}

#[cfg(test)]
mod tests;
