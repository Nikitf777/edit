use std::ops::Range;

use crate::arena::{Arena, scratch_arena};
use crate::document::ReadableDocument;
use crate::helpers::*;
use crate::unicode;

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    #[default]
    Other,
    Comment,
    Keyword,
    Operator,
    Number,
    String,
    Variable,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Token {
    pub range: Range<usize>,
    pub kind: TokenKind,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct State {}

#[derive(Default, Clone, Copy)]
pub enum Test {
    #[default]
    Always,
    LineEnd,
    Prefix(&'static str),
    Word(&'static str),
    NonAlpha,
    NonDigit,
}

#[derive(Default, Clone, Copy)]
pub struct Transition {
    enter: Test,
    exit: Test,
    kind: TokenKind,
}

const POWERSHELL: &[&[Transition]] = &[
    // Ground state
    &[
        // Comments
        Transition { enter: Test::Prefix("#"), exit: Test::LineEnd, kind: TokenKind::Comment },
        Transition {
            enter: Test::Prefix("<#"),
            exit: Test::Prefix("#>"),
            kind: TokenKind::Comment,
        },
        // Keywords
        Transition { enter: Test::Word("break"), exit: Test::Always, kind: TokenKind::Keyword },
        Transition { enter: Test::Word("catch"), exit: Test::Always, kind: TokenKind::Keyword },
        Transition { enter: Test::Word("continue"), exit: Test::Always, kind: TokenKind::Keyword },
        Transition { enter: Test::Word("do"), exit: Test::Always, kind: TokenKind::Keyword },
        Transition { enter: Test::Word("else"), exit: Test::Always, kind: TokenKind::Keyword },
        Transition { enter: Test::Word("finally"), exit: Test::Always, kind: TokenKind::Keyword },
        Transition { enter: Test::Word("foreach"), exit: Test::Always, kind: TokenKind::Keyword },
        Transition { enter: Test::Word("function"), exit: Test::Always, kind: TokenKind::Keyword },
        Transition { enter: Test::Word("if"), exit: Test::Always, kind: TokenKind::Keyword },
        Transition { enter: Test::Word("return"), exit: Test::Always, kind: TokenKind::Keyword },
        Transition { enter: Test::Word("switch"), exit: Test::Always, kind: TokenKind::Keyword },
        Transition { enter: Test::Word("throw"), exit: Test::Always, kind: TokenKind::Keyword },
        Transition { enter: Test::Word("try"), exit: Test::Always, kind: TokenKind::Keyword },
        Transition { enter: Test::Word("using"), exit: Test::Always, kind: TokenKind::Keyword },
        Transition { enter: Test::Word("while"), exit: Test::Always, kind: TokenKind::Keyword },
        // Operators
        Transition { enter: Test::Prefix("=="), exit: Test::Always, kind: TokenKind::Operator },
        Transition { enter: Test::Prefix("!="), exit: Test::Always, kind: TokenKind::Operator },
        Transition { enter: Test::Prefix("&&"), exit: Test::Always, kind: TokenKind::Operator },
        Transition { enter: Test::Prefix("||"), exit: Test::Always, kind: TokenKind::Operator },
        Transition { enter: Test::Prefix("<="), exit: Test::Always, kind: TokenKind::Operator },
        Transition { enter: Test::Prefix(">="), exit: Test::Always, kind: TokenKind::Operator },
        Transition { enter: Test::Prefix("++"), exit: Test::Always, kind: TokenKind::Operator },
        Transition { enter: Test::Prefix("--"), exit: Test::Always, kind: TokenKind::Operator },
        Transition { enter: Test::Prefix("="), exit: Test::Always, kind: TokenKind::Operator },
        Transition { enter: Test::Prefix("<"), exit: Test::Always, kind: TokenKind::Operator },
        Transition { enter: Test::Prefix(">"), exit: Test::Always, kind: TokenKind::Operator },
        Transition { enter: Test::Prefix("+"), exit: Test::Always, kind: TokenKind::Operator },
        Transition { enter: Test::Prefix("-"), exit: Test::Always, kind: TokenKind::Operator },
        Transition { enter: Test::Prefix("*"), exit: Test::Always, kind: TokenKind::Operator },
        Transition { enter: Test::Prefix("/"), exit: Test::Always, kind: TokenKind::Operator },
        Transition { enter: Test::Prefix("%"), exit: Test::Always, kind: TokenKind::Operator },
        Transition { enter: Test::Prefix("!"), exit: Test::Always, kind: TokenKind::Operator },
        // Numbers
        // Strings
        Transition { enter: Test::Prefix("'"), exit: Test::Prefix("'"), kind: TokenKind::String },
        Transition { enter: Test::Prefix("\""), exit: Test::Prefix("\""), kind: TokenKind::String },
        // Variables
        Transition { enter: Test::Prefix("$"), exit: Test::NonAlpha, kind: TokenKind::Variable },
    ],
];

pub struct Parser<'a> {
    doc: &'a dyn ReadableDocument,
    offset: usize,
    logical_pos_y: CoordType,
    state: Transition,
}

impl<'doc> Parser<'doc> {
    pub fn new(doc: &'doc dyn ReadableDocument, state: Transition) -> Self {
        Self { doc, offset: 0, logical_pos_y: 0, state }
    }

    pub fn logical_pos_y(&self) -> CoordType {
        self.logical_pos_y
    }

    pub fn parse_next_line<'a>(&mut self, arena: &'a Arena) -> Vec<Token, &'a Arena> {
        let scratch = scratch_arena(Some(arena));
        let line_offset = self.offset;
        let mut line_buf = Vec::new_in(&*scratch);
        let mut res = Vec::new_in(arena);

        // Accumulate a line of text into `line_buf`.
        {
            let mut chunk = self.doc.read_forward(self.offset);

            // Check if the last line was the last line in the document.
            if chunk.is_empty() {
                return res;
            }

            if self.offset != 0 {
                self.logical_pos_y += 1;
            }

            loop {
                let (off, line) = unicode::newlines_forward(chunk, 0, 0, 1);
                self.offset += off;

                // Overly long lines are not highlighted, so we limit the line length to 32 KiB.
                // I'm worried it may run into weird edge cases.
                let end = off.min(MEBI - line_buf.len());
                // If we're at it we can also help Rust understand that indexing with `end` doesn't panic.
                let end = end.min(chunk.len());

                line_buf.extend_from_slice(&chunk[..end]);

                // Start of the next line found.
                if line == 1 {
                    break;
                }

                chunk = self.doc.read_forward(self.offset);
                if chunk.is_empty() {
                    // End of document reached
                    break;
                }
            }
        }

        // If the line is too long, we don't highlight it.
        // This is to prevent performance issues with very long lines.
        if line_buf.len() > MEBI {
            self.state = Transition::default();
            return res;
        }

        let line_buf = unicode::strip_newline(&line_buf);
        let mut off_end = 0;

        'outer: loop {
            let mut off_beg = off_end;

            if matches!(self.state.enter, Test::Always) {
                'inner: loop {
                    while off_end < line_buf.len() && line_buf[off_end].is_ascii_whitespace() {
                        off_end += 1;
                    }
                    if off_end >= line_buf.len() {
                        break 'outer;
                    }

                    off_beg = off_end;

                    for t in POWERSHELL[0] {
                        match t.enter {
                            Test::Always => {
                                self.state = *t;
                                break 'inner;
                            }
                            Test::Prefix(prefix) => {
                                if line_buf[off_end..].starts_with(prefix.as_bytes()) {
                                    off_end += prefix.len();
                                    self.state = *t;
                                    break 'inner;
                                }
                            }
                            _ => {}
                        }
                    }

                    while off_end < line_buf.len() && !line_buf[off_end].is_ascii_whitespace() {
                        off_end += 1;
                    }
                    if off_end >= line_buf.len() {
                        break 'inner;
                    }
                }
            }

            match self.state.exit {
                Test::Always => self.state = Transition::default(),
                Test::LineEnd => {
                    off_end = line_buf.len();
                    self.state = Transition::default();
                }
                Test::Prefix(prefix) => loop {
                    while off_end < line_buf.len() && line_buf[off_end].is_ascii_whitespace() {
                        off_end += 1;
                    }

                    if line_buf[off_end..].starts_with(prefix.as_bytes()) {
                        self.state = Transition::default();
                        off_end += prefix.len();
                        break;
                    }

                    while off_end < line_buf.len() && !line_buf[off_end].is_ascii_whitespace() {
                        off_end += 1;
                    }
                    if off_end >= line_buf.len() {
                        break;
                    }
                },
                Test::NonAlpha => {
                    while off_end < line_buf.len()
                        && (line_buf[off_end].is_ascii_alphanumeric() || line_buf[off_end] >= 0x80)
                    {
                        off_end += 1;
                    }
                    self.state = Transition::default();
                }
                Test::NonDigit => {
                    while off_end < line_buf.len() && line_buf[off_end].is_ascii_digit() {
                        off_end += 1;
                    }
                    self.state = Transition::default();
                }
            }

            res.push(Token {
                range: line_offset + off_beg..line_offset + off_end,
                kind: self.state.kind,
            });

            if off_end >= line_buf.len() {
                break 'outer;
            }
        }

        res
    }
}
