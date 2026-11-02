use crate::TableId;
use crate::store::invalid;
use std::io;
use std::mem::size_of;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Publication(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Publication(Publication),
    Table(TableId),
}

#[derive(Clone, Copy, Debug)]
pub enum KeyFilter<'a> {
    All,
    Prefix(&'a [u8]),
    Keys(&'a [&'a [u8]]),
}

#[derive(Clone, Copy, Debug)]
pub struct Subscription<'a> {
    pub source: Source,
    pub fields: &'a [usize],
    pub filter: KeyFilter<'a>,
}

#[derive(Clone, Copy, Debug)]
pub struct Declaration<'a> {
    pub output: Publication,
    pub inputs: &'a [Subscription<'a>],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Day {
    pub segment: u64,
    pub day: u64,
    pub first: bool,
}

pub trait Publications<R> {
    fn ready(&self, source: Source, day: Day) -> bool;
    fn changed(&self, input: Subscription<'_>, day: Day) -> bool;
    fn read(
        &mut self,
        input: Subscription<'_>,
        day: Day,
        visit: &mut dyn FnMut(&R) -> io::Result<()>,
    ) -> io::Result<()>;
    fn publish(&mut self, publication: Publication, day: Day, record: &R) -> io::Result<()>;
    fn finish(&mut self, publication: Publication, day: Day) -> io::Result<()>;
}

pub trait DerivedStage<R> {
    fn declaration(&self) -> Declaration<'_>;
    fn run(&mut self, day: Day, publications: &mut dyn Publications<R>) -> io::Result<()>;
}

pub struct StageOrder {
    order: Vec<usize>,
    nodes: Vec<Planned>,
    inputs: Vec<Source>,
    pub reserved_bytes: usize,
}

struct Planned {
    output: Publication,
    first: usize,
    end: usize,
}

impl StageOrder {
    pub fn new(
        declarations: &[Declaration<'_>],
        external: &[Publication],
        allowance: usize,
    ) -> io::Result<Self> {
        let input_count = declarations.iter().try_fold(0usize, |count, declaration| {
            count
                .checked_add(declaration.inputs.len())
                .ok_or_else(|| invalid("stage metadata overflow"))
        })?;
        let bytes = declarations
            .len()
            .checked_mul(size_of::<usize>() + size_of::<Planned>())
            .and_then(|bytes| bytes.checked_add(input_count.checked_mul(size_of::<Source>())?))
            .and_then(|bytes| bytes.checked_add(size_of::<Self>()))
            .ok_or_else(|| invalid("stage metadata overflow"))?;
        if bytes > allowance {
            return Err(invalid("stage order exceeds its metadata allowance"));
        }
        for (at, declaration) in declarations.iter().enumerate() {
            if external.contains(&declaration.output)
                || declarations[..at]
                    .iter()
                    .any(|other| other.output == declaration.output)
            {
                return Err(invalid("publication has multiple producers"));
            }
            for input in declaration.inputs {
                if let Source::Table(table) = input.source {
                    if input
                        .fields
                        .iter()
                        .any(|&field| table.field_name(field).is_none())
                    {
                        return Err(invalid("stage input names an invalid table field"));
                    }
                }
                if !matches!(input.source, Source::Table(_))
                    && !external
                        .iter()
                        .any(|&p| input.source == Source::Publication(p))
                    && !declarations
                        .iter()
                        .any(|other| Source::Publication(other.output) == input.source)
                {
                    return Err(invalid("stage input has no producer"));
                }
            }
        }
        let mut order = Vec::new();
        let mut nodes = Vec::new();
        let mut inputs = Vec::new();
        nodes
            .try_reserve_exact(declarations.len())
            .map_err(io::Error::other)?;
        inputs
            .try_reserve_exact(input_count)
            .map_err(io::Error::other)?;
        for declaration in declarations {
            let first = inputs.len();
            inputs.extend(declaration.inputs.iter().map(|input| input.source));
            nodes.push(Planned {
                output: declaration.output,
                first,
                end: inputs.len(),
            });
        }
        order
            .try_reserve_exact(declarations.len())
            .map_err(io::Error::other)?;
        while order.len() < declarations.len() {
            let before = order.len();
            for (at, declaration) in declarations.iter().enumerate() {
                if order.contains(&at) {
                    continue;
                }
                if declaration.inputs.iter().all(|input| {
                    matches!(input.source, Source::Table(_))
                        || external
                            .iter()
                            .any(|&p| input.source == Source::Publication(p))
                        || order.iter().any(|&producer| {
                            Source::Publication(declarations[producer].output) == input.source
                        })
                }) {
                    order.push(at);
                }
            }
            if order.len() == before {
                return Err(invalid("derived-stage dependency cycle"));
            }
        }
        Ok(Self {
            order,
            nodes,
            inputs,
            reserved_bytes: bytes,
        })
    }

    pub fn indices(&self) -> &[usize] {
        &self.order
    }

    pub fn run_day<R>(
        &self,
        day: Day,
        stages: &mut [&mut dyn DerivedStage<R>],
        publications: &mut dyn Publications<R>,
    ) -> io::Result<()> {
        if stages.len() != self.order.len() {
            return Err(invalid("stage plan length changed"));
        }
        for (index, stage) in stages.iter().enumerate() {
            let actual = stage.declaration();
            let planned = &self.nodes[index];
            if actual.output != planned.output
                || !actual
                    .inputs
                    .iter()
                    .map(|input| input.source)
                    .eq(self.inputs[planned.first..planned.end].iter().copied())
            {
                return Err(invalid("stage dependencies changed after planning"));
            }
        }
        for &index in &self.order {
            let declaration = stages[index].declaration();
            if declaration
                .inputs
                .iter()
                .any(|input| !publications.ready(input.source, day))
            {
                return Err(invalid("stage input has not published this segment/day"));
            }
            let output = declaration.output;
            let changed = day.first
                || declaration
                    .inputs
                    .iter()
                    .any(|input| publications.changed(*input, day));
            if changed {
                stages[index].run(day, publications)?;
            }
            publications.finish(output, day)?;
        }
        Ok(())
    }
}
