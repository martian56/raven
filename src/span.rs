//! Source spans for the Raven compiler.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

/// A half open byte range `[start, end)` inside a source file. `line`/`col` are
/// 1 indexed and refer to the span's start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub file: Arc<PathBuf>,
    pub start: usize,
    pub end: usize,
    pub line: u32,
    pub col: u32,
}

static NEXT_COPY: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
const COPY_SHIFT: u32 = 40;

/// A fresh id for a compiler-made copy of source nodes.
/// Copy id of the `x[i] op v` value that `x[i] op= v` on a class builds. It
/// is fixed so the type checker and HIR lowering derive the same span.
pub const COMPOUND_INDEX_COPY: usize = (1 << 23) + 1;

pub fn next_copy_id() -> usize {
    NEXT_COPY.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl Span {
    /// This span as part of copy `id`. Later passes key on byte offsets, so a
    /// copied node needs offsets no other node has; the copy id replaces the
    /// high bits, which keeps nested copies distinct.
    /// The copy id this span belongs to; 0 for a span from source.
    pub fn copy_id(&self) -> usize {
        self.start >> COPY_SHIFT
    }

    pub fn in_copy(&self, id: usize) -> Span {
        let low = (1usize << COPY_SHIFT) - 1;
        Span {
            start: (self.start & low) | (id << COPY_SHIFT),
            end: (self.end & low) | (id << COPY_SHIFT),
            ..self.clone()
        }
    }

    pub fn new(file: Arc<PathBuf>, start: usize, end: usize, line: u32, col: u32) -> Self {
        Span {
            file,
            start,
            end,
            line,
            col,
        }
    }

    /// A zero width span, used for EOF tokens and synthetic positions.
    pub fn point(file: Arc<PathBuf>, offset: usize, line: u32, col: u32) -> Self {
        Span {
            file,
            start: offset,
            end: offset,
            line,
            col,
        }
    }

    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(&self) -> bool {
        self.end == self.start
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.file.display(), self.line, self.col)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file() -> Arc<PathBuf> {
        Arc::new(PathBuf::from("test.rv"))
    }

    #[test]
    fn span_len_and_emptiness() {
        let s = Span::new(file(), 3, 7, 1, 4);
        assert_eq!(s.len(), 4);
        assert!(!s.is_empty());

        let p = Span::point(file(), 12, 2, 1);
        assert_eq!(p.len(), 0);
        assert!(p.is_empty());
    }

    #[test]
    fn span_display_formats_as_path_line_col() {
        let s = Span::new(file(), 0, 1, 5, 12);
        assert_eq!(format!("{}", s), "test.rv:5:12");
    }
}
