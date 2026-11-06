use super::*;
use pages::Pages;
use std::{
    cell::RefCell,
    ops::{Bound, Deref},
    rc::Rc,
};

enum Found<'a, V> {
    Open(&'a Interval<V>),
    Page(Rc<Vec<Interval<V>>>, usize),
}
pub struct FactRef<'a, V> {
    found: Found<'a, V>,
}
impl<V> Deref for FactRef<'_, V> {
    type Target = Interval<V>;
    fn deref(&self) -> &Self::Target {
        match &self.found {
            Found::Open(interval) => interval,
            Found::Page(page, index) => &page[*index],
        }
    }
}
pub struct Cursor<'a, V> {
    section: String,
    directory: index::IndexCursor<i32, directory::Entry>,
    entry: Option<directory::Entry>,
    page: Option<Rc<Vec<Interval<V>>>>,
    row: usize,
    column: Option<&'a Column<V>>,
    pending: usize,
    open: bool,
    first: i32,
    last: i32,
    finished: bool,
    owner: Rc<PhaseOwner>,
}
pub struct Reader<'a, K, V, R> {
    facts: &'a Facts<K, V>,
    cache: Rc<RefCell<Pages<'a, R>>>,
    owner: Rc<PhaseOwner>,
}
impl<
    'a,
    K: Ord + Clone + Serialize + DeserializeOwned + Resident + 'static,
    V: PartialEq + Serialize + DeserializeOwned + Resident + 'static,
    R: ReadPages,
> Reader<'a, K, V, R>
{
    pub(super) fn new(facts: &'a Facts<K, V>, source: &'a mut R) -> io::Result<Self> {
        ReadPhase::new(source).reader(facts)
    }
    pub fn cursor(&mut self, key: &K, first: i32, last: i32) -> io::Result<Cursor<'a, V>> {
        if first > last {
            return Err(invalid("Private fact query interval is reversed"));
        }
        let section = self.facts.namespace.section();
        let column = self.facts.columns.get(key);
        let mut cache = self.cache.borrow_mut();
        let root = match column {
            Some(column) => column.root,
            None => index::get(&mut cache, &format!("{section}/keys"), self.facts.keys, key)?
                .unwrap_or_default(),
        };
        let directory = directory::cursor(&mut cache, format!("{section}/directory"), root, first)?;
        Ok(Cursor {
            section,
            directory,
            entry: None,
            page: None,
            row: 0,
            column,
            pending: 0,
            open: false,
            first,
            last,
            finished: false,
            owner: self.owner.clone(),
        })
    }
    pub fn next(&mut self, cursor: &mut Cursor<'a, V>) -> io::Result<Option<FactRef<'a, V>>> {
        if !Rc::ptr_eq(&cursor.owner, &self.owner)
            || cursor.section != self.facts.namespace.section()
        {
            return Err(invalid(
                "Private fact cursor belongs to another column or read phase",
            ));
        }
        if cursor.finished {
            return Ok(None);
        }
        loop {
            if let Some(page) = &cursor.page {
                while cursor.row < page.len() {
                    let at = cursor.row;
                    cursor.row += 1;
                    let row = &page[at];
                    if row.first > cursor.last {
                        cursor.finished = true;
                        return Ok(None);
                    }
                    if row.last >= cursor.first {
                        return Ok(Some(FactRef {
                            found: Found::Page(page.clone(), at),
                        }));
                    }
                }
                cursor.page = None;
            }
            if cursor.entry.is_none() {
                cursor.entry = cursor
                    .directory
                    .next(&mut self.cache.borrow_mut())?
                    .map(|entry| entry.1);
            }
            if let Some(entry) = cursor.entry {
                if entry.first > cursor.last {
                    cursor.finished = true;
                    return Ok(None);
                }
                let page: Rc<Vec<Interval<V>>> = self
                    .cache
                    .borrow_mut()
                    .read(&format!("{}/values", cursor.section), entry.payload)?;
                if page.first().map(|r| r.first) != Some(entry.first)
                    || page.last().map(|r| r.last) != Some(entry.last)
                    || page.iter().any(|r| r.first > r.last)
                    || page.windows(2).any(|r| r[0].last >= r[1].first)
                {
                    return Err(invalid("Private fact payload interval changed"));
                }
                cursor.entry = None;
                cursor.page = Some(page);
                cursor.row = 0;
                continue;
            }
            if let Some(column) = cursor.column {
                while cursor.pending < column.pending.len() {
                    let row = &column.pending[cursor.pending].interval;
                    cursor.pending += 1;
                    if row.first > cursor.last {
                        cursor.finished = true;
                        return Ok(None);
                    }
                    if row.last >= cursor.first {
                        return Ok(Some(FactRef {
                            found: Found::Open(row),
                        }));
                    }
                }
                if !cursor.open {
                    cursor.open = true;
                    if let Some(row) = column
                        .open
                        .as_ref()
                        .map(|v| &v.interval)
                        .filter(|r| r.first <= cursor.last && r.last >= cursor.first)
                    {
                        return Ok(Some(FactRef {
                            found: Found::Open(row),
                        }));
                    }
                }
            }
            cursor.finished = true;
            return Ok(None);
        }
    }
    pub fn at(&mut self, key: &K, hour: i32) -> io::Result<Option<FactRef<'a, V>>> {
        if let Some(column) = self.facts.columns.get(key) {
            if let Some(open) = column
                .open
                .as_ref()
                .filter(|v| v.interval.first <= hour && v.interval.last >= hour)
            {
                return Ok(Some(FactRef {
                    found: Found::Open(&open.interval),
                }));
            }
            let index = column.pending.partition_point(|v| v.interval.last < hour);
            if let Some(row) = column
                .pending
                .get(index)
                .filter(|v| v.interval.first <= hour)
            {
                return Ok(Some(FactRef {
                    found: Found::Open(&row.interval),
                }));
            }
        }
        let mut cursor = self.cursor(key, hour, hour)?;
        self.next(&mut cursor)
    }
    pub fn visit(
        &mut self,
        key: &K,
        first: i32,
        last: i32,
        mut visit: impl FnMut(i32, i32, &Status<V>) -> io::Result<()>,
    ) -> io::Result<()> {
        let mut cursor = self.cursor(key, first, last)?;
        while let Some(row) = self.next(&mut cursor)? {
            visit(first.max(row.first), last.min(row.last), &row.status)?;
        }
        Ok(())
    }
}

