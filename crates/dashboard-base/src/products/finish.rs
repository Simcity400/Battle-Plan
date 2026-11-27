use super::{LoadContext, ReplyWarnings};
use crate::{
    Source,
    catalogue::{self, OwnRefusals},
    presentation::Presented,
};
use content_reader_layers::content::ResolvedContent;
use std::{io, sync::Arc};

#[derive(Clone)]
#[cfg_attr(any(test, feature = "test-support"), derive(Default))]
pub struct Finish {
    changed: Option<Arc<ResolvedContent>>,
    presented: Option<Arc<Presented>>,
    opened: Arc<OwnRefusals>,
}

impl Finish {
    pub fn new(source: &Source, load: &LoadContext, content_dependent: bool) -> Self {
        Self::open(source, load, content_dependent, || ()).0
    }

    pub fn open<T>(source: &Source, load: &LoadContext, content_dependent: bool, open: impl FnOnce() -> T) -> (Self, T) {
        let mut opened = OwnRefusals::default();
        let value = catalogue::own_refusals(&mut opened, open);
        let content = load.content.as_ref();
        let finish = Self {
            changed: content.filter(|_| content_dependent).and_then(|content| source.changed_content(content)),
            presented: content.and_then(|content| source.presentation_refusals(content)),
            opened: Arc::new(opened),
        };
        (finish, value)
    }

    pub fn scope<T>(&self, own: &mut OwnRefusals, read: impl FnOnce() -> T) -> T {
        catalogue::own_refusals(own, || {
            catalogue::credit(&self.opened);
            read()
        })
    }

    pub fn reply<R: ReplyWarnings>(&self, read: impl FnOnce() -> io::Result<R>) -> io::Result<R> {
        let mut own = OwnRefusals::default();
        let mut reply = self.scope(&mut own, read).map_err(|error| dashboard_core::duplicate(&error))?;
        self.seal(reply.warnings_mut(), &own);
        Ok(reply)
    }

    pub fn seal(&self, warnings: &mut Vec<String>, own: &OwnRefusals) {
        let mut noted = OwnRefusals::clone(&self.opened);
        noted.merge(own);
        noted.diagnostics().append_to(warnings);
        if let Some(content) = &self.changed {
            content.append_changed_to(warnings);
        }
        if let Some(presented) = &self.presented {
            presented.append_to(warnings);
        }
        if let Some(line) = noted.fallback_line().filter(|line| !warnings.contains(line)) {
            warnings.push(line);
        }
    }

    pub fn refusal(error: &io::Error) -> String {
        dashboard_core::duplicate(error).to_string()
    }
}
