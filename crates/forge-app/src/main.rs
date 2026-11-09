//! Forge (native): the app.

// Parts of the views are wired up as the workspace is built (M2); remove when it is.
#![allow(dead_code)]

mod document;
mod editor;
mod theme;

use document::Document;
use editor::EditorView;
use gpui_kit::{
    px, size, App, AppContext, Bounds, Entity, Focusable, ParentElement, Render, Styled, Window, WindowBounds,
    WindowOptions,
};
use std::borrow::Cow;
use std::sync::Arc;
use theme::{ForgeTheme, Theme, ThemeName};

/// M1 shell: one editor showing the file given on the command line.
struct Shell {
    editor: Entity<EditorView>,
}

impl Render for Shell {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui_kit::Context<Self>) -> impl gpui_kit::IntoElement {
        gpui_kit::div().size_full().child(self.editor.clone())
    }
}

fn main() {
    gpui_kit::application().run(|cx: &mut App| {
        gpui_kit::init(cx);
        let fonts: Vec<Cow<'static, [u8]>> = theme::FONTS.iter().map(|f| Cow::Borrowed(*f)).collect();
        if let Err(err) = cx.text_system().add_fonts(fonts) {
            eprintln!("couldn't load Forge's fonts: {err}");
        }
        cx.set_global(ForgeTheme(Arc::new(Theme::get(ThemeName::Molten))));
        editor::bind_keys(cx);

        let path = std::env::args().nth(1);
        let text = path.as_deref().and_then(|p| std::fs::read_to_string(p).ok());
        let bounds = Bounds::centered(None, size(px(1100.0), px(750.0)), cx);
        let window = gpui_kit::open_window(
            WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() },
            cx,
            |window, cx| {
                let editor = cx.new(EditorView::new);
                match (&path, &text) {
                    (Some(p), Some(t)) => {
                        let doc = cx.new(|_| Document::new(p.clone(), t));
                        editor.update(cx, |e, cx| e.show(doc, cx));
                    }
                    _ => editor.update(cx, |e, cx| e.clear(Some("Open a file: forge <path>".into()), cx)),
                }
                window.focus(&editor.focus_handle(cx), cx);
                cx.new(|_| Shell { editor })
            },
        );
        if let Err(err) = window {
            eprintln!("couldn't open a window: {err}");
        }
    });
}
