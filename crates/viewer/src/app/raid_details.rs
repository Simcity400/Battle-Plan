//! The raid target details window's toggle: one window is open at a time.

use std::sync::Arc;
use viewer_raids::map::raids::tooltips::Details;

/// What the overlay draws of the raid bubbles from the frame alone, laying nothing out: the bubble
/// whose details window is open, with what its click built, and the bubble under the pointer.
pub(crate) type Shown = (Option<(u32, Arc<Details>)>, Option<u32>);

/// Whether an overlay last drawn showing `drawn` draws again to show `now`: a click's window opens
/// or closes, or the pointer moves to another bubble.
pub(crate) fn redraws(drawn: &Shown, now: &Shown) -> bool {
    let same_window = match (&drawn.0, &now.0) {
        (Some((one, a)), Some((two, b))) => one == two && Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    };
    !same_window || drawn.1 != now.1
}

/// Whether a click on the offer bubble of `clicked` opens its window: the bubble whose window is
/// open closes it, another bubble replaces it.
pub(crate) fn opens(open: Option<u32>, clicked: u32) -> bool {
    open != Some(clicked)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_bubble_closes_its_window_and_another_replaces_it() {
        assert!(opens(None, 7));
        assert!(!opens(Some(7), 7));
        assert!(opens(Some(7), 9));
    }

    #[test]
    fn a_click_or_a_hover_changes_what_the_overlay_draws_without_a_layout() {
        let details = Arc::new(Details { header: Ok("Raid targets".into()), entries: Vec::new() });
        let open = (Some((7, Arc::clone(&details))), None);
        assert!(redraws(&(None, None), &open), "the click opens its window");
        assert!(!redraws(&open, &open.clone()), "the same window, drawn already");
        assert!(redraws(&open, &(Some((7, Arc::new((*details).clone()))), None)), "a new click's build");
        assert!(redraws(&open, &(None, None)), "the window closes");
        assert!(redraws(&(None, None), &(None, Some(7))), "the pointer reaches a bubble");
    }
}
