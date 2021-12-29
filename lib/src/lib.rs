#![recursion_limit = "1024"]
mod lints;
mod make;
pub mod session;
mod utils;

pub use lints::LINTS;
use session::SessionInfo;

use rnix::{parser::ParseError, SyntaxElement, SyntaxKind, TextRange};
use std::{convert::Into, default::Default};

#[cfg(feature = "json-out")]
use serde::{
    ser::{SerializeStruct, Serializer},
    Serialize,
};

#[derive(Debug)]
#[cfg_attr(feature = "json-out", derive(Serialize))]
pub enum Severity {
    Warn,
    Error,
    Hint,
}

impl Default for Severity {
    fn default() -> Self {
        Self::Warn
    }
}

/// Report generated by a lint
#[derive(Debug, Default)]
#[cfg_attr(feature = "json-out", derive(Serialize))]
pub struct Report {
    /// General information about this lint and where it applies.
    pub note: &'static str,
    /// An error code to uniquely identify this lint
    pub code: u32,
    /// Report severity level
    pub severity: Severity,
    /// Collection of diagnostics raised by this lint
    pub diagnostics: Vec<Diagnostic>,
}

impl Report {
    /// Construct a report. Do not invoke `Report::new` manually, see `lint` macro
    pub fn new(note: &'static str, code: u32) -> Self {
        Self {
            note,
            code,
            ..Default::default()
        }
    }
    /// Add a diagnostic to this report
    pub fn diagnostic<S: AsRef<str>>(mut self, at: TextRange, message: S) -> Self {
        self.diagnostics.push(Diagnostic::new(at, message));
        self
    }
    /// Add a diagnostic with a fix to this report
    pub fn suggest<S: AsRef<str>>(
        mut self,
        at: TextRange,
        message: S,
        suggestion: Suggestion,
    ) -> Self {
        self.diagnostics
            .push(Diagnostic::suggest(at, message, suggestion));
        self
    }
    /// Set severity level
    pub fn severity(mut self, severity: Severity) -> Self {
        self.severity = severity;
        self
    }
    /// A range that encompasses all the suggestions provided in this report
    pub fn total_suggestion_range(&self) -> Option<TextRange> {
        self.diagnostics
            .iter()
            .flat_map(|d| Some(d.suggestion.as_ref()?.at))
            .reduce(|acc, next| acc.cover(next))
    }
    /// A range that encompasses all the diagnostics provided in this report
    pub fn total_diagnostic_range(&self) -> Option<TextRange> {
        self.diagnostics
            .iter()
            .flat_map(|d| Some(d.at))
            .reduce(|acc, next| acc.cover(next))
    }
    /// Unsafe but handy replacement for above
    pub fn range(&self) -> TextRange {
        self.total_suggestion_range().unwrap()
    }
    /// Apply all diagnostics. Assumption: diagnostics do not overlap
    pub fn apply(&self, src: &mut String) {
        for d in self.diagnostics.iter() {
            d.apply(src);
        }
    }
    /// Create a report out of a parse error
    pub fn from_parse_err(err: ParseError) -> Self {
        let at = match err {
            ParseError::Unexpected(at) => at,
            ParseError::UnexpectedExtra(at) => at,
            ParseError::UnexpectedWanted(_, at, _) => at,
            ParseError::UnexpectedDoubleBind(at) => at,
            ParseError::UnexpectedEOF | ParseError::UnexpectedEOFWanted(_) => {
                TextRange::empty(0u32.into())
            }
            _ => panic!("report a bug, pepper forgot to handle a parse error"),
        };
        let mut message = err.to_string();
        message
            .as_mut_str()
            .get_mut(0..1)
            .unwrap()
            .make_ascii_uppercase();
        Self::new("Syntax error", 0)
            .diagnostic(at, message)
            .severity(Severity::Error)
    }
}

/// Mapping from a bytespan to an error message.
/// Can optionally suggest a fix.
#[derive(Debug)]
pub struct Diagnostic {
    pub at: TextRange,
    pub message: String,
    pub suggestion: Option<Suggestion>,
}

