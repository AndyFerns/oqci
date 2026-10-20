//! Gate parameters: concrete angles and symbolic placeholders.
//!
//! Stage F (`docs/core_architecture/stage-f-static-parameterized-circuit-scope.md`)
//! requires QC-IR to represent **static circuit topology with symbolic *or*
//! numeric rotation parameters**, so that variational (VQE-style) ansatzes are
//! expressible without introducing dynamic control flow. [`Param`] is that
//! representation: every gate angle is either a concrete [`Angle`] or a named
//! symbol resolved later by an explicit binding step
//! ([`crate::ir::bind_parameters`]).
//!
//! # Why a separate type from [`Angle`]
//!
//! [`Angle`] remains the concrete radian value. `Param` wraps it with the
//! "not yet known" case. Keeping them distinct preserves the Stage F
//! requirement to distinguish *a concrete numeric angle* from *a
//! parameter/symbol*, and keeps the MLIR mapping honest: a concrete `Param`
//! lowers to a `FloatAttr`, a symbolic one to a named parameter attribute (see
//! `docs/mlir_dialect.md`).
//!
//! # Binding is explicit
//!
//! A circuit containing symbolic parameters is a **valid** QC-IR circuit —
//! [`crate::ir::CircuitBuilder::build`] accepts it. Only lowering requires
//! concrete values: [`crate::ir::emit_qir`] reports
//! [`crate::ir::IrError::UnboundParameter`] rather than inventing a value.

use std::fmt;

use crate::ir::types::Angle;

/// A gate parameter: a concrete angle, or a named symbol awaiting binding.
///
/// Construct with [`Param::concrete`] / [`Param::symbol`], or via the `From`
/// impls (`f64` and [`Angle`] both convert to [`Param::Concrete`]).
///
/// ```
/// use oqci::ir::Param;
/// let fixed = Param::concrete(std::f64::consts::FRAC_PI_2);
/// let free = Param::symbol("theta");
/// assert!(fixed.is_concrete());
/// assert_eq!(free.as_symbol(), Some("theta"));
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum Param {
    /// A known numeric angle, in radians.
    Concrete(Angle),
    /// A named symbolic parameter, resolved by [`crate::ir::bind_parameters`].
    ///
    /// The name must be non-empty; this is validated by
    /// [`crate::ir::CircuitBuilder::build`], which reports
    /// [`crate::ir::IrError::EmptyParameterSymbol`].
    Symbol(String),
}

impl Param {
    /// Builds a concrete parameter from a radian value.
    #[must_use]
    pub const fn concrete(radians: f64) -> Self {
        Param::Concrete(Angle::new(radians))
    }

    /// Builds a symbolic parameter with the given name.
    #[must_use]
    pub fn symbol(name: impl Into<String>) -> Self {
        Param::Symbol(name.into())
    }

    /// `true` if this parameter already holds a numeric value.
    #[must_use]
    pub const fn is_concrete(&self) -> bool {
        matches!(self, Param::Concrete(_))
    }

    /// `true` if this parameter is an unbound symbol.
    #[must_use]
    pub const fn is_symbolic(&self) -> bool {
        matches!(self, Param::Symbol(_))
    }

    /// The concrete angle, or `None` if this parameter is symbolic.
    #[must_use]
    pub const fn as_concrete(&self) -> Option<Angle> {
        match self {
            Param::Concrete(a) => Some(*a),
            Param::Symbol(_) => None,
        }
    }

    /// The symbol name, or `None` if this parameter is concrete.
    #[must_use]
    pub fn as_symbol(&self) -> Option<&str> {
        match self {
            Param::Symbol(name) => Some(name),
            Param::Concrete(_) => None,
        }
    }

    /// `true` if this parameter is well-formed: a concrete angle must be
    /// finite, a symbol must have a non-empty name.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        match self {
            Param::Concrete(a) => a.is_finite(),
            Param::Symbol(name) => !name.is_empty(),
        }
    }
}

/// The most a compiler-computed angle may differ, through floating-point
/// rounding, from the exact value of the arithmetic that produced it.
///
/// Rewrites that compute an angle — merging `Rz(a); Rz(b)` into
/// `Rz(a + b)`, or a decomposition rule's `scale·θ + offset` — are exact in
/// real arithmetic but rounded in `f64`. For ordinary angles the rounding is
/// around 1e-16 rad. For huge ones it is not: at |θ| ≈ 7e18 one floating-point
/// step is about 1024 rad, so `7e18 + 0.4` rounds back to `7e18` and the
/// merged rotation is simply different. The fuzz targets found exactly that
/// miscompilation, and the in-compiler verifier reported it. So every such
/// computation is checked: an optimization skips a rewrite whose rounding
/// exceeds this bound, and lowering refuses one. In practice that happens
/// only beyond about 10³ rad.
pub const ANGLE_ROUNDING_LIMIT: f64 = 1e-12;

