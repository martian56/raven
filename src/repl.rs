//! REPL helpers: detect when accumulated source still needs more lines (unclosed delimiters).

/// Count `(`, `{`, and `[` outside of strings and `//` / `/* */` comments.
/// Used to offer multi-line input in the interactive REPL until delimiters balance.
pub fn delimiter_depth(source: &str) -> (i32, i32, i32) {
    let mut paren = 0i32;
    let mut brace = 0i32;
    let mut bracket = 0i32;
    let mut it = source.chars().peekable();
    while let Some(c) = it.next() {
        // Line comment
        if c == '/' && it.peek() == Some(&'/') {
            it.next();
            while let Some(x) = it.next() {
                if x == '\n' {
                    break;
                }
            }
            continue;
        }
        // Block comment
        if c == '/' && it.peek() == Some(&'*') {
            it.next();
            while let Some(x) = it.next() {
                if x == '*' && it.peek() == Some(&'/') {
                    it.next();
                    break;
                }
            }
            continue;
        }
        // String literal
        if c == '"' {
            while let Some(x) = it.next() {
                if x == '\\' {
                    it.next();
                    continue;
                }
                if x == '"' {
                    break;
                }
            }
            continue;
        }
        match c {
            '(' => paren += 1,
            ')' => paren -= 1,
            '{' => brace += 1,
            '}' => brace -= 1,
            '[' => bracket += 1,
            ']' => bracket -= 1,
            _ => {}
        }
    }
    (paren, brace, bracket)
}

#[cfg(test)]
mod tests {
    use super::delimiter_depth;

    #[test]
    fn balanced_single_line() {
        assert_eq!(delimiter_depth("print(1);"), (0, 0, 0));
    }

    #[test]
    fn unclosed_paren() {
        assert_eq!(delimiter_depth("print(1"), (1, 0, 0));
    }

    #[test]
    fn unclosed_brace() {
        assert_eq!(delimiter_depth("fun f() -> void {"), (0, 1, 0));
    }

    #[test]
    fn unclosed_bracket() {
        assert_eq!(delimiter_depth("let a: int[] = ["), (0, 0, 1));
    }

    #[test]
    fn ignores_brace_in_string() {
        assert_eq!(delimiter_depth(r#"let s: string = "{";"#), (0, 0, 0));
    }

    #[test]
    fn ignores_line_comment() {
        assert_eq!(delimiter_depth("// {{\n"), (0, 0, 0));
    }
}
