//! Semantic highlighting: the language server says what each name is
//! (parameter, property, class, module, a constant...), which the hand-written
//! tokenizer can't know. Those colours are laid over the tokenizer's: they
//! only replace plain names, never strings, comments, keywords or numbers.
//! Replaces `src/renderer/editor/semantic.ts`.
//!
//! Note: the server's columns are UTF-16 units; `decode_semantic_tokens`
//! returns them as given, and the language client converts them to character
//! columns with the line's text before handing them to the highlighter.

use crate::tokenizer::{Token, TokenType};
use std::collections::BTreeMap;

pub type SemToken = Token;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Legend {
    pub token_types: Vec<String>,
    pub token_modifiers: Vec<String>,
}

/// The colour for a token type and its modifiers, or `None` to leave the name as it is.
pub fn semantic_type(kind: &str, mods: &[&str]) -> Option<TokenType> {
    let has = |m: &str| mods.contains(&m);
    Some(match kind {
        "parameter" => TokenType::SemParameter,
        "selfParameter" | "clsParameter" => TokenType::SelfParam,
        "variable" => {
            if has("readonly") {
                TokenType::SemConstant
            } else if has("defaultLibrary") || has("builtin") {
                TokenType::Builtin
            } else {
                TokenType::SemVariable
            }
        }
        "property" => {
            if has("readonly") {
                TokenType::SemConstant
            } else {
                TokenType::SemProperty
            }
        }
        "enumMember" => TokenType::SemConstant,
        "class" | "type" | "enum" | "interface" | "struct" | "typeParameter" => {
            if has("defaultLibrary") || has("builtin") {
                TokenType::Builtin
            } else {
                TokenType::SemClass
            }
        }
        "function" => {
            if has("defaultLibrary") || has("builtin") {
                TokenType::Builtin
            } else {
                TokenType::SemFunction
            }
        }
        "method" => TokenType::SemMethod,
        "namespace" | "module" => TokenType::SemModule,
        "decorator" => TokenType::Decorator,
        _ => return None,
    })
}

/// The server's data (5 numbers per token, positions relative to the previous
/// one) as tokens by line.
pub fn decode_semantic_tokens(data: &[u32], legend: &Legend) -> BTreeMap<usize, Vec<SemToken>> {
    let mut out: BTreeMap<usize, Vec<SemToken>> = BTreeMap::new();
    let mut line: usize = 0;
    let mut ch: usize = 0;
    for group in data.as_chunks::<5>().0 {
        let dl = group[0] as usize;
        line += dl;
        ch = if dl == 0 { ch + group[1] as usize } else { group[1] as usize };
        let length = group[2] as usize;
        let Some(type_name) = legend.token_types.get(group[3] as usize) else {
            continue;
        };
        if length == 0 {
            continue;
        }
        let bits = group[4];
        let mods: Vec<&str> = legend
            .token_modifiers
            .iter()
            .enumerate()
            .filter(|(k, _)| *k < 32 && bits & (1 << k) != 0)
            .map(|(_, m)| m.as_str())
            .collect();
        let Some(kind) = semantic_type(type_name, &mods) else {
            continue;
        };
        out.entry(line).or_default().push(Token::new(ch, ch + length, kind));
    }
    out
}

/// Tokenizer token types a semantic colour may replace (names); the rest are kept.
fn replaceable(kind: TokenType) -> bool {
    matches!(
        kind,
        TokenType::Call
            | TokenType::Builtin
            | TokenType::Function
            | TokenType::Class
            | TokenType::SelfParam
            | TokenType::Property
    )
}

