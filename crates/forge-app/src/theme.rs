//! Forge's colours and fonts. Replaces the colour tokens in `src/renderer/theme.css`.

use forge_core::settings::{CornerStyle, Theme as ThemeName};
use forge_text::TokenType;
use gpui_kit::{rgba, FontStyle, FontWeight, Global, Hsla, Pixels};
use std::sync::Arc;

/// The theme every view draws with (switching themes replaces it).
pub struct ForgeTheme(pub Arc<Theme>);

impl Global for ForgeTheme {}

pub const CODE_FONT: &str = "JetBrains Mono";
pub const UI_FONT: &str = "Space Grotesk";

/// The bundled fonts (SIL Open Font License, see assets/fonts/OFL-*.txt).
pub const FONTS: [&[u8]; 7] = [
    include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../../../assets/fonts/JetBrainsMono-Bold.ttf"),
    include_bytes!("../../../assets/fonts/JetBrainsMono-Italic.ttf"),
    include_bytes!("../../../assets/fonts/JetBrainsMono-BoldItalic.ttf"),
    include_bytes!("../../../assets/fonts/SpaceGrotesk-Regular.ttf"),
    include_bytes!("../../../assets/fonts/SpaceGrotesk-Medium.ttf"),
    include_bytes!("../../../assets/fonts/SpaceGrotesk-Bold.ttf"),
];

fn c(hex: u32) -> Hsla {
    rgba(hex).into()
}

/// Every colour token of the old `theme.css` (as `--name` → `name`).
#[derive(Clone, Debug)]
pub struct Theme {
    /// Scale panel chrome, never the independently sized editor/terminal.
    pub ui_scale: f32,
    pub corner_style: CornerStyle,
    pub name: ThemeName,
    pub bg: Hsla,
    pub bg_panel: Hsla,
    pub raised: Hsla,
    pub bg_hover: Hsla,
    pub bg_active: Hsla,
    pub border: Hsla,
    pub line_strong: Hsla,
    pub overlay: Hsla,
    pub shadow: Hsla,
    pub fg: Hsla,
    pub fg_muted: Hsla,
    pub accent: Hsla,
    pub on_accent: Hsla,
    pub link: Hsla,
    pub heat_1: Hsla,
    pub heat_2: Hsla,
    pub heat_3: Hsla,
    pub heat_4: Hsla,
    pub on_heat: Hsla,
    pub err: Hsla,
    pub warn: Hsla,
    pub ok: Hsla,
    pub info: Hsla,
    pub selection: Hsla,
    pub selection_unfocused: Hsla,
    pub match_bg: Hsla,
    pub highlight: Hsla,
    pub highlight_strong: Hsla,
    pub shade: Hsla,
    pub shade_strong: Hsla,
    pub breakpoint: Hsla,
    pub exec_line: Hsla,
    pub exec_arrow: Hsla,
    pub minimap_view: Hsla,
    pub git_modified: Hsla,
    pub git_added: Hsla,
    pub git_deleted: Hsla,
    pub diff_add: Hsla,
    pub diff_del: Hsla,
    pub git_conflict: Hsla,
    /// Merge conflicts in the editor: your side, the incoming side.
    pub conflict_current: Hsla,
    pub conflict_incoming: Hsla,
    /// The commit graph's branch lines.
    pub lanes: [Hsla; 8],
    pub ref_bg: Hsla,
    pub graph_out: Hsla,
    pub graph_in: Hsla,
    pub tok_keyword: Hsla,
    pub tok_constant: Hsla,
    pub tok_builtin: Hsla,
    pub tok_self: Hsla,
    pub tok_function: Hsla,
    pub tok_class: Hsla,
    pub tok_call: Hsla,
    pub tok_decorator: Hsla,
    pub tok_string: Hsla,
    pub tok_number: Hsla,
    pub tok_comment: Hsla,
}

impl Theme {
    /// Adjust an existing radius, preserving any density or zoom scaling.
    pub fn corner(&self, base: Pixels) -> Pixels {
        base * match self.corner_style {
            CornerStyle::Square => 0.0,
            CornerStyle::Subtle => 1.0,
            CornerStyle::Rounded => 2.0,
            CornerStyle::Soft => 3.0,
        }
    }

