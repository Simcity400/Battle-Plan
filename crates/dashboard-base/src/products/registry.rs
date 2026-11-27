use super::{Envelope, Finish, LoadContext, Product, ReplyWarnings};
use crate::Source;
use dashboard_core::{QueryError, transport::{MAX_REPLY_BYTES, bounded_json}};
use replay_recording::columnar_frames::{Damage, DamageScope};
use std::sync::atomic::AtomicBool;

#[macro_export]
macro_rules! register_products {
    (
        $($name:ident => $product:ty $(; $kind:ident($($args:tt)*))?,)*
        @hourly
        $($hour_name:ident => exact<$hour_product:ty>,)*
    ) => {
        $($crate::register_products!(@declare $name, $product, $crate::products::register, $crate::products::register_for_export
            $(, $crate::register_products!(@ $kind $product, $($args)*))?);)*
        $($crate::register_products!(@declare $hour_name, $hour_product, $crate::products::at_hour::entry, $crate::products::at_hour::entry_for_export,
            $crate::products::at_hour::exact::<$hour_product>());)*
        pub const PRODUCTS: &[$crate::products::Registered] = &[$($name,)* $($hour_name,)*];
    };
    (@declare $name:ident, $product:ty, $register:path, $export:path $(, $panel:expr)?) => {
        pub const $name: $crate::products::Registered = {
            use $register as register;
            #[cfg(any(test, feature = "export-types"))]
            use $export as registration;
            #[cfg(not(any(test, feature = "export-types")))]
            use $register as registration;

            fn load(
                source: &$crate::Source,
                lease: &str,
                params: serde_json::Value,
                context: &$crate::products::LoadContext,
                cancelled: &std::sync::atomic::AtomicBool,
            ) -> Result<String, dashboard_core::QueryError> {
                (register::<$product>().load)(source, lease, params, context, cancelled)
            }
            $crate::products::Registered { load, $(panel: Some($panel),)? ..registration::<$product>() }
        };
    };
    (@ interval $product:ty, $date:expr, $through:expr) => {
        $crate::products::PanelReader {
            open: |source, load, asked| $crate::products::at_hour::product_interval_panel::<$product>(source, load, asked, $date, $through),
        }
    };
    (@ day $product:ty, $start:literal, $end:literal, $through:expr) => {
        $crate::products::PanelReader {
            open: |source, load, asked| $crate::products::at_hour::day_panel::<$product>(source, load, asked, $start, $end, $through),
        }
    };
    (@ panel $product:ty, $reader:path) => {
        $crate::products::PanelReader { open: |source, load, asked| $reader(source, load, asked) }
    };
}

#[derive(Clone, Copy)]
pub struct PanelReader {
    pub open: fn(&std::sync::Arc<Source>, &LoadContext, &serde_json::Value) -> Result<Box<dyn dashboard_follow::panel::Panel>, String>,
}

#[derive(Clone, Copy)]
pub struct Registered {
    pub panel: Option<PanelReader>,
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
    pub fields: super::Fields,
    pub read_hour: bool,
    #[cfg(any(test, feature = "export-types"))]
    pub types: Option<fn() -> Vec<String>>,
    #[cfg(any(test, feature = "export-types"))]
    pub params: fn() -> dashboard_query::fields::Walk,
}

pub const fn register<P: Product>() -> Registered {
    Registered {
        panel: None,
        key: P::KEY,
        content_dependent: P::CONTENT_DEPENDENT,
        interactive: P::INTERACTIVE,
        replays: P::REPLAYS,
        load: load_json::<P>,
        fields: P::FIELDS,
        read_hour: false,
        #[cfg(any(test, feature = "export-types"))]
        types: None,
        #[cfg(any(test, feature = "export-types"))]
        params: dashboard_query::fields::walk::<P::Params>,
    }
}

impl Registered {
    pub fn answer(&self, source: &Source, params: serde_json::Value, load: &LoadContext, cancelled: &AtomicBool) -> std::io::Result<serde_json::Value> {
        let text = (self.load)(source, "answer", params, load, cancelled).map_err(std::io::Error::other)?;
        let mut reply: serde_json::Value = serde_json::from_str(&text)?;
        Ok(reply.get_mut("data").map(serde_json::Value::take).unwrap_or(serde_json::Value::Null))
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
    let params = dashboard_query::named::named::<P::Params>(&request)
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
    let data = Finish::new(source, context, content_dependent).reply(load).map_err(|error| {
        match Damage::of(&error).map(|damage| &damage.scope) {
            Some(DamageScope::Withheld { .. }) => QueryError::Refused { text: error.to_string() },
            _ => QueryError::from(error),
        }
    })?;
    reply.finish(bounded_json(&data, cancelled)?)
}

struct JsonReply {
    prefix: String,
    _content: Option<std::sync::Arc<content_reader_layers::content::ResolvedContent>>,
}

impl JsonReply {
    #[inline(never)]
    fn new(
        source: &Source,
        lease_id: &str,
        key: &'static str,
        content_dependent: bool,
        context: &LoadContext,
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
        Ok(Self { prefix, _content: content })
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
