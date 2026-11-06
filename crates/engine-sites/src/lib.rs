//! Game sites located by content. Code names each hook site, call site, vtable and global by its
//! position in the reference build, an [`Rva`] key. At attach the collector locates every key in
//! the running executable ([`Located::locate`]) and from then on resolves keys only through that
//! result: a key whose code, vtable or global could not be verified is withheld by name, never
//! read at a fixed offset. `tools/sites.py` generates the locator tables from the RE index and
//! checks them against every known build.

mod fingerprint;
mod generated;
mod image;
mod locate;

pub use image::{Image, OwnedImage};
pub use locate::{Located, Summary};
pub use engine_executable::availability::{Verified, Withheld};

use std::cell::Cell;
use std::sync::atomic::{AtomicPtr, AtomicU8, Ordering};

/// A site's position in the reference build. It is a key the locator resolves, not an address;
/// without a located image (tests) it is also the site's synthetic offset from a synthetic base.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Rva(pub usize);

impl Rva {
    /// The position `delta` bytes further into the same function, such as a call's return address.
    pub const fn after(self, delta: usize) -> Self {
        Self(self.0 + delta)
    }

    /// The position `delta` bytes earlier in the same function.
    pub const fn before(self, delta: usize) -> Self {
        Self(self.0 - delta)
    }
}

/// Bytes of verified code copied at each code key when the image is located.
pub const ORIGINAL: usize = 32;

/// Executables whose struct layouts were checked against this source: 1.19.4.2.
const LAYOUT_CHECKED: &[[u8; 32]] = &[[
    0xbc, 0xcc, 0x62, 0x90, 0x9b, 0x6a, 0x1e, 0xc2, 0xc1, 0xd5, 0xa4, 0x6e, 0xf1, 0x86, 0x08, 0xe4,
    0xd0, 0xc7, 0xcd, 0xc8, 0x9a, 0xd3, 0x41, 0xb7, 0x40, 0x2a, 0x23, 0xc4, 0x0f, 0x7a, 0x80, 0x37,
]];

/// Whether this source's struct layouts were checked against the executable with this SHA-256.
/// Any other build has to verify its layouts through constructor and vtable-shape witnesses.
pub fn layout_checked(sha256: &[u8; 32]) -> bool {
    LAYOUT_CHECKED.contains(sha256)
}

const SYNTHETIC: u8 = 0;
const PENDING: u8 = 1;
const INSTALLED: u8 = 2;
static MODE: AtomicU8 = AtomicU8::new(SYNTHETIC);
static LOCATED: AtomicPtr<Located> = AtomicPtr::new(std::ptr::null_mut());

/// Called first by every process that reads the game: from now on keys resolve only through an
/// installed [`Located`], never as synthetic offsets.
pub fn production() {
    let _ = MODE.compare_exchange(SYNTHETIC, PENDING, Ordering::AcqRel, Ordering::Acquire);
}

/// Publish the located image for every later lookup. A replaced table stays allocated, because
/// lookups on other threads may still hold it.
pub fn install(located: Located) -> &'static Located {
    let leaked: &'static mut Located = Box::leak(Box::new(located));
    LOCATED.store(leaked, Ordering::Release);
    MODE.store(INSTALLED, Ordering::Release);
    leaked
}

/// Locate the executable `file` for the process that loaded it at `base`, such as another
/// process's game, and install it; a table already installed for the same file and base is reused.
pub fn install_file(file: &[u8], sha256: &[u8; 32], base: usize) -> Verified<&'static Located> {
    production();
    if let Some(located) = located()
        && located.base() == base
        && located.identity() == Some(sha256)
    {
        return Ok(located);
    }
    let owned = OwnedImage::new(file)?;
    let mut located = Located::locate(&owned.image(base), layout_checked(sha256));
    located.set_identity(*sha256);
    Ok(install(located))
}

/// The installed table, if any.
pub fn located() -> Option<&'static Located> {
    // SAFETY: only `install` stores, a leaked and never freed table.
    unsafe { LOCATED.load(Ordering::Acquire).as_ref() }
}

/// The live address of a key, or why it is withheld.
pub fn resolve(base: usize, key: Rva) -> Verified<usize> {
    match MODE.load(Ordering::Acquire) {
        SYNTHETIC => base
            .checked_add(key.0)
            .ok_or_else(|| Withheld::new(name(key), "synthetic address overflows")),
        INSTALLED => located()
            .ok_or_else(|| Withheld::new(name(key), "no located image"))?
            .resolve(base, key),
        _ => Err(Withheld::new(name(key), "the executable has not been located yet")),
    }
}

/// The live address of a key; a withheld key taints the reading thread (see [`take_taint`]) and
/// reads as absent. Readers use this where they would have added a fixed offset to the image base.
pub fn at(base: usize, key: Rva) -> Option<usize> {
    match resolve(base, key) {
        Ok(address) => Some(address),
        Err(_) => {
            taint(key);
            None
        }
    }
}

