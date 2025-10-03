/// Represents a position in source code
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub line: usize,
    pub column: usize,
    pub offset: usize,
    pub length: usize,
}

impl Span {
    pub fn new(line: usize, column: usize, offset: usize, length: usize) -> Self {
        Span {
            line,
            column,
            offset,
            length,
        }
    }
    
    pub fn dummy() -> Self {
        Span {
            line: 0,
            column: 0,
            offset: 0,
            length: 0,
        }
    }
    
    /// Combine two spans into one spanning both
    pub fn merge(&self, other: &Span) -> Span {
        let start = self.offset.min(other.offset);
        let end = (self.offset + self.length).max(other.offset + other.length);
        
        Span {
            line: self.line.min(other.line),
            column: if self.line == other.line {
                self.column.min(other.column)
            } else {
                self.column
            },
            offset: start,
            length: end - start,
        }
    }
}

impl Default for Span {
    fn default() -> Self {
        Self::dummy()
    }
}

/// Wraps a value with its source location
#[derive(Debug, Clone)]
pub struct Spanned<T> {
    pub value: T,
    pub span: Span,
}

impl<T> Spanned<T> {
    pub fn new(value: T, span: Span) -> Self {
        Spanned { value, span }
    }
}