    pub fn px(&self, value: f32) -> gpui_kit::Pixels {
        gpui_kit::px(value * self.ui_scale)
    }

    pub fn get(name: ThemeName) -> Self {
        match name {
            ThemeName::Molten => Self::molten(),
            ThemeName::Plain => Self::plain(),
            ThemeName::Midnight => Self::palette(name, [0x101522, 0xe2eafd, 0x9baaca, 0x91afff, 0xc5a8ff, 0x89d9bd]),
            ThemeName::Slate => Self::palette(name, [0x21252c, 0xe9edf3, 0xaeb8c5, 0x9fc7ef, 0xe0b9cb, 0xb9d3ab]),
            ThemeName::Aurora => Self::palette(name, [0x11211e, 0xe1f4eb, 0x9cbdb0, 0x83dfb6, 0xc7bcff, 0xbade83]),
            ThemeName::Ember => Self::palette(name, [0x26181b, 0xf8e7dd, 0xc6a69e, 0xffa881, 0xf4a6c1, 0xe5ca8b]),
            ThemeName::Orchid => Self::palette(name, [0x21172c, 0xf2e6fa, 0xbca7ce, 0xd9afff, 0xffa9d2, 0xa6dec2]),
            ThemeName::Ocean => Self::palette(name, [0x10232c, 0xdef4f9, 0x98bdca, 0x7ce1ef, 0xa4baff, 0x9fe2aa]),
            ThemeName::Parchment => Self::palette(name, [0xf8f0df, 0x34271c, 0x675445, 0x994722, 0x833765, 0x37613e]),
            ThemeName::Daybreak => Self::palette(name, [0xf3f7fc, 0x192941, 0x4b5f7a, 0x285c9f, 0x694294, 0x246b5b]),
        }
    }

    fn palette(name: ThemeName, colors: [u32; 6]) -> Self {
        let [bg, fg, muted, accent, keyword, string] = colors.map(|rgb| c((rgb << 8) | 0xff));
        let (err, warn, info) = if name.is_light() {
            (c(0xae263cff), c(0x815000ff), c(0x25578dff))
        } else {
            (c(0xff909cff), c(0xf1c382ff), c(0x95d4ffff))
        };
        let neutral = |opacity| bg.blend(fg.opacity(opacity));
        let tint = |opacity| bg.blend(accent.opacity(opacity));
        Self {
            name,
            bg,
            fg,
            fg_muted: muted,
            accent,
            on_accent: bg,
            link: accent,
            bg_panel: neutral(0.035),
            raised: neutral(0.07),
            bg_hover: neutral(0.12),
            bg_active: tint(0.19),
            border: neutral(0.17),
            line_strong: neutral(0.35),
            heat_1: info,
            heat_2: accent,
            heat_3: keyword,
            heat_4: err,
            on_heat: bg,
            err,
            warn,
            ok: string,
            info,
            selection: tint(0.30),
            selection_unfocused: tint(0.17),
            match_bg: warn.opacity(0.23),
            highlight: accent.opacity(0.12),
            highlight_strong: accent.opacity(0.28),
            shade: fg.opacity(0.065),
            shade_strong: fg.opacity(0.16),
            breakpoint: err,
            exec_line: warn.opacity(0.14),
            exec_arrow: warn,
            minimap_view: fg.opacity(0.16),
            git_modified: warn,
            git_added: string,
            git_deleted: err,
            git_conflict: err,
            diff_add: string.opacity(0.16),
            diff_del: err.opacity(0.16),
            conflict_current: string.opacity(0.16),
            conflict_incoming: info.opacity(0.16),
            lanes: [accent, info, string, err, keyword, warn, muted, fg],
            ref_bg: info.opacity(0.18),
            graph_out: accent,
            graph_in: info,
            tok_keyword: keyword,
            tok_constant: warn,
            tok_builtin: info,
            tok_self: muted,
            tok_function: accent,
            tok_class: info,
            tok_call: accent,
            tok_decorator: keyword,
            tok_string: string,
            tok_number: warn,
            tok_comment: muted,
            ..Self::molten()
        }
    }

