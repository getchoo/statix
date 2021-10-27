mod lints;
mod make;

pub use lints::LINTS;

use rnix::{SyntaxElement, SyntaxKind, TextRange};
use std::{convert::Into, default::Default};

#[cfg(feature = "json-out")]
use serde::{
    ser::{SerializeStruct, Serializer},
    Serialize,
};

/// Report generated by a lint
#[derive(Debug, Default)]
#[cfg_attr(feature = "json-out", derive(Serialize))]
pub struct Report {
    /// General information about this lint and where it applies.
    pub note: &'static str,
    /// An error code to uniquely identify this lint
    pub code: u32,
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
    fn validate(&self, node: &SyntaxElement) -> Option<Report>;
}

/// Contains information about the lint itself. Do not implement manually,
/// look at the `lint` attribute macro instead for implementing rules
pub trait Metadata {
    fn name() -> &'static str
    where
        Self: Sized;
    fn note() -> &'static str
    where
        Self: Sized;
    fn code() -> u32
    where
        Self: Sized;
    fn report() -> Report
    where
        Self: Sized;
    fn match_with(&self, with: &SyntaxKind) -> bool;
    fn match_kind(&self) -> Vec<SyntaxKind>;
}

/// Combines Rule and Metadata, do not implement manually, this is derived by
/// the `lint` macro.
pub trait Lint: Metadata + Rule + Send + Sync {}

/// Helper utility to take lints from modules and insert them into a map for efficient
/// access. Mapping is from a SyntaxKind to a list of lints that apply on that Kind.
///
/// See `lints.rs` for usage.
#[macro_export]
macro_rules! lint_map {
    ($($s:ident),*,) => {
        lint_map!($($s),*);
    };
    ($($s:ident),*) => {
        use ::std::collections::HashMap;
        use ::rnix::SyntaxKind;
        $(
            mod $s;
        )*
        ::lazy_static::lazy_static! {
            pub static ref LINTS: HashMap<SyntaxKind, Vec<&'static Box<dyn $crate::Lint>>> = {
                let mut map = HashMap::new();
                $(
                    {
                        let temp_lint = &*$s::LINT;
                        let temp_matches = temp_lint.match_kind();
                        for temp_match in temp_matches {
                            map.entry(temp_match)
                               .and_modify(|v: &mut Vec<_>| v.push(temp_lint))
                               .or_insert_with(|| vec![temp_lint]);
                        }
                    }
                )*
                map
            };
        }
    }
}
