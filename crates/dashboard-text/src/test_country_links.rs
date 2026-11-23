use replay_data::rows::{CountryLinkRow, TableId, TableRows};
use replay_frame::frame::{DecodedFrame, TableView};

pub fn rows(links: impl IntoIterator<Item = CountryLinkRow>) -> TableRows {
    let mut links: Vec<_> = links.into_iter().collect();
    links.sort_unstable_by_key(|row| row.id);
    let rows = TableRows::CountryLinks(links);
    rows.validate_keys().expect("valid fixture country links");
    rows
}

pub fn extend(frame: &mut DecodedFrame, links: impl IntoIterator<Item = CountryLinkRow>) {
    let TableView::Complete(TableRows::CountryLinks(current)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    else {
        panic!("the fixture records country links");
    };
    let rows = rows(std::mem::take(current).into_iter().chain(links));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(rows);
}
