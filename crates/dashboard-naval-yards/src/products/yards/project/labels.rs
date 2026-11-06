//! Naming operands as rows carry them: each spelled once, its spelling the key its branch numbers
//! it by, and kept so an operand read again is not spelled again.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(replay_spool::Spooled)]
pub(crate) struct Label {
    /// The operand's spelling.
    pub(crate) key: String,
    pub(crate) operand: Operand,
    /// Whether an hour handed to a branch carried it yet: the walk's charging alone.
    #[spool(skip)]
    carried: AtomicBool,
}

impl Label {
    pub(crate) fn new(operand: Operand) -> Self {
        Self {
            key: serde_json::to_string(&operand).expect("serializable naming operand"),
            operand,
            carried: AtomicBool::new(false),
        }
    }

    /// What it holds, charged to the first hour handed on that carries it.
    pub(crate) fn carry(&self) -> usize {
        if self.carried.swap(true, Ordering::Relaxed) {
            return 0;
        }
        let mut bytes = HeldBytes::default();
        self.operand.held(&mut bytes);
        size_of::<Self>()
            + 2 * size_of::<usize>()
            + self.key.capacity()
            + usize::try_from(bytes.total()).unwrap_or(usize::MAX)
    }
}

/// `operand`'s label: the one `held` carries where it is the same operand.
pub(super) fn label(held: Option<&Spelled>, operand: Operand) -> Arc<Label> {
    match held {
        Some(Spelled::Token(label)) if label.operand == operand => label.clone(),
        _ => Arc::new(Label::new(operand)),
    }
}

/// The refusals' reasons, numbered in the order a segment first counts them.
#[derive(Default)]
pub(super) struct Reasons {
    labels: Vec<Arc<Label>>,
    numbers: HashMap<String, u32>,
}

impl Reasons {
    pub(super) fn number(&mut self, why: &str) -> u32 {
        if let Some(&number) = self.numbers.get(why) {
            return number;
        }
        let number = self.labels.len() as u32;
        self.labels
            .push(Arc::new(Label::new(Operand::Refusal(why.to_owned()))));
        self.numbers.insert(why.to_owned(), number);
        number
    }

    pub(super) fn label(&self, number: u32) -> &Arc<Label> {
        &self.labels[number as usize]
    }
}

/// Labels of the operands many rows share, until the tables they spell change.
#[derive(Default)]
pub(super) struct Shared {
    policies: HashMap<(bool, i32), Arc<Label>>,
    /// By class token and ship-definition index, while LexicalTokens stays as it was.
    classes: HashMap<(u32, Option<i32>), Arc<Label>>,
    /// By variant handle, while its definition, the country tags and its equipment token's
    /// spelling stay as they were; with that token.
    designs: HashMap<u64, (Option<u32>, Result<Arc<Label>, String>)>,
}

impl Shared {
    pub(super) fn policy(&mut self, port: bool, value: i32) -> Arc<Label> {
        self.policies
            .entry((port, value))
            .or_insert_with(|| Arc::new(Label::new(Operand::Policy { port, value })))
            .clone()
    }

    pub(super) fn class<F: Tables + ?Sized>(
        &mut self,
        frame: &Frame<'_, F>,
        token: u32,
        index: Option<i32>,
    ) -> Arc<Label> {
        self.classes
            .entry((token, index))
            .or_insert_with(|| {
                let spelling = frame
                    .get::<tables::LexicalTokens>(token)
                    .ok_or_else(|| {
                    format!("Ship class token {token}: LexicalTokens does not supply its spelling")
                })
                .and_then(|row| {
                    std::str::from_utf8(&row.literal)
                        .map(str::to_owned)
                        .map_err(|_| "Ship class token is not UTF-8".to_owned())
                });
                Arc::new(Label::new(Operand::Class {
                    token,
                    spelling,
                    index,
                }))
            })
            .clone()
    }

