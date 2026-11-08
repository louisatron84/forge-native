//! A hand-written Python tokenizer for syntax highlighting. Replaces
//! `src/renderer/editor/tokenizer.ts`.
//!
//! It works one line at a time. The only thing that can carry over from one
//! line to the next is an open triple-quoted string (or, for JSONC, a block
//! comment), so the state between lines is a small enum. Only interesting
//! pieces get a token; plain text, operators and spaces are drawn in the
//! default colour. Token columns count characters.

use regex::Regex;
use std::sync::LazyLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenType {
    Keyword,
    /// True, False, None
    Constant,
    /// print, len, ...
    Builtin,
    /// self, cls
    SelfParam,
    /// The name after `def`.
    Function,
    /// The name after `class`.
    Class,
    /// A name followed by `(`.
    Call,
    Decorator,
    String,
    Number,
    Comment,
    /// Object keys (JSON, config files).
    Property,
    // From the language server (semantic highlighting, see semantic.rs):
    SemParameter,
    SemVariable,
    SemConstant,
    SemProperty,
    SemClass,
    SemFunction,
    SemMethod,
    SemModule,
}

impl TokenType {
    /// The old CSS class suffix (`tok-…`), used by themes and tests.
    pub fn as_str(self) -> &'static str {
        match self {
            TokenType::Keyword => "keyword",
            TokenType::Constant => "constant",
            TokenType::Builtin => "builtin",
            TokenType::SelfParam => "self",
            TokenType::Function => "function",
            TokenType::Class => "class",
            TokenType::Call => "call",
            TokenType::Decorator => "decorator",
            TokenType::String => "string",
            TokenType::Number => "number",
            TokenType::Comment => "comment",
            TokenType::Property => "property",
            TokenType::SemParameter => "sem-parameter",
            TokenType::SemVariable => "sem-variable",
            TokenType::SemConstant => "sem-constant",
            TokenType::SemProperty => "sem-property",
            TokenType::SemClass => "sem-class",
            TokenType::SemFunction => "sem-function",
            TokenType::SemMethod => "sem-method",
            TokenType::SemModule => "sem-module",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub kind: TokenType,
}

impl Token {
    pub const fn new(start: usize, end: usize, kind: TokenType) -> Self {
        Self { start, end, kind }
    }
}

/// What carries over from the end of one line to the start of the next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LineState {
    #[default]
    Normal,
    /// Inside a """ string.
    TripleDouble,
    /// Inside a ''' string.
    TripleSingle,
    /// Inside a /* comment */ (JSONC).
    BlockComment,
}

const KEYWORDS: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except",
    "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise",
    "return", "try", "while", "with", "yield",
];

const CONSTANTS: &[&str] = &["True", "False", "None", "NotImplemented", "Ellipsis", "__debug__"];

const BUILTINS: &[&str] = &[
    "abs",
    "aiter",
    "all",
    "anext",
    "any",
    "ascii",
    "bin",
    "bool",
    "breakpoint",
    "bytearray",
    "bytes",
    "callable",
    "chr",
    "classmethod",
    "compile",
    "complex",
    "delattr",
    "dict",
    "dir",
    "divmod",
    "enumerate",
    "eval",
    "exec",
    "filter",
    "float",
    "format",
    "frozenset",
    "getattr",
    "globals",
    "hasattr",
    "hash",
    "help",
    "hex",
    "id",
    "input",
    "int",
    "isinstance",
    "issubclass",
    "iter",
    "len",
    "list",
    "locals",
    "map",
    "max",
    "memoryview",
    "min",
    "next",
    "object",
    "oct",
    "open",
    "ord",
    "pow",
    "print",
    "property",
    "range",
    "repr",
    "reversed",
    "round",
    "set",
    "setattr",
    "slice",
    "sorted",
    "staticmethod",
    "str",
    "sum",
    "super",
    "tuple",
    "type",
    "vars",
    "zip",
    "__import__",
    // common exceptions
    "BaseException",
    "Exception",
    "ArithmeticError",
    "AssertionError",
    "AttributeError",
    "EOFError",
    "FileNotFoundError",
    "ImportError",
    "IndexError",
    "KeyError",
    "KeyboardInterrupt",
    "LookupError",
    "ModuleNotFoundError",
    "NameError",
    "NotImplementedError",
    "OSError",
    "RuntimeError",
    "StopIteration",
    "SyntaxError",
    "TypeError",
    "ValueError",
    "ZeroDivisionError",
];

