//! coordinate/0 errors (spec section Errors). An error is its code and only
//! the members that code carries, with no free text, so every implementation
//! gives the same bytes for the same input.

use std::fmt;

use evidence_core::json::Value;

/// A coordinate/0 error. [`Error::to_value`] gives the spec's error object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// `P_COORDINATE_DICTIONARY`: the dictionary is invalid. `detail` names
    /// the first check that fails; `core` is evidence/0's code, set only
    /// when the bytes are not strict JSON (step 1).
    Dictionary {
        detail: DictionaryDetail,
        core: Option<evidence_core::Code>,
    },
    /// `P_COORDINATE_SYNTAX`: the coordinate is not a string in canonical
    /// decimal form.
    Syntax,
    /// `P_COORDINATE_RANGE`: the coordinate, or the product encode computes,
    /// is 2^1024 or more.
    Range,
    /// `P_COORDINATE_UNKNOWN_FACET`: a facet id the dictionary does not
    /// hold, the first in list order.
    UnknownFacet { facet: String },
    /// `P_COORDINATE_UNKNOWN_FACTOR`: the coordinate has a prime factor the
    /// dictionary does not hold.
    UnknownFactor,
    /// `P_COORDINATE_CARDINALITY`: a family's rule is broken, the first such
    /// family in the order of `families`.
    Cardinality { family: String },
    /// `P_COORDINATE_DRIFT`: a claimed coordinate is not the one its facets
    /// encode to.
    Drift,
    /// `P_COORDINATE_NOT_EXTENSION`: the new dictionary is not an extension
    /// of the old one.
    NotExtension { detail: ExtensionDetail },
}

impl Error {
    /// The error's code, such as `P_COORDINATE_SYNTAX`.
    pub fn code(&self) -> &'static str {
        match self {
            Error::Dictionary { .. } => "P_COORDINATE_DICTIONARY",
            Error::Syntax => "P_COORDINATE_SYNTAX",
            Error::Range => "P_COORDINATE_RANGE",
            Error::UnknownFacet { .. } => "P_COORDINATE_UNKNOWN_FACET",
            Error::UnknownFactor => "P_COORDINATE_UNKNOWN_FACTOR",
            Error::Cardinality { .. } => "P_COORDINATE_CARDINALITY",
            Error::Drift => "P_COORDINATE_DRIFT",
            Error::NotExtension { .. } => "P_COORDINATE_NOT_EXTENSION",
        }
    }

    /// The error object: `code` and only the members the code carries.
    pub fn to_value(&self) -> Value {
        let mut members = vec![("code".to_string(), string(self.code()))];
        match self {
            Error::Dictionary { detail, core } => {
                members.push(("detail".to_string(), string(detail.as_str())));
                if let Some(core) = core {
                    members.push(("core".to_string(), string(core.as_str())));
                }
            }
            Error::UnknownFacet { facet } => members.push(("facet".to_string(), string(facet))),
            Error::Cardinality { family } => members.push(("family".to_string(), string(family))),
            Error::NotExtension { detail } => {
                members.push(("detail".to_string(), string(detail.as_str())));
            }
            Error::Syntax | Error::Range | Error::UnknownFactor | Error::Drift => {}
        }
        Value::Object(members)
    }

    pub(crate) fn dictionary(detail: DictionaryDetail) -> Error {
        Error::Dictionary { detail, core: None }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())?;
        match self {
            Error::Dictionary { detail, core } => {
                write!(f, " ({detail}")?;
                if let Some(core) = core {
                    write!(f, ", {core}")?;
                }
                f.write_str(")")
            }
            Error::UnknownFacet { facet } => write!(f, " ({facet:?})"),
            Error::Cardinality { family } => write!(f, " ({family})"),
            Error::NotExtension { detail } => write!(f, " ({detail})"),
            Error::Syntax | Error::Range | Error::UnknownFactor | Error::Drift => Ok(()),
        }
    }
}

impl std::error::Error for Error {}

fn string(s: &str) -> Value {
    Value::String(s.to_string())
}

/// The `detail` of `P_COORDINATE_DICTIONARY`: which check of "Loading a
/// dictionary" failed first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DictionaryDetail {
    /// Step 1 (with `core`) or step 2: not strict JSON, or not the
    /// dictionary's shape.
    Shape,
    /// Step 3: `v` is not `coordinate-dictionary/0`.
    Version,
    /// Step 4: `profile` does not match its pattern.
    Profile,
    /// Step 5a: a family name does not match its pattern.
    FamilyName,
    /// Step 5b: a family's `values` is not `one`, `set` or `multiset`.
    Values,
    /// Step 5c: a family name repeats an earlier one.
    FamilyDuplicate,
    /// Step 5d: a family name does not come after the previous one.
    FamilyOrder,
    /// Step 6: `facets` is empty.
    Empty,
    /// Step 7: `facets` has more than 4,096 members.
    TooMany,
    /// Step 8a: a facet id is not `<family>.<name>`.
    IdForm,
    /// Step 8b: a facet id names a family that is not declared.
    UnknownFamily,
    /// Step 8c: a facet id repeats an earlier one.
    IdDuplicate,
    /// Step 8d: a prime is not an integer.
    NotInteger,
    /// Step 8e: a prime is below 2 or not below 2^31.
    PrimeRange,
    /// Step 8f: a prime is not prime.
    NotPrime,
    /// Step 8g: a prime repeats an earlier one.
    PrimeDuplicate,
    /// Step 8h: a prime is not greater than the previous one.
    PrimeOrder,
}

impl DictionaryDetail {
    pub fn as_str(self) -> &'static str {
        match self {
            DictionaryDetail::Shape => "shape",
            DictionaryDetail::Version => "version",
            DictionaryDetail::Profile => "profile",
            DictionaryDetail::FamilyName => "family-name",
            DictionaryDetail::Values => "values",
            DictionaryDetail::FamilyDuplicate => "family-duplicate",
            DictionaryDetail::FamilyOrder => "family-order",
            DictionaryDetail::Empty => "empty",
            DictionaryDetail::TooMany => "too-many",
            DictionaryDetail::IdForm => "id-form",
            DictionaryDetail::UnknownFamily => "unknown-family",
            DictionaryDetail::IdDuplicate => "id-duplicate",
            DictionaryDetail::NotInteger => "not-integer",
            DictionaryDetail::PrimeRange => "prime-range",
            DictionaryDetail::NotPrime => "not-prime",
            DictionaryDetail::PrimeDuplicate => "prime-duplicate",
            DictionaryDetail::PrimeOrder => "prime-order",
        }
    }
}

impl fmt::Display for DictionaryDetail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The `detail` of `P_COORDINATE_NOT_EXTENSION`: which check of `extends`
/// failed first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExtensionDetail {
    /// The new dictionary has a different `profile`.
    Profile,
    /// A family of the old dictionary is missing from the new one or has a
    /// different `values`.
    Family,
    /// A facet of the old dictionary is missing from the new one or has a
    /// different prime.
    Facet,
}

impl ExtensionDetail {
    pub fn as_str(self) -> &'static str {
        match self {
            ExtensionDetail::Profile => "profile",
            ExtensionDetail::Family => "family",
            ExtensionDetail::Facet => "facet",
        }
    }
}

impl fmt::Display for ExtensionDetail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