impl Diagnostic {
    /// Construct a diagnostic.
    pub fn new<S: AsRef<str>>(at: TextRange, message: S) -> Self {
        Self {
            at,
            message: message.as_ref().into(),
            suggestion: None,
        }
    }
    /// Construct a diagnostic with a fix.
    pub fn suggest<S: AsRef<str>>(at: TextRange, message: S, suggestion: Suggestion) -> Self {
        Self {
            at,
            message: message.as_ref().into(),
            suggestion: Some(suggestion),
        }
    }
    /// Apply a diagnostic to a source file
    pub fn apply(&self, src: &mut String) {
        if let Some(s) = &self.suggestion {
            s.apply(src);
        }
    }
}

#[cfg(feature = "json-out")]
impl Serialize for Diagnostic {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut s = serializer.serialize_struct("Diagnostic", 3)?;
        let at = {
            let start = usize::from(self.at.start());
            let end = usize::from(self.at.end());
            (start, end)
        };
        s.serialize_field("at", &at)?;
        s.serialize_field("message", &self.message)?;
        if let Some(suggestion) = &self.suggestion {
            s.serialize_field("suggestion", suggestion)?;
        }
        s.end()
    }
}

/// Suggested fix for a diagnostic, the fix is provided as a syntax element.
/// Look at `make.rs` to construct fixes.
#[derive(Debug)]
pub struct Suggestion {
    pub at: TextRange,
    pub fix: SyntaxElement,
}

impl Suggestion {
    /// Construct a suggestion.
    pub fn new<E: Into<SyntaxElement>>(at: TextRange, fix: E) -> Self {
        Self {
            at,
            fix: fix.into(),
        }
    }
    /// Apply a suggestion to a source file
    pub fn apply(&self, src: &mut String) {
        let start = usize::from(self.at.start());
        let end = usize::from(self.at.end());
        src.replace_range(start..end, &self.fix.to_string())
    }
}

#[cfg(feature = "json-out")]
impl Serialize for Suggestion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut s = serializer.serialize_struct("Suggestion", 2)?;
        let at = {
            let start = usize::from(self.at.start());
            let end = usize::from(self.at.end());
            (start, end)
        };
        let fix = self.fix.to_string();
        s.serialize_field("at", &at)?;
        s.serialize_field("fix", &fix)?;
        s.end()
    }
}

/// Lint logic is defined via this trait. Do not implement manually,
/// look at the `lint` attribute macro instead for implementing rules
pub trait Rule {
    fn validate(&self, node: &SyntaxElement, sess: &SessionInfo) -> Option<Report>;
}

/// Contains information about the lint itself. Do not implement manually,
/// look at the `lint` attribute macro instead for implementing rules
pub trait Metadata {
    fn name(&self) -> &'static str;
    fn note(&self) -> &'static str;
    fn code(&self) -> u32;
    fn report(&self) -> Report;
    fn match_with(&self, with: &SyntaxKind) -> bool;
    fn match_kind(&self) -> Vec<SyntaxKind>;
}

/// Contains offline explanation for each lint
/// The `lint` macro scans nearby doc comments for
/// explanations and derives this trait.
///
/// FIXME: the lint macro does way too much, maybe
/// split it into smaller macros.
pub trait Explain {
    fn explanation(&self) -> &'static str {
        "no explanation found"
    }
}

/// Combines Rule and Metadata, do not implement manually, this is derived by
/// the `lint` macro.
pub trait Lint: Metadata + Explain + Rule + Send + Sync {}

/// Helper utility to take lints from modules and insert them into a map for efficient
/// access. Mapping is from a SyntaxKind to a list of lints that apply on that Kind.
///
/// See `lints.rs` for usage.
#[macro_export]
macro_rules! lints {
    ($($s:ident),*,) => {
        lints!($($s),*);
    };
    ($($s:ident),*) => {
        $(
            mod $s;
        )*
        ::lazy_static::lazy_static! {
            pub static ref LINTS: Vec<&'static Box<dyn $crate::Lint>> = {
                let mut v = Vec::new();
                $(
                    {
                        let temp_lint = &*$s::LINT;
                        v.push(temp_lint);
                    }
                )*
                v
            };
        }
    }
}
