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
            prepared.publish(
                &mut pages,
                replay,
                block,
                icons.as_deref(),
                &AtomicBool::new(false),
            )?;
        }
    }
    *source.state::<FixturePages>().lock().unwrap() = pages;
    Ok(())
}

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
            replay.block_position(block).unwrap(),
            icons,
            cancelled,
        )?;
        if page_each_block || at + 1 == blocks.len() {
            prepared.publish(&mut pages, replay, block, icons, cancelled)?;
        }
    }
    let selected = match holder {
        None => tag_key(tag),
        Some((slot, id)) => {
            let mut found = None;
            for range in 0..prepared.directory.pages {
                let bytes = pages
                    .read_at(&alias_section(first.branch), range)?
                    .ok_or_else(|| crate::invalid("Missing fixture aliases"))?;
                let aliases: Vec<Alias> = pages::read(&bytes)?;
                if let Some(alias) = aliases
                    .into_iter()
                    .find(|alias| alias.country.slot == slot && alias.country.id == id)
                {
                    found = Some(alias.country);
                    break;
                }
            }
            native_key(&found.ok_or_else(|| crate::invalid("Missing fixture country"))?)
        }
    };
    assemble(
        &prepared.directory,
        &selected,
        icons,
        recorded_language(replay),
        cancelled,
        |section, range| pages.read_at(section, range),
    )
}

pub(super) fn query(
    source: &Source,
    branch: u64,
    tag: &str,
    holder: Option<NativeCountry>,
    icons: Option<&IconContent>,
    cancelled: &AtomicBool,
) -> io::Result<Option<(ScannedLines, crate::prepared::coverage::Coverage)>> {
    let state = source.state::<FixturePages>();
    let mut pages = state.lock().unwrap();
    let Some(bytes) = pages.read_at(&directory_section(branch), 0)? else {
        return Ok(None);
    };
    let directory: Directory = pages::read_json(&bytes)?;
    let selected = match holder {
        None => tag_key(tag),
        Some(country) => native_key(&country),
    };
    let result = assemble(
        &directory,
        &selected,
        icons,
        recorded_language(source.replay_ref()),
        cancelled,
        |section, range| pages.read_at(section, range),
    )?;
    Ok(Some((result, directory.coverage)))
}

pub(super) fn country_at(
    source: &Source,
    branch: u64,
    tag: &str,
    hour: i32,
) -> io::Result<Option<NativeCountry>> {
    let state = source.state::<FixturePages>();
    let mut pages = state.lock().unwrap();
    let Some(bytes) = pages.read_at(&directory_section(branch), 0)? else {
        return Ok(None);
    };
    let directory: Directory = pages::read_json(&bytes)?;
    read_country_at(&directory, tag, hour, |section, range| {
        pages.read_at(section, range)
    })
}
