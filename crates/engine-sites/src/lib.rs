
mod fingerprint;
mod generated;
mod image;
mod locate;

pub use image::{Image, OwnedImage};
pub use locate::{Located, Summary};
pub use engine_executable::availability::{Verified, Withheld};

use std::cell::Cell;
use std::sync::atomic::{AtomicPtr, AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Rva(pub usize);

impl Rva {
    pub const fn after(self, delta: usize) -> Self {
        Self(self.0 + delta)
    }

    pub const fn before(self, delta: usize) -> Self {
        Self(self.0 - delta)
    }
}

pub const ORIGINAL: usize = 32;

const LAYOUT_CHECKED: &[[u8; 32]] = &[
    [
        0x70, 0x9f, 0x91, 0x6a, 0xfb, 0xeb, 0xcf, 0xe6, 0x4a, 0x26, 0x67, 0x87, 0x02, 0x69, 0x1a, 0xeb,
        0x40, 0x94, 0x5d, 0x20, 0x60, 0x74, 0xc2, 0xa8, 0x0d, 0xe8, 0xe9, 0xd6, 0x01, 0xfc, 0xe1, 0x48,
    ],
    [
        0xbc, 0xcc, 0x62, 0x90, 0x9b, 0x6a, 0x1e, 0xc2, 0xc1, 0xd5, 0xa4, 0x6e, 0xf1, 0x86, 0x08, 0xe4,
        0xd0, 0xc7, 0xcd, 0xc8, 0x9a, 0xd3, 0x41, 0xb7, 0x40, 0x2a, 0x23, 0xc4, 0x0f, 0x7a, 0x80, 0x37,
    ],
];

pub fn layout_checked(sha256: &[u8; 32]) -> bool {
    LAYOUT_CHECKED.contains(sha256)
}

const SYNTHETIC: u8 = 0;
const PENDING: u8 = 1;
const INSTALLED: u8 = 2;
static MODE: AtomicU8 = AtomicU8::new(SYNTHETIC);
static LOCATED: AtomicPtr<Located> = AtomicPtr::new(std::ptr::null_mut());

pub fn production() {
    let _ = MODE.compare_exchange(SYNTHETIC, PENDING, Ordering::AcqRel, Ordering::Acquire);
}

pub fn install(located: Located) -> &'static Located {
    let leaked: &'static mut Located = Box::leak(Box::new(located));
    LOCATED.store(leaked, Ordering::Release);
    MODE.store(INSTALLED, Ordering::Release);
    leaked
}

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

#[inline]
pub fn located() -> Option<&'static Located> {
    // SAFETY: only `install` stores, a leaked and never freed table.
    unsafe { LOCATED.load(Ordering::Acquire).as_ref() }
}

#[inline]
pub fn resolve(base: usize, key: Rva) -> Verified<usize> {
    match MODE.load(Ordering::Acquire) {
        SYNTHETIC => {
            if SYNTHETIC_WITHHELD.try_with(Cell::get).ok().flatten() == Some(key) {
                return Err(Withheld::new(name(key), "withheld in synthetic test"));
            }
            base.checked_add(key.0)
                .ok_or_else(|| Withheld::new(name(key), "synthetic address overflows"))
        }
        INSTALLED => located()
            .ok_or_else(|| Withheld::new(name(key), "no located image"))?
            .resolve(base, key),
        _ => Err(Withheld::new(name(key), "the executable has not been located yet")),
    }
}

#[inline]
pub fn at(base: usize, key: Rva) -> Option<usize> {
    match resolve(base, key) {
        Ok(address) => Some(address),
        Err(_) => {
            taint(key);
            None
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Code {
    pub address: usize,
    pub function: usize,
    pub end: usize,
    original: [u8; ORIGINAL],
    verified: usize,
}

impl Code {
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
    pub fn sentinel<const N: usize>(&self) -> Result<[u8; N], &'static str> {
        self.original::<N>()
            .map_err(|_| "site withheld: its function verifies fewer bytes than the hook replaces")
    }
}

pub fn code(base: usize, key: Rva) -> Verified<Code> {
    match MODE.load(Ordering::Acquire) {
        INSTALLED => located()
            .ok_or_else(|| Withheld::new(name(key), "no located image"))?
            .code(base, key),
        _ => Err(Withheld::new(name(key), "the executable has not been located yet")),
    }
}

pub fn hook(base: usize, key: Rva) -> Result<Code, &'static str> {
    code(base, key).map_err(|_| match located() {
        Some(l) => l.message(key).unwrap_or("site withheld: not a located code site"),
        None => "site withheld: the executable has not been located yet",
    })
}

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

#[inline]
pub fn reference(base: usize, address: usize) -> Option<Rva> {
    match MODE.load(Ordering::Acquire) {
        SYNTHETIC => address.checked_sub(base).map(Rva),
        INSTALLED => located()?.reference(base, address),
        _ => None,
    }
}

#[inline]
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
    if found.is_some() {
        return Ok(None);
    }
    for &candidate in candidates {
        resolve(base, candidate)?;
    }
    Ok(None)
}

#[inline]
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

pub fn name(key: Rva) -> std::borrow::Cow<'static, str> {
    match locate::key(key) {
        Some(entry) => entry.name.into(),
        None => format!("site {:#x}", key.0).into(),
    }
}

thread_local! {
    static TAINT: Cell<Option<Rva>> = const { Cell::new(None) };
    static SYNTHETIC_WITHHELD: Cell<Option<Rva>> = const { Cell::new(None) };
}

pub fn with_synthetic_withheld<R>(key: Rva, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<Rva>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = SYNTHETIC_WITHHELD.try_with(|cell| cell.set(self.0));
        }
    }
    let previous = SYNTHETIC_WITHHELD.with(|cell| cell.replace(Some(key)));
    let _restore = Restore(previous);
    f()
}

fn taint(key: Rva) {
    let _ = TAINT.try_with(|cell| {
        if cell.get().is_none() {
            cell.set(Some(key));
        }
    });
}

pub fn take_taint() -> Option<Rva> {
    TAINT.try_with(Cell::take).ok().flatten()
}

pub fn withheld(key: Rva) -> Withheld {
    located()
        .and_then(|l| l.withheld_key(key))
        .unwrap_or_else(|| Withheld::new(name(key), "the executable has not been located"))
}
