use super::{Envelope, LoadContext, Product, ReplyWarnings};
use crate::Source;
use dashboard_core::transport::{MAX_REPLY_BYTES, bounded_json};
use serde::Deserialize;
use std::sync::atomic::AtomicBool;

#[macro_export]
macro_rules! register_products {
    ($($name:ident => $product:ty),+ $(,)?) => {
        $(pub const $name: $crate::products::Registered = {
            fn load(
                source: &$crate::Source,
                lease: &str,
                params: serde_json::Value,
                context: &$crate::products::LoadContext,
                cancelled: &std::sync::atomic::AtomicBool,
            ) -> Result<String, dashboard_core::QueryError> {
                ($crate::products::register::<$product>().load)(source, lease, params, context, cancelled)
            }
            #[cfg(any(test, feature = "export-types"))]
            use $crate::products::register_for_export as registration;
            #[cfg(not(any(test, feature = "export-types")))]
            use $crate::products::register as registration;
            $crate::products::Registered { load, ..registration::<$product>() }
        };)+
    };
}

#[macro_export]
macro_rules! register_hourly_products {
    ($($name:ident => $product:ty),+ $(,)?) => {
        $(pub const $name: $crate::products::at_hour::Entry = {
            fn load(
                source: &$crate::Source,
                lease: &str,
                params: serde_json::Value,
                context: &$crate::products::LoadContext,
                cancelled: &std::sync::atomic::AtomicBool,
            ) -> Result<String, dashboard_core::QueryError> {
                ($crate::products::at_hour::entry::<$product>().registered.load)(source, lease, params, context, cancelled)
            }
            fn panel(
                source: &$crate::Source,
                context: &$crate::products::LoadContext,
                params: &serde_json::Value,
            ) -> Result<Box<dyn dashboard_follow::panel::Panel>, String> {
                ($crate::products::at_hour::entry::<$product>().panel)(source, context, params)
            }
            #[cfg(any(test, feature = "export-types"))]
            use $crate::products::at_hour::entry_for_export as registration;
            #[cfg(not(any(test, feature = "export-types")))]
            use $crate::products::at_hour::entry as registration;
            $crate::products::at_hour::Entry {
                registered: $crate::products::Registered {
                    load,
                    ..registration::<$product>().registered
                },
                panel,
            }
        };)+
    };
}

#[derive(Clone, Copy)]
pub struct Registered {
    pub key: &'static str,
    pub content_dependent: bool,
    pub interactive: bool,
    pub replays: bool,
    pub load: fn(
        &Source,
        &str,
        serde_json::Value,
        &LoadContext,
        &AtomicBool,
    ) -> Result<String, dashboard_core::QueryError>,
    #[cfg(any(test, feature = "export-types"))]
    pub types: Option<fn() -> Vec<String>>,
}

pub const fn register<P: Product>() -> Registered {
    Registered {
        key: P::KEY,
        content_dependent: P::CONTENT_DEPENDENT,
        interactive: P::INTERACTIVE,
        replays: P::REPLAYS,
        load: load_json::<P>,
        #[cfg(any(test, feature = "export-types"))]
        types: None,
    }
}

fn load_json<P: Product>(
    source: &Source,
    lease_id: &str,
    params: serde_json::Value,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> Result<String, dashboard_core::QueryError> {
    let request = params;
    let params = P::Params::deserialize(&request)
        .map_err(|error| format!("Invalid {} request: {error}", P::KEY))?;
    P::validate(&params).map_err(str::to_owned)?;
    reply_json(
        source,
        lease_id,
        P::KEY,
        P::CONTENT_DEPENDENT,
        context,
        cancelled,
        || P::load(source, &params, context, cancelled),
    )
}

pub fn reply_json<R: serde::Serialize + ReplyWarnings>(
    source: &Source,
    lease_id: &str,
    key: &'static str,
    content_dependent: bool,
    context: &LoadContext,
    cancelled: &AtomicBool,
    load: impl FnOnce() -> std::io::Result<R>,
) -> Result<String, dashboard_core::QueryError> {
    let mut reply = JsonReply::new(source, lease_id, key, content_dependent, context, cancelled)?;
    let mut own = crate::catalogue::OwnRefusals::default();
    let mut data =
        crate::catalogue::own_refusals(&mut own, load).map_err(dashboard_core::QueryError::from)?;
    reply.append_warnings(data.warnings_mut(), &own);
    reply.finish(bounded_json(&data, cancelled)?)
}

struct JsonReply<'a> {
    source: &'a Source,
    context: &'a LoadContext,
    content_dependent: bool,
    prefix: String,
    _content: Option<std::sync::Arc<content_reader_layers::content::ResolvedContent>>,
}

impl<'a> JsonReply<'a> {
    #[inline(never)]
    fn new(
        source: &'a Source,
        lease_id: &str,
        key: &'static str,
        content_dependent: bool,
        context: &'a LoadContext,
        cancelled: &AtomicBool,
    ) -> Result<Self, dashboard_core::QueryError> {
        crate::metadata::check_cancel(cancelled).map_err(dashboard_core::QueryError::from)?;
        let content = context
            .content
            .as_ref()
            .filter(|_| content_dependent)
            .and_then(|content| source.content(&content.options).ok());
        let mut prefix = bounded_json(
            &Envelope {
                source_id: lease_id,
                revision: &source.key().revision,
                product: key,
                data: (),
            },
            cancelled,
        )?;
        prefix.truncate(prefix.len() - "null}".len());
        Ok(Self {
            source,
            context,
            content_dependent,
            prefix,
            _content: content,
        })
    }

    #[inline(never)]
    fn append_warnings(&self, warnings: &mut Vec<String>, own: &crate::catalogue::OwnRefusals) {
        own.diagnostics().append_to(warnings);
        if self.content_dependent {
            self.source
                .append_changed_warnings(warnings, self.context.content.as_ref());
        }
        self.source
            .append_presentation_warnings(warnings, self.context.content.as_ref());
        warnings.extend(own.fallback_line());
    }

    #[inline(never)]
    fn finish(&mut self, data: String) -> Result<String, dashboard_core::QueryError> {
        if self.prefix.len() + data.len() + 1 > MAX_REPLY_BYTES {
            return Err("Projected response exceeds the transport byte budget".into());
        }
        self.prefix.reserve_exact(data.len() + 1);
        self.prefix.push_str(&data);
        self.prefix.push('}');
        Ok(std::mem::take(&mut self.prefix))
    }
}

#[cfg(any(test, feature = "export-types"))]
pub const fn register_for_export<P: Product>() -> Registered
where
    P::Params: ts_rs::TS,
    P::Reply: ts_rs::TS,
{
    Registered { types: Some(P::types), ..register::<P>() }
}