static NUMBER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:0[xX][\da-fA-F_]+|0[oO][0-7_]+|0[bB][01_]+|(?:\d[\d_]*\.?[\d_]*|\.\d[\d_]*)(?:[eE][+-]?\d[\d_]*)?[jJ]?)",
    )
    .expect("valid regex")
});

pub(crate) fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

pub(crate) fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn starts_with_at(text: &[char], i: usize, pat: &str) -> bool {
    (i..).zip(pat.chars()).all(|(k, p)| text.get(k) == Some(&p))
}

/// Length of an identifier starting at `i` (0 if none).
fn ident_len(text: &[char], i: usize) -> usize {
    if !text.get(i).copied().is_some_and(is_ident_start) {
        return 0;
    }
    let mut j = i + 1;
    while j < text.len() && is_ident_char(text[j]) {
        j += 1;
    }
    j - i
}

/// A string prefix (f, r, b, u, rb...) of 0-2 letters followed by a quote, at `i`.
/// Like the JavaScript `(?:[rRbBuUfF]{1,2})?(?="""|'''|"|')`: longest prefix first.
fn string_prefix(text: &[char], i: usize) -> Option<usize> {
    let is_prefix = |c: char| "rRbBuUfF".contains(c);
    let is_quote = |k: usize| matches!(text.get(k), Some('"') | Some('\''));
    let mut n = 0;
    while n < 2 && text.get(i + n).copied().is_some_and(is_prefix) {
        n += 1;
    }
    (0..=n).rev().find(|&len| is_quote(i + len))
}

/// Decorator `@name.attr` at `i`; returns its length.
fn decorator_len(text: &[char], i: usize) -> usize {
    if text.get(i) != Some(&'@') {
        return 0;
    }
    let first = ident_len(text, i + 1);
    if first == 0 {
        return 0;
    }
    let mut j = i + 1 + first;
    while text.get(j) == Some(&'.') {
        let next = ident_len(text, j + 1);
        if next == 0 {
            break;
        }
        j += 1 + next;
    }
    j - i
}

/// Find where a string body ends. Starts scanning at `from` (just after the
/// opening quote). Returns the index just after the closing quote, or `None`
/// if the string doesn't close on this line. Backslashes escape the next char.
fn find_string_end(text: &[char], from: usize, quote: &str) -> Option<usize> {
    let mut i = from;
    while i < text.len() {
        if text[i] == '\\' {
            i += 2;
            continue;
        }
        if starts_with_at(text, i, quote) {
            return Some(i + quote.chars().count());
        }
        i += 1;
    }
    None
}

