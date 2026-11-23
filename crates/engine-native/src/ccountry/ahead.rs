use super::{Country, Reader};
use engine_native_memory::{Memory, batch_fallback::Link, read};

pub struct Ahead {
    slot: u32,
    listed: Result<Option<Country>, &'static str>,
    link: Link,
}

impl Ahead {
    pub fn read<M: Memory>(
        memory: &M,
        countries: &Reader<'_, M>,
        slot: u32,
        offset: usize,
        hint: impl FnOnce(usize),
    ) -> Option<Self> {
        if slot >= countries.count() {
            return None;
        }
        let listed = countries.at(slot);
        let link = match listed {
            Ok(Some(country)) => match country
                .pointer
                .checked_add(offset)
                .and_then(|at| read::<u64>(memory, at))
            {
                Some(link) => Link::Read(link),
                None => Link::Failed,
            },
            _ => Link::Unread,
        };
        if let Link::Read(link) = link
            && let Ok(target) = usize::try_from(link)
            && target != 0
        {
            hint(target);
        }
        Some(Self { slot, listed, link })
    }

    pub fn take<M: Memory>(
        ahead: &mut Option<Self>,
        countries: &Reader<'_, M>,
        slot: u32,
    ) -> (Result<Option<Country>, &'static str>, Link) {
        match ahead.take() {
            Some(ahead) if ahead.slot == slot => (ahead.listed, ahead.link),
            _ => (countries.at(slot), Link::Unread),
        }
    }
}