    pub fn molten() -> Self {
        Self {
            ui_scale: 1.0,
            corner_style: CornerStyle::Subtle,
            name: ThemeName::Molten,
            bg: c(0x120c0aff),
            bg_panel: c(0x170e0bff),
            raised: c(0x1d120eff),
            bg_hover: c(0x24150fff),
            bg_active: c(0x3a2017ff),
            border: c(0x3a2017ff),
            line_strong: c(0x5a3322ff),
            overlay: c(0x00000073),
            shadow: c(0x00000080),
            fg: c(0xf3e2d4ff),
            fg_muted: c(0xa08576ff),
            accent: c(0xffb02eff),
            on_accent: c(0x1a0904ff),
            link: c(0xffc857ff),
            heat_1: c(0xffcf5cff),
            heat_2: c(0xffb02eff),
            heat_3: c(0xff6a1aff),
            heat_4: c(0xe8431dff),
            on_heat: c(0x1a0904ff),
            err: c(0xff5a4eff),
            warn: c(0xffb02eff),
            ok: c(0x9fcf7aff),
            info: c(0x7cc4e8ff),
            selection: c(0x5a2a14ff),
            selection_unfocused: c(0x2e1a12ff),
            match_bg: c(0xffb02e4d),
            highlight: c(0xffffff1a),
            highlight_strong: c(0xffffff40),
            shade: c(0xffffff12),
            shade_strong: c(0xffffff26),
            breakpoint: c(0xff5a4eff),
            exec_line: c(0xffb02e26),
            exec_arrow: c(0xffcf5cff),
            minimap_view: c(0xffffff1a),
            git_modified: c(0xffc857ff),
            git_added: c(0x9fcf7aff),
            git_deleted: c(0xff5a4eff),
            diff_add: c(0x9fcf7a26),
            diff_del: c(0xff5a4e29),
            git_conflict: c(0xff5a4eff),
            conflict_current: c(0x9fcf7a24),
            conflict_incoming: c(0x7cc4e829),
            lanes: [
                c(0xffb02eff),
                c(0x7cc4e8ff),
                c(0x9fcf7aff),
                c(0xff6a3dff),
                c(0xc79bf2ff),
                c(0xf2c879ff),
                c(0x4fd1c5ff),
                c(0xe88aa8ff),
            ],
            ref_bg: c(0x7cc4e833),
            graph_out: c(0xffb02eff),
            graph_in: c(0x7cc4e8ff),
            tok_keyword: c(0xff6a3dff),
            tok_constant: c(0xff8f5aff),
            tok_builtin: c(0xe0b48aff),
            tok_self: c(0xd98a6aff),
            tok_function: c(0xffc857ff),
            tok_class: c(0xffd27aff),
            tok_call: c(0xf5c98aff),
            tok_decorator: c(0xd9a35fff),
            tok_string: c(0xe8a87cff),
            tok_number: c(0xf0a36bff),
            tok_comment: c(0xa0806fff),
        }
    }

