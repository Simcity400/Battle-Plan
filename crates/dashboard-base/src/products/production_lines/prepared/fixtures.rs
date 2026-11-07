use super::*;

#[derive(Default)]
pub struct FixturePages {
    pub pages: BTreeMap<(String, u64), Vec<u8>>,
}
impl FixturePages {
    // Ranges are written consecutively, so the section's last range + 1 is its page count.
    fn next(&self, section: &str) -> u64 {
        self.pages
            .range((section.to_owned(), 0)..=(section.to_owned(), u64::MAX))
            .next_back()
            .map_or(0, |((_, range), _)| range + 1)
    }
}
impl Pages for FixturePages {
    fn write_at(&mut self, section: &str, range: u64, bytes: &[u8]) -> io::Result<()> {
        if range > self.next(section) {
            return Err(crate::invalid("Non-consecutive fixture page range"));
        }
        self.pages.insert((section.into(), range), bytes.to_vec());
        Ok(())
    }
    fn write(&mut self, section: &str, bytes: &[u8]) -> io::Result<u64> {
        let range = self.next(section);
        self.write_at(section, range, bytes)?;
        Ok(range)
    }
    fn read_at(&mut self, section: &str, range: u64) -> io::Result<Option<Vec<u8>>> {
        Ok(self.pages.get(&(section.into(), range)).cloned())
    }
}

/// The fixture pages a source holds, read as a query reads its store.
struct Held(Arc<std::sync::Mutex<FixturePages>>);
impl Pages for Held {
    fn write_at(&mut self, _: &str, _: u64, _: &[u8]) -> io::Result<()> {
        Err(crate::invalid("Production query cannot write pages"))
    }
    fn write(&mut self, _: &str, _: &[u8]) -> io::Result<u64> {
        Err(crate::invalid("Production query cannot write pages"))
    }
    fn read_at(&mut self, section: &str, range: u64) -> io::Result<Option<Vec<u8>>> {
        self.0.lock().unwrap().read_at(section, range)
    }
}

pub(super) fn read_directory(bytes: &[u8]) -> io::Result<Directory> {
    serde_json::from_slice(&crate::prepared::series::decompress(bytes)?).map_err(io::Error::other)
}

pub fn prepare(source: &Source, context: &LoadContext) -> io::Result<()> {
    let icons = preparation_icons(source, context);
    let replay = source.replay_ref();
    let tags: Vec<_> = source
        .metadata()
        .unwrap()
        .countries
        .iter()
        .map(|country| country.tag.clone())
        .collect();
    let mut pages = FixturePages::default();
    for branch in replay
        .index
        .iter()
        .map(|block| block.branch)
        .collect::<BTreeSet<_>>()
    {
        let mut prepared = Prepared::new(replay, branch, tags.clone(), icons.as_deref());
        for (position, block) in replay
            .index
            .iter()
            .enumerate()
            .filter(|(_, block)| block.branch == branch)
        {
            prepared.advance_block(
                &mut pages,
                replay,
                position,
                icons.as_deref(),
                &AtomicBool::new(false),
            )?;
            prepared.publish(&mut pages, replay, block, &AtomicBool::new(false))?;
        }
    }
    *source.state::<FixturePages>().lock().unwrap() = pages;
    Ok(())
}

/// A tag's whole scan (or, with `holder`, the native country's) as the index serves it
/// once `blocks` are prepared.
pub fn scan(
    replay: &Replay,
    blocks: &[&BlockIndex],
    tag: &str,
    icons: Option<&IconContent>,
    holder: Option<(u32, u32)>,
    cancelled: &AtomicBool,
    page_each_block: bool,
) -> io::Result<ScannedLines> {
    crate::metadata::check_cancel(cancelled)?;
    let Some(first) = blocks.first() else {
        return Ok(ScannedLines::default());
    };
    let mut prepared = Prepared::new(replay, first.branch, [tag.to_owned()], icons);
    let mut pages = FixturePages::default();
    for (at, block) in blocks.iter().enumerate() {
        prepared.advance_block(
            &mut pages,
            replay,
            replay.index.iter().position(|held| held == *block).unwrap(),
            icons,
            cancelled,
        )?;
        if page_each_block || at + 1 == blocks.len() {
            prepared.publish(&mut pages, replay, block, cancelled)?;
        }
    }
    let mut index = read::Index::open(Box::new(pages), prepared.directory.clone())?;
    let history = match holder {
        None => index.tag_history(tag),
        Some((slot, id)) => index
            .identity()
            .natives
            .iter()
            .find(|(country, _)| country.slot == slot && country.id == id)
            .map(|(_, history)| *history)
            .ok_or_else(|| crate::invalid("Missing fixture country"))?,
    };
    let localisation = icons
        .and_then(|icons| icons.catalogue.as_ref())
        .filter(|_| prepared.directory.localised)
        .map(|catalogue| Arc::clone(&catalogue.localisation));
    index.scanned(history, localisation, recorded_language(replay), cancelled)
}

/// The fixture index a source holds for `branch`, if one was prepared into its state.
pub(super) fn open(source: &Source, branch: u64) -> io::Result<Option<read::Index<'static>>> {
    let state = source.state::<FixturePages>();
    let directory = {
        let mut pages = state.lock().unwrap();
        match pages.read_at(&directory_section(branch), 0)? {
            Some(bytes) => read_directory(&bytes)?,
            None => return Ok(None),
        }
    };
    read::Index::open(Box::new(Held(state)), directory).map(Some)
}
