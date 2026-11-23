use crate::content::{Bytes, Generation};
use content_reader_image::{dds, native};
use std::io;

pub const TEXTURE_FILE_LIMIT: usize = 16 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Texture {
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub image: dds::Texture,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoTexture {
    Empty,
    NoExtension(Vec<u8>),
    Missing(Vec<String>),
    TooLarge { path: String, bytes: usize },
    Undecodable { path: String, reason: String },
    Undefined { path: String, reason: String },
    Device { path: String, reason: String },
    OverBudget { path: String, reason: String },
}

impl NoTexture {
    pub fn native_failure(&self) -> bool {
        !matches!(
            self,
            Self::Undefined { .. } | Self::Device { .. } | Self::OverBudget { .. }
        )
    }
}

impl std::fmt::Display for NoTexture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("names no texture"),
            Self::NoExtension(name) => write!(
                f,
                "names texture {:?}, which has no extension, so the texture extension resolver \
                 resolves nothing",
                String::from_utf8_lossy(name)
            ),
            Self::Missing(tried) => write!(f, "names a texture no layer holds (tried {tried:?})"),
            Self::TooLarge { path, bytes } => write!(
                f,
                "texture {path} is {bytes} bytes, past texture file loader's 16 MiB, so it does \
                 not load"
            ),
            Self::Undecodable { path, reason } => {
                write!(f, "texture {path} does not load in the game: {reason}")
            }
            Self::Undefined { path, reason } => {
                write!(
                    f,
                    "texture {path} does not determine what the game draws: {reason}"
                )
            }
            Self::Device { path, reason } => write!(
                f,
                "texture {path} draws as the recording client's graphics device decides, which \
                 is not recorded: {reason}"
            ),
            Self::OverBudget { path, reason } => {
                write!(f, "texture {path} is not decoded here: {reason}")
            }
        }
    }
}

pub fn texture_candidates(name: &[u8]) -> Vec<Vec<u8>> {
    let name: Vec<u8> = content_reader_parser::parser::c_string(name)
        .iter()
        .map(|byte| if *byte == b'\\' { b'/' } else { *byte })
        .collect();
    let Some(dot) = name.iter().rposition(|byte| *byte == b'.') else {
        return Vec::new();
    };
    let base = &name[..dot];
    vec![
        [base, b".dds"].concat(),
        [base, b".tga"].concat(),
        name.clone(),
    ]
}

fn vfs_path(candidate: &[u8]) -> Vec<u8> {
    candidate
        .split(|byte| *byte == b'/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(&b'/')
}

pub fn texture<B: AsRef<[u8]> + std::ops::Deref<Target = [u8]> + Into<Bytes>>(
    name: &[u8],
    read: &dyn Fn(&[u8]) -> io::Result<B>,
    decode: native::Decode,
) -> io::Result<Result<Texture, NoTexture>> {
    texture_within(name, read, decode, None)
}

pub fn texture_within<B: AsRef<[u8]> + std::ops::Deref<Target = [u8]> + Into<Bytes>>(
    name: &[u8],
    read: &dyn Fn(&[u8]) -> io::Result<B>,
    decode: native::Decode,
    max_texels: Option<u64>,
) -> io::Result<Result<Texture, NoTexture>> {
    let name = content_reader_parser::parser::c_string(name);
    if name.is_empty() {
        return Ok(Err(NoTexture::Empty));
    }
    let candidates = texture_candidates(name);
    if candidates.is_empty() {
        return Ok(Err(NoTexture::NoExtension(name.to_vec())));
    }
    let mut tried = Vec::with_capacity(candidates.len());
    for candidate in &candidates {
        let bytes_path = vfs_path(candidate);
        let path = String::from_utf8_lossy(&bytes_path).into_owned();
        tried.push(String::from_utf8_lossy(candidate).into_owned());
        let bytes: Bytes = match read(&bytes_path) {
            Ok(bytes) => bytes.into(),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::InvalidInput
                ) =>
            {
                continue;
            }
            Err(error) => return Err(error),
        };
        if bytes.len() > TEXTURE_FILE_LIMIT {
            return Ok(Err(NoTexture::TooLarge {
                path,
                bytes: bytes.len(),
            }));
        }
        let mut parameters = vec![
            u8::from(decode.force_pow2_textures),
            u8::from(name.len() > 3 && name[name.len() - 4..].eq_ignore_ascii_case(b".tga")),
            u8::from(max_texels.is_some()),
        ];
        parameters.extend_from_slice(&max_texels.unwrap_or_default().to_le_bytes());
        let decoded =
            Generation::derive(&bytes, "native texture decoding", 1, &parameters, || {
                Ok(match max_texels {
                    Some(max_texels) => {
                        native::decode_texture_image_within(name, &bytes, decode, max_texels)
                    }
                    None => native::decode_texture_image(name, &bytes, decode),
                })
            })?;
        return Ok(match decoded {
            Ok(image) => Ok(Texture {
                path,
                width: image.width,
                height: image.height,
                image,
            }),
            Err(native::Failure::Undecodable(reason)) => {
                Err(NoTexture::Undecodable { path, reason })
            }
            Err(native::Failure::Undefined(reason)) => Err(NoTexture::Undefined { path, reason }),
            Err(native::Failure::Device(reason)) => Err(NoTexture::Device { path, reason }),
            Err(native::Failure::OverBudget(reason)) => Err(NoTexture::OverBudget { path, reason }),
        });
    }
    Ok(Err(NoTexture::Missing(tried)))
}
