
use std::{any::Any, sync::OnceLock};

pub type Enter = Box<dyn Fn(&'static str) -> Box<dyn Any> + Send + Sync>;
static CAPTURE: OnceLock<fn() -> Option<Enter>> = OnceLock::new();

pub fn install(capture: fn() -> Option<Enter>) {
    CAPTURE.get_or_init(|| capture);
}

pub struct Context(Option<Enter>);
impl Context {
    pub fn capture() -> Self {
        Self(CAPTURE.get().and_then(|capture| capture()))
    }
    pub fn enter(&self, category: &'static str) -> Scope {
        Scope {
            _owner: self.0.as_ref().map(|enter| enter(category)),
        }
    }
}

#[must_use]
pub struct Scope {
    _owner: Option<Box<dyn Any>>,
}