/// A located code key: its address, its function's start and the verified bytes there.
#[derive(Clone, Copy, Debug)]
pub struct Code {
    pub address: usize,
    pub function: usize,
    original: [u8; ORIGINAL],
    verified: usize,
}

impl Code {
    /// The first `N` bytes at the site, copied when the image was located. They are the hook's
    /// expected bytes: the verified code with its relocated operands as this build has them.
    pub fn original<const N: usize>(&self) -> Verified<[u8; N]> {
        if N > self.verified {
            return Err(Withheld::new(
                "site bytes",
                format!("{N} bytes requested, {} verified", self.verified),
            ));
        }
        let mut out = [0; N];
        out.copy_from_slice(&self.original[..N]);
        Ok(out)
    }
    /// [`Self::original`] for hook constructors whose failures are static text.
    pub fn sentinel<const N: usize>(&self) -> Result<[u8; N], &'static str> {
        self.original::<N>()
            .map_err(|_| "site withheld: its function verifies fewer bytes than the hook replaces")
    }
}

/// A located code key with its verified bytes, for hook construction.
pub fn code(base: usize, key: Rva) -> Verified<Code> {
    match MODE.load(Ordering::Acquire) {
        INSTALLED => located()
            .ok_or_else(|| Withheld::new(name(key), "no located image"))?
            .code(base, key),
        _ => Err(Withheld::new(name(key), "the executable has not been located yet")),
    }
}

/// [`code`] for hook constructors whose failures are static text: the cause names the site and
/// why it is withheld.
pub fn hook(base: usize, key: Rva) -> Result<Code, &'static str> {
    code(base, key).map_err(|_| match located() {
        Some(l) => l.message(key).unwrap_or("site withheld: not a located code site"),
        None => "site withheld: the executable has not been located yet",
    })
}

/// Every key resolves, or the reason the first withheld one is. Constructors of families whose
/// callbacks classify addresses against these keys call it, so a withheld candidate refuses the
/// family by name instead of classifying as unknown.
pub fn require(base: usize, keys: &[Rva]) -> Result<(), &'static str> {
    for &key in keys {
        if resolve(base, key).is_err() {
            return Err(match located() {
                Some(l) => l.message(key).unwrap_or("site withheld: a classification site is not located"),
                None => "site withheld: the executable has not been located yet",
            });
        }
    }
    Ok(())
}

/// The key of a live address: a position inside a located function, or a located vtable or global.
pub fn reference(base: usize, address: usize) -> Option<Rva> {
    match MODE.load(Ordering::Acquire) {
        SYNTHETIC => address.checked_sub(base).map(Rva),
        INSTALLED => located()?.reference(base, address),
        _ => None,
    }
}

/// Which candidate key a live address is. `Ok(None)` only when the address is none of them and
/// every candidate is located; otherwise the candidate that could not be ruled out is withheld.
pub fn classify(base: usize, address: usize, candidates: &[Rva]) -> Verified<Option<Rva>> {
    let found = reference(base, address);
    if let Some(key) = found
        && candidates.contains(&key)
    {
        return Ok(Some(key));
    }
    if MODE.load(Ordering::Acquire) == SYNTHETIC {
        return Ok(None);
    }
    // An address inside a located function cannot be a key of an unlocated one.
    if found.is_some() {
        return Ok(None);
    }
    for &candidate in candidates {
        resolve(base, candidate)?;
    }
    Ok(None)
}

/// [`classify`] for readers: an undecidable address taints the current scope and reads as none.
pub fn which(base: usize, address: usize, candidates: &[Rva]) -> Option<Rva> {
    match classify(base, address, candidates) {
        Ok(found) => found,
        Err(_) => {
            for &candidate in candidates {
                if resolve(base, candidate).is_err() {
                    taint(candidate);
                    break;
                }
            }
            None
        }
    }
}

/// The name generated for a key, or its reference position.
pub fn name(key: Rva) -> std::borrow::Cow<'static, str> {
    match locate::key(key) {
        Some(entry) => entry.name.into(),
        None => format!("site {:#x}", key.0).into(),
    }
}

thread_local! {
    static TAINT: Cell<Option<Rva>> = const { Cell::new(None) };
}

fn taint(key: Rva) {
    let _ = TAINT.try_with(|cell| {
        if cell.get().is_none() {
            cell.set(Some(key));
        }
    });
}

/// The first key a read withheld on this thread since the last take, if any. The collector's
/// read accounting takes it at every table switch and withholds that table by the key's name.
pub fn take_taint() -> Option<Rva> {
    TAINT.try_with(Cell::take).ok().flatten()
}

/// Why a key is withheld, for messages built outside capture paths.
pub fn withheld(key: Rva) -> Withheld {
    located()
        .and_then(|l| l.withheld_key(key))
        .unwrap_or_else(|| Withheld::new(name(key), "the executable has not been located"))
}