/// The largest |angle|, in radians, whose sine and cosine OQCI evaluates
/// numerically.
///
/// Platform maths libraries differ for huge arguments: on the Windows GNU
/// toolchain `cos(5.5e19)` is wrong in the third decimal place, while glibc and
/// the MSVC runtime get it right. A pass that built a matrix from such a value
/// would emit a wrong gate on one platform and a right one on another. So
/// nothing in the compiler evaluates trigonometry beyond this bound (2²⁰ rad,
/// where every mainstream libm is accurate to around 1e-14): fusion leaves
/// such gates alone, and the in-compiler verifier reports them inconclusive.
/// Gates with larger angles still compile; their angles are carried through
/// verbatim, never re-derived from a matrix. Found by fuzzing.
pub const MAX_EVALUATED_ANGLE: f64 = 1_048_576.0;

/// `scale · x + offset`, or `None` if `f64` cannot compute it to within
/// [`ANGLE_ROUNDING_LIMIT`] of the exact real value (or it is not finite).
///
/// The rounding error is computed exactly, not estimated: the product's error
/// by a fused multiply-add, the sum's by Knuth's TwoSum.
#[must_use]
pub fn exact_affine(scale: f64, x: f64, offset: f64) -> Option<f64> {
    let product = scale * x;
    let product_error = scale.mul_add(x, -product);
    let sum = product + offset;
    let virtual_offset = sum - product;
    let sum_error = (product - (sum - virtual_offset)) + (offset - virtual_offset);
    let error = product_error.abs() + sum_error.abs();
    (sum.is_finite() && error.is_finite() && error <= ANGLE_ROUNDING_LIMIT).then_some(sum)
}

impl From<Angle> for Param {
    fn from(angle: Angle) -> Self {
        Param::Concrete(angle)
    }
}

impl From<f64> for Param {
    fn from(radians: f64) -> Self {
        Param::Concrete(Angle::new(radians))
    }
}

impl fmt::Display for Param {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Param::Concrete(a) => write!(f, "{a}"),
            Param::Symbol(name) => write!(f, "%{name}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concrete_round_trips() {
        let p = Param::concrete(1.5);
        assert!(p.is_concrete());
        assert!(!p.is_symbolic());
        assert_eq!(p.as_concrete().map(Angle::radians), Some(1.5));
        assert_eq!(p.as_symbol(), None);
    }

    #[test]
    fn symbol_round_trips() {
        let p = Param::symbol("theta");
        assert!(p.is_symbolic());
        assert!(!p.is_concrete());
        assert_eq!(p.as_symbol(), Some("theta"));
        assert_eq!(p.as_concrete(), None);
    }

    #[test]
    fn conversions_produce_concrete() {
        assert_eq!(Param::from(0.25), Param::concrete(0.25));
        assert_eq!(Param::from(Angle::new(0.25)), Param::concrete(0.25));
    }

    #[test]
    fn validity_covers_both_cases() {
        assert!(Param::concrete(0.0).is_valid());
        assert!(!Param::concrete(f64::NAN).is_valid());
        assert!(Param::symbol("x").is_valid());
        assert!(!Param::symbol("").is_valid());
    }

    #[test]
    fn exact_affine_accepts_ordinary_angles_and_refuses_lossy_ones() {
        assert_eq!(exact_affine(1.0, 0.3, 0.4), Some(0.3 + 0.4));
        assert_eq!(
            exact_affine(-0.5, 2.0, std::f64::consts::PI),
            Some(-1.0 + std::f64::consts::PI)
        );
        // The fuzz finding: 7e18 + 0.4 rounds the 0.4 away entirely.
        assert_eq!(exact_affine(1.0, 7.009_997_822_827_971e18, 0.4), None);
        assert_eq!(exact_affine(1.0, f64::MAX, f64::MAX), None);
        // A large angle whose arithmetic happens to be exact is fine.
        assert_eq!(exact_affine(1.0, 1024.0, 1.0), Some(1025.0));
    }

    #[test]
    fn display_distinguishes_symbols() {
        assert_eq!(Param::concrete(0.5).to_string(), "0.5");
        assert_eq!(Param::symbol("theta").to_string(), "%theta");
    }
}