    /// The name of the design `handle`, or why there is none.
    pub(super) fn design<F: Tables + ?Sized>(
        &mut self,
        naming: &mut Naming<'_, F>,
        handle: u64,
    ) -> Result<Arc<Label>, String> {
        if let Some((_, held)) = self.designs.get(&handle) {
            return held.clone();
        }
        let frame = naming.frame;
        let (token, named) = match naming.get() {
            Err(why) => (None, Err(why.clone())),
            Ok((variants, tags, lexical)) => match frame.found(variants.get(u128::from(handle))) {
                Some(definition) => {
                    let token = definition.equipment_token;
                    let literal = (*lexical)
                        .and_then(|rows| frame.found(rows.get(u128::from(token))));
                    let literal = literal.as_ref().map(|row| row.literal.as_slice());
                    (
                        Some(token),
                        Ok(Arc::new(Label::new(design_operand(&definition, tags, literal)))),
                    )
                }
                None => (
                    None,
                    Err(format!("Design {handle}: VariantDefinitions has no matching design")),
                ),
            },
        };
        self.designs.insert(handle, (token, named.clone()));
        named
    }

    /// Forgets the names of the designs `handles`, whose definitions changed.
    pub(super) fn redefined(&mut self, handles: impl IntoIterator<Item = u64>) {
        for handle in handles {
            self.designs.remove(&handle);
        }
    }

    /// Forgets the labels spelling any of `tokens`, ascending, which LexicalTokens changed; the
    /// variants whose names it forgot.
    pub(super) fn relexed(&mut self, tokens: &[u32]) -> Vec<u64> {
        let touched = |token: &u32| tokens.binary_search(token).is_ok();
        self.classes.retain(|(token, _), _| !touched(token));
        let mut variants = Vec::new();
        self.designs.retain(|&handle, (token, _)| {
            let kept = !token.as_ref().is_some_and(touched);
            if !kept {
                variants.push(handle);
            }
            kept
        });
        variants
    }
}

/// The design names' tables: definitions, country tags and lexical tokens, these only when
/// complete.
type Designs<'f> = (
    Rowset<'f, rows::VariantDefinitionRow>,
    Tags<'f>,
    Option<Rowset<'f, rows::LexicalTokenRow>>,
);

/// The rows country tags are read from, where an hour's tables are a walk's views: built at
/// the first design an hour names.
#[derive(Default)]
pub(super) struct Store {
    countries: std::cell::OnceCell<Vec<rows::CountryRow>>,
    links: std::cell::OnceCell<Vec<rows::CountryLinkRow>>,
    appearance: std::cell::OnceCell<Vec<rows::CountryAppearanceRow>>,
}

/// `rows` as one slice: a decoded frame's own, else built once into `held`.
fn slice<'s, F: Tables + ?Sized, R: rows::RowCodec>(
    frame: &Frame<'s, F>,
    rows: Rowset<'s, R>,
    held: &'s std::cell::OnceCell<Vec<R>>,
) -> &'s [R] {
    match rows {
        Rowset::Decoded(rows) => rows,
        walked => held.get_or_init(|| {
            frame
                .each(walked.iter())
                .into_iter()
                .map(Cow::into_owned)
                .collect()
        }),
    }
}

/// An hour's design naming tables, read at the first design it names.
pub(super) struct Naming<'f, F: ?Sized> {
    frame: &'f Frame<'f, F>,
    store: &'f Store,
    read: Option<Result<Designs<'f>, String>>,
}

impl<'f, F: Tables + ?Sized> Naming<'f, F> {
    pub(super) fn new(frame: &'f Frame<'f, F>, store: &'f Store) -> Self {
        Self {
            frame,
            store,
            read: None,
        }
    }

    fn get(&mut self) -> &Result<Designs<'f>, String> {
        let (frame, store) = (self.frame, self.store);
        self.read.get_or_insert_with(|| {
            let variants = frame.complete_or::<tables::VariantDefinitions>()?;
            let countries = frame.complete_or::<tables::Countries>()?;
            let links = frame.complete_or::<tables::CountryLinks>()?;
            let lexical = frame.complete::<tables::LexicalTokens>();
            let appearance = frame.complete::<tables::CountryAppearance>();
            let tags = Tags::new(
                slice(frame, countries, &store.countries),
                slice(frame, links, &store.links),
                appearance.map(|rows| slice(frame, rows, &store.appearance)),
            );
            Ok((variants, tags, lexical))
        })
    }
}
