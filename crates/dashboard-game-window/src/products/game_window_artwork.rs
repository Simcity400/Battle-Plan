//! `game-window-artwork`: the texture pages a game window samples, at their native size, by
//! their ids in the window's artwork session. Pages are sent whole and unscaled: the window
//! draws every frame, tile and glyph from them with the coordinates its quads carry.

use std::{
    io,
    sync::{Arc, atomic::AtomicBool},
};

use dashboard_base::{
    Source,
    catalogue::{self, png_data_url},
    metadata::check_cancel,
    products::{LoadContext, Product, check_reply_budget},
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{invalid, session};

pub struct GameWindowArtwork;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GameWindowArtworkParams {
    /// [`super::game_window::GameWindowReply::session`].
    pub session: String,
    pub textures: Vec<u32>,
}

/// One texture page, or why it is unavailable.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WindowTextureImage {
    pub id: u32,
    pub width: u32,
    pub height: u32,
    /// A PNG data URL of the page's top level, straight alpha.
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct GameWindowArtworkReply {
    pub warnings: Vec<String>,
    pub items: Vec<WindowTextureImage>,
}

impl Product for GameWindowArtwork {
    const KEY: &'static str = "game-window-artwork";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = GameWindowArtworkParams;
    type Reply = GameWindowArtworkReply;

    fn types() -> Vec<String> {
        vec![
            GameWindowArtworkParams::decl(),
            WindowTextureImage::decl(),
            GameWindowArtworkReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.session.is_empty() || params.textures.is_empty() {
            return Err("Invalid game window artwork request");
        }
        Ok(())
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        // The recording's content is verified as for any content product.
        catalogue::catalogue(source, context.content()?)?;
        let shared = session::current(&source.state::<session::Sessions>(), &params.session)
            .ok_or_else(|| {
                invalid("This game window's artwork session was replaced; open the window again")
            })?;
        let mut items = Vec::with_capacity(params.textures.len());
        for &id in &params.textures {
            check_cancel(cancelled)?;
            // Hold the session only to copy the page, not while encoding it.
            let (page, encoded) = {
                let session = shared.lock().unwrap_or_else(|error| error.into_inner());
                let encoded = session.encoded(id as usize);
                let page = session.images.get(id as usize);
                (
                    page.map(|page| {
                        (
                            [page.width, page.height],
                            encoded.is_none().then(|| page.clone()),
                        )
                    }),
                    encoded,
                )
            };
            let Some(([width, height], page)) = page else {
                return Err(invalid(format!(
                    "Game window texture {id} is not part of this artwork session"
                )));
            };
            let (image, unavailable) = match (encoded, page) {
                (Some(image), _) => (Some(image.to_string()), None),
                (None, Some(page)) => match installed_content_image::rgba::dds_base(page)
                    .and_then(|image| png_data_url(&image))
                {
                    Ok(image) => {
                        shared
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .keep_encoded(id as usize, Arc::from(image.as_str()));
                        (Some(image), None)
                    }
                    Err(why) => (None, Some(why.to_string())),
                },
                (None, None) => unreachable!("a page is copied when it is not encoded"),
            };
            items.push(WindowTextureImage {
                id,
                width,
                height,
                image,
                unavailable,
            });
        }
        let reply = GameWindowArtworkReply {
            warnings: Vec::new(),
            items,
        };
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}
