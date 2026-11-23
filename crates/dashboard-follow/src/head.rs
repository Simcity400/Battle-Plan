
use crate::answer::{self, State, Table};
use replay_data::Hour;
use serde_json::Value;
use std::sync::{Arc, Mutex, PoisonError};

pub trait HeadValue: Send {
    fn columns(&self) -> usize;
    fn answer(&mut self, hour: Hour, reset: bool, rows: &mut HeadRows<'_>) -> Result<(), String>;
}

pub struct HeadRows<'a>(&'a mut Table);

impl HeadRows<'_> {
    pub fn row(&mut self, key: u128, name: &str) {
        self.0.row(key);
        self.0.name(name);
    }

    pub fn number(&mut self, value: Option<i64>) {
        match value {
            Some(value) => self.0.cell(value, State::Value),
            None => self.0.cell(0, State::Missing),
        }
    }

    pub fn float(&mut self, value: Option<f64>) {
        self.number(value.map(|value| value.to_bits() as i64));
    }

    pub fn flag(&mut self, value: bool) {
        self.number(Some(i64::from(value)));
    }

    pub fn text(&mut self, value: Option<&str>) {
        match value {
            Some(value) => {
                let at = self.0.text(value);
                self.0.cell(i64::from(at), State::Value);
            }
            None => self.0.cell(0, State::Missing),
        }
    }
}

#[derive(Clone)]
pub struct Head {
    value: Arc<Mutex<Box<dyn HeadValue>>>,
}

impl Head {
    pub fn new(value: impl HeadValue + 'static) -> Self {
        Self { value: Arc::new(Mutex::new(Box::new(value))) }
    }
}

pub type HeadOf<'a> = dyn FnOnce(&Value) -> Option<Result<Head, String>> + 'a;

pub(crate) const TABLE: usize = usize::MAX;

pub(crate) fn write(head: &Head, hour: Hour, reset: bool, rows: &mut Table, out: &mut Vec<u8>) {
    let mut value = head.value.lock().unwrap_or_else(PoisonError::into_inner);
    rows.clear(value.columns(), false);
    match value.answer(hour, reset, &mut HeadRows(rows)) {
        Ok(()) => rows.encode(out),
        Err(reason) => answer::not_recorded(&reason, out),
    }
}