/// The tokenizer's tokens with semantic ones laid over plain names.
pub fn merge_tokens(syntax: &[Token], semantic: Option<&[SemToken]>) -> Vec<Token> {
    let Some(semantic) = semantic.filter(|s| !s.is_empty()) else {
        return syntax.to_vec();
    };
    let mut sem = semantic.to_vec();
    sem.sort_by_key(|t| t.start);
    let mut out = Vec::with_capacity(syntax.len() + sem.len());
    let (mut s, mut k) = (0, 0);
    // Walk both lists in order; a semantic token wins over the names it covers, loses to anything else.
    while s < syntax.len() || k < sem.len() {
        let a = syntax.get(s);
        let b = sem.get(k);
        match (a, b) {
            (Some(a), None) => {
                out.push(*a);
                s += 1;
            }
            (None, Some(b)) => {
                out.push(*b);
                k += 1;
            }
            (Some(a), Some(b)) if a.end <= b.start => {
                out.push(*a);
                s += 1;
            }
            (Some(a), Some(b)) if b.end <= a.start => {
                out.push(*b);
                k += 1;
            }
            (Some(a), Some(b)) => {
                // They overlap.
                if !replaceable(a.kind) || a.start < b.start || a.end > b.end {
                    // A string, comment, keyword... or a token that only partly overlaps: keep the tokenizer's.
                    k += 1;
                } else {
                    s += 1; // the semantic token replaces this name (and anything else it covers)
                }
            }
            (None, None) => break,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legend() -> Legend {
        Legend {
            token_types: ["variable", "parameter", "class", "function", "module", "selfParameter"]
                .map(String::from)
                .to_vec(),
            token_modifiers: ["declaration", "readonly", "defaultLibrary"].map(String::from).to_vec(),
        }
    }

    #[test]
    fn turns_the_relative_5_number_groups_into_tokens_by_line() {
        let map = decode_semantic_tokens(&[0, 4, 1, 1, 0, 0, 6, 3, 0, 0b10, 2, 0, 2, 4, 0], &legend());
        assert_eq!(
            map.get(&0),
            Some(&vec![Token::new(4, 5, TokenType::SemParameter), Token::new(10, 13, TokenType::SemConstant)])
        );
        assert_eq!(map.get(&2), Some(&vec![Token::new(0, 2, TokenType::SemModule)]));
        assert!(!map.contains_key(&1));
    }

    #[test]
    fn skips_unknown_types_and_empty_tokens() {
        assert!(decode_semantic_tokens(&[0, 0, 3, 99, 0, 0, 4, 0, 0, 0], &legend()).is_empty());
    }

    #[test]
    fn maps_types_and_modifiers_to_colours() {
        assert_eq!(semantic_type("function", &["defaultLibrary"]), Some(TokenType::Builtin));
        assert_eq!(semantic_type("class", &[]), Some(TokenType::SemClass));
        assert_eq!(semantic_type("selfParameter", &[]), Some(TokenType::SelfParam));
        assert_eq!(semantic_type("keyword", &[]), None);
    }

    fn t(start: usize, end: usize, kind: TokenType) -> Token {
        Token::new(start, end, kind)
    }

    #[test]
    fn semantic_tokens_replace_names_but_never_strings_comments_or_keywords() {
        let syntax = [
            t(0, 3, TokenType::Keyword),
            t(4, 7, TokenType::Call),
            t(8, 15, TokenType::String),
            t(16, 20, TokenType::Comment),
        ];
        let sem = [t(4, 7, TokenType::SemFunction), t(9, 12, TokenType::SemVariable), t(0, 3, TokenType::SemVariable)];
        assert_eq!(
            merge_tokens(&syntax, Some(&sem)),
            [
                t(0, 3, TokenType::Keyword),
                t(4, 7, TokenType::SemFunction),
                t(8, 15, TokenType::String),
                t(16, 20, TokenType::Comment)
            ]
        );
    }

    #[test]
    fn names_the_tokenizer_left_plain_get_their_semantic_colour() {
        assert_eq!(
            merge_tokens(&[t(0, 6, TokenType::Keyword)], Some(&[t(7, 10, TokenType::SemParameter)])),
            [t(0, 6, TokenType::Keyword), t(7, 10, TokenType::SemParameter)]
        );
    }

    #[test]
    fn no_semantic_tokens_unchanged() {
        let syntax = [t(0, 1, TokenType::Number)];
        assert_eq!(merge_tokens(&syntax, None), syntax);
    }
}
