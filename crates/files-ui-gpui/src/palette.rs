//! The command palette (Ctrl+Shift+P), like Zed's: type a few letters of a
//! command or a folder, arrows choose, Enter runs it.

use crate::i18n::tr;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::path::PathBuf;

/// What a palette line does.
pub enum Run {
    Action(Box<dyn Action>),
    Go(PathBuf),
}

pub struct Entry {
    pub label: String,
    /// The shortcut or the folder's path, shown muted on the right.
    pub hint: String,
    pub run: Run,
}

pub struct Palette {
    input: Entity<InputState>,
    entries: Vec<Entry>,
    /// Indexes into `entries` that match what is typed.
    shown: Vec<usize>,
    selected: usize,
    /// Called with the chosen entry; the palette closes first.
    on_choose: Box<Choose>,
}

/// Runs the chosen entry.
type Choose = dyn Fn(&Run, &mut Window, &mut App);

/// True when every letter of `query` appears in `text` in order, ignoring
/// case and Polish marks ("zolw" finds "Żółw").
fn matches(query: &str, text: &str) -> bool {
    let fold = |c: char| match c {
        'ą' => 'a',
        'ć' => 'c',
        'ę' => 'e',
        'ł' => 'l',
        'ń' => 'n',
        'ó' => 'o',
        'ś' => 's',
        'ź' | 'ż' => 'z',
        c => c,
    };
    let mut chars = text.chars().flat_map(char::to_lowercase).map(fold);
    query
        .chars()
        .flat_map(char::to_lowercase)
        .map(fold)
        .filter(|c| !c.is_whitespace())
        .all(|q| chars.any(|c| c == q))
}

impl Palette {
    pub fn new(
        entries: Vec<Entry>,
        on_choose: impl Fn(&Run, &mut Window, &mut App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(tr!(
                "Type a command or a folder…",
                "Wpisz polecenie albo folder…"
            ))
        });
        cx.subscribe_in(
            &input,
            window,
            |this, input, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => {
                    let q = input.read(cx).value().to_string();
                    this.shown = (0..this.entries.len())
                        .filter(|&i| {
                            matches(&q, &this.entries[i].label)
                                || matches(&q, &this.entries[i].hint)
                        })
                        .collect();
                    this.selected = 0;
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => this.choose(window, cx),
                _ => {}
            },
        )
        .detach();
        input.update(cx, |i, cx| i.focus(window, cx));
        let shown = (0..entries.len()).collect();
        Palette {
            input,
            entries,
            shown,
            selected: 0,
            on_choose: Box::new(on_choose),
        }
    }

    /// Puts the cursor in the field (the dialog takes focus as it opens).
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |i, cx| i.focus(window, cx));
    }

    fn choose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(e) = self
            .shown
            .get(self.selected)
            .and_then(|&i| self.entries.get(i))
        {
            window.close_dialog(cx);
            (self.on_choose)(&e.run, window, cx);
        }
    }
}

impl Render for Palette {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let (muted, active) = (t.muted_foreground, t.table_active);
        let mut list = div()
            .id("palette-list")
            .v_flex()
            .max_h(px(360.))
            .overflow_y_scroll();
        for (row, &i) in self.shown.iter().enumerate().take(200) {
            let e = &self.entries[i];
            list = list.child(
                div()
                    .id(("palette", row))
                    .flex()
                    .flex_row()
                    .gap_3()
                    .px_2()
                    .py_1()
                    .rounded(t.radius)
                    .cursor_pointer()
                    .when(row == self.selected, |d| d.bg(active))
                    .child(div().flex_1().truncate().child(e.label.clone()))
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .truncate()
                            .max_w(px(260.))
                            .child(e.hint.clone()),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.selected = row;
                        this.choose(window, cx);
                    })),
            );
        }
        div()
            .v_flex()
            .gap_2()
            // Arrows move the choice while the field keeps the focus.
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                let last = this.shown.len().saturating_sub(1);
                match e.keystroke.key.as_str() {
                    "down" => this.selected = (this.selected + 1).min(last),
                    "up" => this.selected = this.selected.saturating_sub(1),
                    _ => return,
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .child(Input::new(&self.input))
            .child(list)
    }
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn letters_in_order() {
        assert!(matches("nt", "New tab"));
        assert!(matches("NEW f", "New folder"));
        assert!(matches("", "anything"));
        assert!(!matches("tn", "New tab x"), "order matters");
        assert!(matches("zaz", "Zażółć"));
    }
}