    pub fn plain() -> Self {
        Self {
            ui_scale: 1.0,
            corner_style: CornerStyle::Subtle,
            name: ThemeName::Plain,
            bg: c(0x000000ff),
            bg_panel: c(0x000000ff),
            raised: c(0x0d0d0dff),
            bg_hover: c(0x1f1f1fff),
            bg_active: c(0x333333ff),
            border: c(0x8a8a8aff),
            line_strong: c(0xffffffff),
            overlay: c(0x000000a6),
            shadow: c(0x000000ff),
            fg: c(0xffffffff),
            fg_muted: c(0xc8c8c8ff),
            accent: c(0xffffffff),
            on_accent: c(0x000000ff),
            link: c(0xffffffff),
            heat_1: c(0xffffffff),
            heat_2: c(0xffffffff),
            heat_3: c(0xffffffff),
            heat_4: c(0xffffffff),
            on_heat: c(0x000000ff),
            err: c(0xff6b6bff),
            warn: c(0xffd75eff),
            ok: c(0x8fe388ff),
            info: c(0x8cc8ffff),
            selection: c(0x264f78ff),
            selection_unfocused: c(0x333333ff),
            match_bg: c(0xffffff40),
            highlight: c(0xffffff26),
            highlight_strong: c(0xffffff59),
            shade: c(0xffffff14),
            shade_strong: c(0xffffff26),
            breakpoint: c(0xff6b6bff),
            exec_line: c(0xffffff26),
            exec_arrow: c(0xffffffff),
            minimap_view: c(0xffffff26),
            git_modified: c(0xffd75eff),
            git_added: c(0x8fe388ff),
            git_deleted: c(0xff6b6bff),
            diff_add: c(0x8fe38826),
            diff_del: c(0xff6b6b29),
            git_conflict: c(0xff6b6bff),
            conflict_current: c(0x8fe38824),
            conflict_incoming: c(0x8cc8ff29),
            lanes: [
                c(0xffffffff),
                c(0x8cc8ffff),
                c(0x8fe388ff),
                c(0xffd75eff),
                c(0xff9e9eff),
                c(0xd0b3ffff),
                c(0x9ee8e0ff),
                c(0xffb3d1ff),
            ],
            ref_bg: c(0x8cc8ff33),
            graph_out: c(0xffffffff),
            graph_in: c(0x8cc8ffff),
            tok_keyword: c(0xffffffff),
            tok_constant: c(0xffffffff),
            tok_builtin: c(0xe0e0e0ff),
            tok_self: c(0xe0e0e0ff),
            tok_function: c(0xffffffff),
            tok_class: c(0xffffffff),
            tok_call: c(0xf0f0f0ff),
            tok_decorator: c(0xd0d0d0ff),
            tok_string: c(0xc8c8c8ff),
            tok_number: c(0xe0e0e0ff),
            tok_comment: c(0x9a9a9aff),
        }
    }

    /// The colour, weight and style of a token (as `.tok-*` in the old styles.css).
    pub fn token_style(&self, kind: TokenType) -> (Hsla, FontWeight, FontStyle) {
        let n = FontWeight::NORMAL;
        let i = FontStyle::Italic;
        let r = FontStyle::Normal;
        match kind {
            TokenType::Keyword => (self.tok_keyword, n, r),
            TokenType::Constant => (self.tok_constant, n, r),
            TokenType::Builtin => (self.tok_builtin, n, r),
            TokenType::SelfParam => (self.tok_self, n, i),
            TokenType::Function => (self.tok_function, n, r),
            TokenType::Class => (self.tok_class, n, r),
            TokenType::Call => (self.tok_call, n, r),
            TokenType::Decorator => (self.tok_decorator, n, r),
            TokenType::String => (self.tok_string, n, r),
            TokenType::Number => (self.tok_number, n, r),
            TokenType::Comment => (self.tok_comment, n, i),
            TokenType::Property => (self.tok_function, n, r),
            TokenType::SemParameter => (self.tok_self, n, r),
            TokenType::SemVariable => (self.fg, n, r),
            TokenType::SemConstant => (self.tok_constant, n, r),
            TokenType::SemProperty => (self.tok_function, n, r),
            TokenType::SemClass => (self.tok_class, n, r),
            TokenType::SemFunction | TokenType::SemMethod => (self.tok_call, n, r),
            TokenType::SemModule => (self.tok_builtin, n, i),
        }
    }

    /// Diagnostic severity colour (1 error, 2 warning, 3 info).
    pub fn severity(&self, severity: u8) -> Hsla {
        match severity {
            1 => self.err,
            2 => self.warn,
            _ => self.info,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_themes_define_distinct_palettes() {
        let mut backgrounds = Vec::new();
        for &(name, _) in ThemeName::ALL {
            let t = Theme::get(name);
            assert_eq!(t.name, name);
            assert_ne!(t.bg, t.fg);
            assert!(!backgrounds.contains(&t.bg));
            backgrounds.push(t.bg);
        }
    }
}
