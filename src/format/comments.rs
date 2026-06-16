//! Comment recovery for the formatter.
//!
//! The lexer drops comments, so the formatter rescans the raw source to
//! recover them with their byte offsets. Each comment is classified as
//! own-line (nothing but whitespace precedes it on its line) or trailing
//! (it follows code on the same line). The formatter weaves them back in
//! by position. String and char literals are skipped so a `//` inside a
//! string is not mistaken for a comment.

/// A recovered source comment.
#[derive(Debug, Clone)]
pub struct Comment {
    /// Byte offset of the `//` or `/*`.
    pub start: usize,
    /// Canonical comment text (no trailing whitespace; line comments keep
    /// their `//` prefix, block comments their `/* */`).
    pub text: String,
    /// True when only whitespace precedes the comment on its line.
    pub own_line: bool,
}

/// Scan `src` for all comments, in source order.
pub fn scan(src: &str) -> Vec<Comment> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    // Track whether the current line has seen non-whitespace before the
    // comment, to classify own-line vs trailing.
    let mut line_has_code = false;
    let n = bytes.len();
    while i < n {
        let b = bytes[i];
        match b {
            b'\n' => {
                line_has_code = false;
                i += 1;
            }
            b' ' | b'\t' | b'\r' => {
                i += 1;
            }
            b'"' => {
                line_has_code = true;
                i = skip_string(bytes, i);
            }
            b'\'' => {
                line_has_code = true;
                i = skip_char(bytes, i);
            }
            b'/' if i + 1 < n && bytes[i + 1] == b'/' => {
                let start = i;
                let own_line = !line_has_code;
                let mut j = i + 2;
                while j < n && bytes[j] != b'\n' {
                    j += 1;
                }
                let raw = &src[start..j];
                out.push(Comment {
                    start,
                    text: raw.trim_end().to_string(),
                    own_line,
                });
                i = j;
                line_has_code = true;
            }
            b'/' if i + 1 < n && bytes[i + 1] == b'*' => {
                let start = i;
                let own_line = !line_has_code;
                let mut j = i + 2;
                while j + 1 < n && !(bytes[j] == b'*' && bytes[j + 1] == b'/') {
                    j += 1;
                }
                j = (j + 2).min(n);
                let raw = &src[start..j];
                out.push(Comment {
                    start,
                    text: raw.to_string(),
                    own_line,
                });
                // A block comment may keep the line "in code" if more follows.
                line_has_code = true;
                i = j;
            }
            _ => {
                line_has_code = true;
                i += 1;
            }
        }
    }
    out
}

/// Skip a `"..."` string literal starting at the opening quote. Returns the
/// index just past the closing quote (or end of input). Triple-quoted block
/// strings are handled too.
fn skip_string(bytes: &[u8], start: usize) -> usize {
    let n = bytes.len();
    // Triple quote?
    if start + 2 < n && bytes[start + 1] == b'"' && bytes[start + 2] == b'"' {
        let mut j = start + 3;
        while j + 2 < n && !(bytes[j] == b'"' && bytes[j + 1] == b'"' && bytes[j + 2] == b'"') {
            j += 1;
        }
        return (j + 3).min(n);
    }
    let mut j = start + 1;
    while j < n {
        match bytes[j] {
            b'\\' => j += 2,
            b'"' => return j + 1,
            b'\n' => return j,
            // A `${ ... }` interpolation: skip the balanced braces (respecting
            // strings and chars nested in the expression) so an inner `"` does
            // not terminate the outer literal and throw off comment scanning.
            b'$' if j + 1 < n && bytes[j + 1] == b'{' => {
                j = skip_interpolation(bytes, j + 1);
            }
            _ => j += 1,
        }
    }
    j
}

/// Skip a `${ ... }` interpolation body. `brace` is the index of the opening
/// `{`; returns the index just past the matching `}`.
fn skip_interpolation(bytes: &[u8], brace: usize) -> usize {
    let n = bytes.len();
    let mut j = brace + 1;
    let mut depth = 1u32;
    while j < n && depth > 0 {
        match bytes[j] {
            b'{' => {
                depth += 1;
                j += 1;
            }
            b'}' => {
                depth -= 1;
                j += 1;
            }
            b'"' => j = skip_string(bytes, j),
            b'\'' => j = skip_char(bytes, j),
            _ => j += 1,
        }
    }
    j
}

/// Skip a `'c'` char literal starting at the opening quote.
fn skip_char(bytes: &[u8], start: usize) -> usize {
    let n = bytes.len();
    let mut j = start + 1;
    while j < n {
        match bytes[j] {
            b'\\' => j += 2,
            b'\'' => return j + 1,
            b'\n' => return j,
            _ => j += 1,
        }
    }
    j
}