/// An f-string on one line, f"total: {price * qty:.2f}": the text parts are
/// strings, the {expressions} are highlighted as code. {{ and }} are literal
/// braces. `start..end` is the whole string, `body..body_end` the part
/// between the quotes.
fn f_string_tokens(text: &[char], start: usize, end: usize, body: usize, body_end: usize) -> Vec<Token> {
    let mut out = Vec::new();
    let mut str_from = start;
    let mut i = body;
    while i < body_end {
        if text[i] == '\\' {
            i += 2;
            continue;
        }
        if (text[i] == '{' || text[i] == '}') && text.get(i + 1) == Some(&text[i]) {
            i += 2; // {{ or }}: a literal brace
            continue;
        }
        if text[i] != '{' {
            i += 1;
            continue;
        }
        // Find the matching } (expressions can nest brackets and contain strings).
        let mut depth: isize = 0;
        let mut j = i + 1;
        let mut quote: Option<char> = None;
        while j < body_end {
            let c = text[j];
            if let Some(q) = quote {
                if c == q {
                    quote = None;
                }
            } else if c == '"' || c == '\'' {
                quote = Some(c);
            } else if c == '{' || c == '(' || c == '[' {
                depth += 1;
            } else if c == ')' || c == ']' {
                depth -= 1;
            } else if c == '}' {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            j += 1;
        }
        if j >= body_end {
            break; // unclosed {: leave the rest as string
        }
        if i + 1 > str_from {
            out.push(Token::new(str_from, i + 1, TokenType::String)); // ...text{
        }
        let inner: String = text[i + 1..j].iter().collect();
        for t in tokenize_line(&inner, LineState::Normal).0 {
            out.push(Token::new(t.start + i + 1, t.end + i + 1, t.kind));
        }
        str_from = j; // }...
        i = j + 1;
    }
    if end > str_from {
        out.push(Token::new(str_from, end, TokenType::String));
    }
    out
}

/// Tokens for one line of Python, and the state at its end.
pub fn tokenize_line(line: &str, state: LineState) -> (Vec<Token>, LineState) {
    let text: Vec<char> = line.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;

    // Continue a triple-quoted string from the previous line.
    let open = match state {
        LineState::TripleDouble => Some("\"\"\""),
        LineState::TripleSingle => Some("'''"),
        _ => None,
    };
    if let Some(quote) = open {
        match find_string_end(&text, 0, quote) {
            None => {
                if !text.is_empty() {
                    tokens.push(Token::new(0, text.len(), TokenType::String));
                }
                return (tokens, state);
            }
            Some(end) => {
                tokens.push(Token::new(0, end, TokenType::String));
                i = end;
            }
        }
    }

    let mut prev_word = String::new(); // last identifier/keyword seen, to spot names after def/class
    let mut after_dot = false; // obj.name: name is an attribute
    let mut only_space_so_far = text[..i].iter().all(|c| c.is_whitespace());

    while i < text.len() {
        let ch = text[i];
        if ch == ' ' || ch == '\t' {
            i += 1;
            continue;
        }
        if ch == '#' {
            tokens.push(Token::new(i, text.len(), TokenType::Comment));
            break;
        }

        // Strings (with optional prefix like f, r, b, rb)
        if let Some(prefix_len) = string_prefix(&text, i) {
            let q_start = i + prefix_len;
            let triple = if starts_with_at(&text, q_start, "\"\"\"") {
                Some("\"\"\"")
            } else if starts_with_at(&text, q_start, "'''") {
                Some("'''")
            } else {
                None
            };
            let quote: String = triple.map_or_else(|| text[q_start].to_string(), str::to_owned);
            let quote_len = quote.chars().count();
            match find_string_end(&text, q_start + quote_len, &quote) {
                None => {
                    tokens.push(Token::new(i, text.len(), TokenType::String));
                    let end_state = match triple {
                        Some("\"\"\"") => LineState::TripleDouble,
                        Some(_) => LineState::TripleSingle,
                        None => LineState::Normal,
                    };
                    return (tokens, end_state);
                }
                Some(end) => {
                    let is_f = text[i..q_start].iter().any(|c| *c == 'f' || *c == 'F');
                    if is_f {
                        tokens.extend(f_string_tokens(&text, i, end, q_start + quote_len, end - quote_len));
                    } else {
                        tokens.push(Token::new(i, end, TokenType::String));
                    }
                    i = end;
                    prev_word.clear();
                    only_space_so_far = false;
                    continue;
                }
            }
        }

        // Decorators: only at the start of a line
        if ch == '@' && only_space_so_far {
            let len = decorator_len(&text, i);
            if len > 0 {
                tokens.push(Token::new(i, i + len, TokenType::Decorator));
                i += len;
                only_space_so_far = false;
                continue;
            }
        }

        // Numbers (a leading "." only counts if a digit follows)
        if ch.is_ascii_digit() || (ch == '.' && text.get(i + 1).is_some_and(char::is_ascii_digit)) {
            let rest: String = text[i..].iter().collect();
            if let Some(m) = NUMBER.find(&rest) {
                if !m.as_str().is_empty() {
                    let len = m.as_str().chars().count();
                    tokens.push(Token::new(i, i + len, TokenType::Number));
                    i += len;
                    prev_word.clear();
                    only_space_so_far = false;
                    continue;
                }
            }
        }

        // Identifiers and keywords
        let len = ident_len(&text, i);
        if len > 0 {
            let end = i + len;
            let word: String = text[i..end].iter().collect();
            let is_call = {
                let mut k = end;
                while k < text.len() && text[k].is_whitespace() {
                    k += 1;
                }
                text.get(k) == Some(&'(')
            };
            let kind = if after_dot {
                is_call.then_some(TokenType::Call)
            } else if prev_word == "def" {
                Some(TokenType::Function)
            } else if prev_word == "class" {
                Some(TokenType::Class)
            } else if KEYWORDS.contains(&word.as_str()) {
                Some(TokenType::Keyword)
            } else if CONSTANTS.contains(&word.as_str()) {
                Some(TokenType::Constant)
            } else if word == "self" || word == "cls" {
                Some(TokenType::SelfParam)
            } else if BUILTINS.contains(&word.as_str()) {
                Some(TokenType::Builtin)
            } else if is_call {
                Some(TokenType::Call)
            } else {
                None
            };
            if let Some(kind) = kind {
                tokens.push(Token::new(i, end, kind));
            }
            prev_word = word;
            after_dot = false;
            i = end;
            only_space_so_far = false;
            continue;
        }

        // Anything else (operators, brackets, dots...) stays plain.
        i += 1;
        after_dot = ch == '.';
        prev_word.clear();
        only_space_so_far = false;
    }
    (tokens, LineState::Normal)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tokenize and return (text, type) pairs, which are easy to read in failures.
    fn toks(line: &str, state: LineState) -> Vec<(String, &'static str)> {
        let chars: Vec<char> = line.chars().collect();
        tokenize_line(line, state).0.iter().map(|t| (chars[t.start..t.end].iter().collect(), t.kind.as_str())).collect()
    }

    fn t(line: &str) -> Vec<(String, &'static str)> {
        toks(line, LineState::Normal)
    }

    fn pairs(list: &[(&str, &'static str)]) -> Vec<(String, &'static str)> {
        list.iter().map(|(a, b)| ((*a).to_owned(), *b)).collect()
    }

    #[test]
    fn labels_keywords_function_names_strings_and_comments() {
        assert_eq!(
            t("def foo(x=\"hi\"):  # comment"),
            pairs(&[("def", "keyword"), ("foo", "function"), ("\"hi\"", "string"), ("# comment", "comment")])
        );
    }

    #[test]
    fn labels_class_names_self_constants_and_builtins() {
        assert_eq!(t("class Dog(Base):"), pairs(&[("class", "keyword"), ("Dog", "class")]));
        assert_eq!(
            t("self.x = None or True"),
            pairs(&[("self", "self"), ("None", "constant"), ("or", "keyword"), ("True", "constant")])
        );
        assert_eq!(t("print(len(items))"), pairs(&[("print", "builtin"), ("len", "builtin")]));
    }

    #[test]
    fn labels_calls_to_your_own_functions() {
        assert_eq!(t("result = compute (a, b)"), pairs(&[("compute", "call")]));
    }

    #[test]
    fn treats_names_after_a_dot_as_attributes() {
        assert_eq!(t("obj.list + os.open(p) + self.type"), pairs(&[("open", "call"), ("self", "self")]));
    }

    #[test]
    fn labels_numbers_in_their_many_forms() {
        assert_eq!(
            t("a = 42 + 3.14 + 1_000 + 0xFF + 1e-3 + 2j + .5"),
            pairs(&[
                ("42", "number"),
                ("3.14", "number"),
                ("1_000", "number"),
                ("0xFF", "number"),
                ("1e-3", "number"),
                ("2j", "number"),
                (".5", "number")
            ])
        );
    }

    #[test]
    fn does_not_treat_digits_inside_names_as_numbers() {
        assert_eq!(t("x1 = var2"), vec![]);
    }

    #[test]
    fn handles_string_prefixes_and_escaped_quotes() {
        assert_eq!(
            t(r#"f"hi {name}" + rb'\d' + "say \"hi\"""#),
            pairs(&[("f\"hi {", "string"), ("}\"", "string"), (r"rb'\d'", "string"), (r#""say \"hi\"""#, "string")])
        );
    }

    #[test]
    fn does_not_start_a_comment_inside_a_string() {
        assert_eq!(t("url = \"a#b\"  # real"), pairs(&[("\"a#b\"", "string"), ("# real", "comment")]));
    }

    #[test]
    fn labels_decorators_including_dotted_ones() {
        assert_eq!(t("@app.get(\"/users\")"), pairs(&[("@app.get", "decorator"), ("\"/users\"", "string")]));
        assert_eq!(t("    @staticmethod"), pairs(&[("@staticmethod", "decorator")]));
    }

    #[test]
    fn an_unterminated_single_quoted_string_runs_to_the_end_of_the_line() {
        assert_eq!(t("x = \"oops"), pairs(&[("\"oops", "string")]));
        assert_eq!(tokenize_line("x = \"oops", LineState::Normal).1, LineState::Normal);
    }

    #[test]
    fn a_triple_quoted_string_closed_on_the_same_line_is_one_token() {
        assert_eq!(t("doc = \"\"\"hello\"\"\""), pairs(&[("\"\"\"hello\"\"\"", "string")]));
    }

    #[test]
    fn an_unclosed_triple_quote_carries_state_to_the_next_line() {
        let first = tokenize_line("def f():  \"\"\"Start", LineState::Normal);
        assert_eq!(first.1, LineState::TripleDouble);
        let middle = tokenize_line("  still a docstring # not a comment", LineState::TripleDouble);
        assert_eq!(middle.0, vec![Token::new(0, 35, TokenType::String)]);
        assert_eq!(middle.1, LineState::TripleDouble);
        assert_eq!(toks("end\"\"\" + x", LineState::TripleDouble), pairs(&[("end\"\"\"", "string")]));
        assert_eq!(tokenize_line("end\"\"\" + x", LineState::TripleDouble).1, LineState::Normal);
    }

    #[test]
    fn single_triple_quotes_do_not_close_a_double_triple_string() {
        assert_eq!(tokenize_line("it's ''' here", LineState::TripleDouble).1, LineState::TripleDouble);
    }

    #[test]
    fn handles_escaped_quotes_inside_a_triple_quoted_string() {
        assert_eq!(tokenize_line("a \\\"\"\" b", LineState::TripleDouble).1, LineState::TripleDouble);
    }

    #[test]
    fn f_strings_highlight_expressions_as_code() {
        assert_eq!(t("s = f\"hi {name}!\""), pairs(&[("f\"hi {", "string"), ("}!\"", "string")]));
        assert_eq!(
            t("f\"{len(items)} items\""),
            pairs(&[("f\"{", "string"), ("len", "builtin"), ("} items\"", "string")])
        );
        assert_eq!(
            t("f'{x:.2f} {self.y}'"),
            pairs(&[("f'{", "string"), (".2", "number"), ("} {", "string"), ("self", "self"), ("}'", "string")])
        );
    }

    #[test]
    fn f_string_double_braces_are_plain_and_normal_strings_are_untouched() {
        assert_eq!(t("f\"{{literal}}\""), pairs(&[("f\"{{literal}}\"", "string")]));
        assert_eq!(t("\"{not code}\""), pairs(&[("\"{not code}\"", "string")]));
    }

    #[test]
    fn f_string_with_strings_inside_the_expression() {
        assert_eq!(t("f\"{d['k']}\""), pairs(&[("f\"{", "string"), ("'k'", "string"), ("}\"", "string")]));
    }

    #[test]
    fn handles_non_ascii_names() {
        assert_eq!(t("def grüße():"), pairs(&[("def", "keyword"), ("grüße", "function")]));
    }
}
