//! Polish and English UI. Each text sits next to its translation at the call
//! site: `tr!("Back", "Wstecz")`, with `format!` arguments after the pair.

use std::sync::atomic::{AtomicBool, Ordering};

static POLISH: AtomicBool = AtomicBool::new(false);

pub fn polish() -> bool {
    POLISH.load(Ordering::Relaxed)
}

pub fn set_polish(on: bool) {
    POLISH.store(on, Ordering::Relaxed);
}

macro_rules! tr {
    ($en:literal, $pl:literal $(, $arg:expr)* $(,)?) => {
        if $crate::i18n::polish() {
            format!($pl $(, $arg)*)
        } else {
            format!($en $(, $arg)*)
        }
    };
}
pub(crate) use tr;
