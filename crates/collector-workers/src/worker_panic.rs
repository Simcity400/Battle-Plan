use std::{
    fmt::{self, Write as _},
    io::Write as _,
    panic::PanicHookInfo,
    path::Path,
    sync::{Arc, OnceLock},
    time::Instant,
};

struct Text<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> Text<N> {
    const fn new() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
        }
    }

    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
}

impl<const N: usize> fmt::Write for Text<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let mut take = s.len().min(N - self.len);
        while !s.is_char_boundary(take) {
            take -= 1;
        }
        self.bytes[self.len..self.len + take].copy_from_slice(&s.as_bytes()[..take]);
        self.len += take;
        if take == s.len() {
            Ok(())
        } else {
            Err(fmt::Error)
        }
    }
}

static FIRST: OnceLock<Text<512>> = OnceLock::new();

pub fn first() -> Option<&'static str> {
    FIRST.get().map(Text::as_str)
}

type Hook = Box<dyn Fn(&PanicHookInfo<'_>) + Send + Sync + 'static>;

pub struct Guard(Option<Arc<Hook>>);

impl Guard {
    /// # Safety
    /// Only attach may install the process-wide panic hook for its workers, or an isolated test may
    /// install it with exclusive use of the process hook.
    pub unsafe fn install(progress: &Path, opened: Instant) -> Self {
        let file = std::fs::OpenOptions::new().append(true).open(progress).ok();
        let previous = Arc::new(std::panic::take_hook());
        let chained = Arc::clone(&previous);
        std::panic::set_hook(Box::new(move |info| {
            let message = info
                .payload()
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
                .unwrap_or("non-string panic payload");
            let mut text = Text::<512>::new();
            let thread = std::thread::current();
            let name = thread.name().unwrap_or("unnamed thread");
            let _ = match info.location() {
                Some(at) => write!(
                    text,
                    "{name} panicked at {}:{}:{}: {message}",
                    at.file(),
                    at.line(),
                    at.column()
                ),
                None => write!(text, "{name} panicked: {message}"),
            };
            if let Some(mut file) = file.as_ref() {
                let mut line = Text::<600>::new();
                let _ = writeln!(
                    line,
                    "{} worker_panic {}",
                    opened.elapsed().as_millis(),
                    text.as_str()
                );
                let _ = file.write_all(line.as_str().as_bytes());
            }
            let _ = FIRST.set(text);
            chained(info);
        }));
        Self(Some(previous))
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        drop(std::panic::take_hook());
        if let Some(previous) = self.0.take()
            && let Ok(previous) = Arc::try_unwrap(previous)
        {
            std::panic::set_hook(previous);
        }
    }
}
