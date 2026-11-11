//! File operations in progress (paste, move, copy, delete), shown together
//! above the window's bottom edge with a bar and a cancel button each.

use crate::i18n::tr;
use files_win::fileop::OpProgress;
use gpui_kit::component::button::*;
use gpui_kit::component::*;
use gpui_kit::*;
use std::sync::atomic::Ordering;
use std::sync::Arc;

struct Op {
    label: String,
    progress: Arc<OpProgress>,
}

#[derive(Default)]
struct Ops(Vec<Op>);

impl Global for Ops {}

/// Registers an operation about to start; hand the result to `fileop`.
pub fn start(label: String, cx: &mut App) -> Arc<OpProgress> {
    let progress: Arc<OpProgress> = Default::default();
    cx.default_global::<Ops>().0.push(Op {
        label,
        progress: progress.clone(),
    });
    progress
}

/// Drops finished operations; true while any is still running.
pub fn tidy(cx: &mut App) -> bool {
    let ops = cx.default_global::<Ops>();
    ops.0
        .retain(|op| !op.progress.finished.load(Ordering::SeqCst));
    !ops.0.is_empty()
}

/// One line per running operation, or nothing.
pub fn bar(cx: &App) -> Option<Div> {
    let ops = cx.try_global::<Ops>().filter(|o| !o.0.is_empty())?;
    let t = cx.theme();
    let mut list = div().v_flex().gap_1().px_3().pb_2().text_sm();
    for (i, op) in ops.0.iter().enumerate() {
        let (total, done) = (
            op.progress.total.load(Ordering::Relaxed),
            op.progress.done.load(Ordering::Relaxed),
        );
        let part = if total > 0 {
            done as f32 / total as f32
        } else {
            0.
        };
        let cancelling = op.progress.cancel.load(Ordering::Relaxed);
        let progress = op.progress.clone();
        list = list.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_3()
                .child(div().flex_1().truncate().child(op.label.clone()))
                .child(
                    div()
                        .text_color(t.muted_foreground)
                        .child(format!("{:.0}%", part * 100.)),
                )
                .child(
                    div()
                        .w(px(200.))
                        .h(px(6.))
                        .rounded_full()
                        .bg(t.muted)
                        .child(
                            div()
                                .h_full()
                                .rounded_full()
                                .bg(t.primary)
                                .w(relative(part.clamp(0., 1.))),
                        ),
                )
                .child(
                    Button::new(("op-cancel", i))
                        .cursor_pointer()
                        .ghost()
                        .xsmall()
                        .disabled(cancelling)
                        .label(if cancelling {
                            tr!("Cancelling…", "Anulowanie…")
                        } else {
                            tr!("Cancel", "Anuluj")
                        })
                        // Stops before the next item; Windows finishes the one in hand.
                        .on_click(move |_, _, _| progress.cancel.store(true, Ordering::SeqCst)),
                ),
        );
    }
    Some(list)
}