// Cursors name the phase they were opened in by this allocation.
pub(super) struct PhaseOwner;
pub struct ReadPhase<'a, R> {
    pub(super) cache: Rc<RefCell<Pages<'a, R>>>,
    pub(super) owner: Rc<PhaseOwner>,
}
impl<'a, R: ReadPages> ReadPhase<'a, R> {
    pub fn new(source: &'a mut R) -> Self {
        Self {
            cache: Rc::new(RefCell::new(Pages::new(source))),
            owner: Rc::new(PhaseOwner),
        }
    }
    pub fn reader<K, V>(&self, facts: &'a Facts<K, V>) -> io::Result<Reader<'a, K, V, R>>
    where
        K: Ord + Clone + Serialize + DeserializeOwned + Resident + 'static,
        V: PartialEq + Serialize + DeserializeOwned + Resident + 'static,
    {
        facts.healthy()?;
        Ok(Reader {
            facts,
            cache: self.cache.clone(),
            owner: self.owner.clone(),
        })
    }
}
enum KeyFound<'a, K> {
    Active(&'a K),
    Archived(index::Item<K, Root>),
}
pub struct KeyRef<'a, K> {
    found: KeyFound<'a, K>,
}
impl<K> Deref for KeyRef<'_, K> {
    type Target = K;
    fn deref(&self) -> &K {
        match &self.found {
            KeyFound::Active(key) => key,
            KeyFound::Archived(row) => &row.0,
        }
    }
}
pub struct KeyCursor<'a, K, V> {
    section: String,
    active: std::iter::Peekable<std::collections::btree_map::Range<'a, K, Column<V>>>,
    archived: index::IndexCursor<K, Root>,
    next: Option<index::Item<K, Root>>,
    owner: Rc<PhaseOwner>,
}
impl<'a, K, V, R> Reader<'a, K, V, R>
where
    K: Ord + Clone + Serialize + DeserializeOwned + Resident + 'static,
    V: PartialEq + Serialize + DeserializeOwned + Resident + 'static,
    R: ReadPages,
{
    pub fn keys(&self, first: Option<&K>, last: Option<&K>) -> io::Result<KeyCursor<'a, K, V>> {
        let section = self.facts.namespace.section();
        let mut cache = self.cache.borrow_mut();
        let mut archived = index::IndexCursor::new(
            &mut cache,
            format!("{}/keys", self.facts.namespace.section()),
            self.facts.keys,
            first,
            last,
        )?;
        let next = archived.next(&mut cache)?;
        Ok(KeyCursor {
            section,
            active: self
                .facts
                .columns
                .range((
                    first.map_or(Bound::Unbounded, Bound::Included),
                    last.map_or(Bound::Unbounded, Bound::Included),
                ))
                .peekable(),
            archived,
            next,
            owner: self.owner.clone(),
        })
    }
    pub fn next_key(&self, cursor: &mut KeyCursor<'a, K, V>) -> io::Result<Option<KeyRef<'a, K>>> {
        if !Rc::ptr_eq(&cursor.owner, &self.owner)
            || cursor.section != self.facts.namespace.section()
        {
            return Err(invalid(
                "Private key cursor belongs to another column or read phase",
            ));
        }
        match (cursor.active.peek(), cursor.next.as_ref()) {
            (Some((key, _)), Some(archived)) if *key < &archived.0 => Ok(Some(KeyRef {
                found: KeyFound::Active(cursor.active.next().unwrap().0),
            })),
            (active, Some(archived)) => {
                let duplicate = active.is_some_and(|(key, _)| *key == &archived.0);
                let next = cursor.archived.next(&mut self.cache.borrow_mut())?;
                if duplicate {
                    cursor.active.next();
                }
                let row = cursor.next.take().unwrap();
                cursor.next = next;
                Ok(Some(KeyRef {
                    found: KeyFound::Archived(row),
                }))
            }
            (Some(_), None) => Ok(Some(KeyRef {
                found: KeyFound::Active(cursor.active.next().unwrap().0),
            })),
            (None, None) => Ok(None),
        }
    }
}
