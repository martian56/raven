use crate::span::Span;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum ErrorType {
    LexError,
    ParseError,
    TypeError,
    RuntimeError,
}

impl fmt::Display for ErrorType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ErrorType::LexError => write!(f, "Lexical Error"),
            ErrorType::ParseError => write!(f, "Parse Error"),
            ErrorType::TypeError => write!(f, "Type Error"),
            ErrorType::RuntimeError => write!(f, "Runtime Error"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RavenError {
    pub error_type: ErrorType,
    pub message: String,
    pub span: Span,
    pub source_code: Option<String>,
    pub filename: Option<String>,
    pub hint: Option<String>,
}

impl RavenError {
    pub fn new(error_type: ErrorType, message: String, span: Span) -> Self {
        RavenError {
            error_type,
            message,
            span,
            source_code: None,
            filename: None,
            hint: None,
        }
    }

    pub fn with_source(mut self, source: String) -> Self {
        self.source_code = Some(source);
        self
    }

    pub fn with_filename(mut self, filename: String) -> Self {
        self.filename = Some(filename);
        self
    }

    pub fn with_hint(mut self, hint: String) -> Self {
        self.hint = Some(hint);
        self
    }

    pub fn format(&self) -> String {
        let mut output = String::new();

        output.push_str(&format!("\x1b[1;31merror\x1b[0m: {}\n", self.message));

        let filename = self.filename.as_deref().unwrap_or("program.rv");
        output.push_str(&format!(
            "  \x1b[1;34m-->\x1b[0m {}:{}:{}\n",
            filename,
            self.span.line + 1,
            self.span.column + 1
        ));

        if let Some(source) = &self.source_code {
            let lines: Vec<&str> = source.lines().collect();

            if self.span.line < lines.len() {
                let line_num = self.span.line + 1;
                let line_num_width = line_num.to_string().len();

                output.push_str(&format!(
                    "   {}\x1b[1;34m|\x1b[0m\n",
                    " ".repeat(line_num_width)
                ));

                output.push_str(&format!(
                    "  \x1b[1;34m{}\x1b[0m \x1b[1;34m|\x1b[0m {}\n",
                    line_num, lines[self.span.line]
                ));

                let padding = " ".repeat(line_num_width);
                let column_padding = " ".repeat(self.span.column);
                let indicator_length = if self.span.length > 0 {
                    self.span.length
                } else {
                    1
                };
                let indicator = "^".repeat(indicator_length);

                output.push_str(&format!(
                    "   {}\x1b[1;34m|\x1b[0m {}\x1b[1;31m{}\x1b[0m\n",
                    padding, column_padding, indicator
                ));
            }
        }

        if let Some(hint) = &self.hint {
            if hint.contains('\n') {
                let mut lines = hint.lines();
                if let Some(first) = lines.next() {
                    output.push_str(&format!("   \x1b[1;36m= help:\x1b[0m {}\n", first));
                }
                for line in lines {
                    output.push_str(&format!("     {}\n", line));
                }
            } else {
                output.push_str(&format!("   \x1b[1;36m= help:\x1b[0m {}\n", hint));
            }
        }

        output
    }
}

impl fmt::Display for RavenError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.format())
    }
}

impl std::error::Error for RavenError {}

// Allow conversion from String for gradual migration
impl From<String> for RavenError {
    fn from(message: String) -> Self {
        RavenError::new(ErrorType::ParseError, message, Span::dummy())
    }
}

impl From<RavenError> for String {
    fn from(error: RavenError) -> Self {
        error.message
    }
}

/// Helper function to create parse errors
pub fn parse_error(message: impl Into<String>, span: Span) -> RavenError {
    RavenError::new(ErrorType::ParseError, message.into(), span)
}

pub fn type_error(message: impl Into<String>, span: Span) -> RavenError {
    RavenError::new(ErrorType::TypeError, message.into(), span)
}

pub fn runtime_error(message: impl Into<String>, span: Span) -> RavenError {
    RavenError::new(ErrorType::RuntimeError, message.into(), span)
}

pub fn lex_error(message: impl Into<String>, span: Span) -> RavenError {
    RavenError::new(ErrorType::LexError, message.into(), span)
}
