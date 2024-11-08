use p3_field::{ExtensionField, Field};

pub(super) enum ZerofierExpression<F> {
    Constant(F),
    X(Exponent),
    G(Exponent),
    Add(Box<Self>, Box<Self>),
    Sub(Box<Self>, Box<Self>),
    Mul(Box<Self>, Box<Self>),
    Div(Box<Self>, Box<Self>),
}

impl<F: Field> ZerofierExpression<F> {
    pub fn eval<EF: ExtensionField<F>>(&self, x: EF, g: F, n: usize) -> Option<EF> {
        let eval = match self {
            Self::Constant(c) => (*c).into(),
            Self::X(exp) => x.exp_u64(exp.power(n) as u64),
            Self::G(exp) => x.exp_u64(exp.power(n) as u64),
            Self::Add(lhs, rhs) => lhs.eval(x, g, n)? + rhs.eval(x, g, n)?,
            Self::Sub(lhs, rhs) => lhs.eval(x, g, n)? - rhs.eval(x, g, n)?,
            Self::Mul(lhs, rhs) => lhs.eval(x, g, n)? * rhs.eval(x, g, n)?,
            Self::Div(lhs, rhs) => lhs.eval(x, g, n)? * rhs.eval(x, g, n)?.try_inverse()?,
        };
        Some(eval)
    }
}

pub(super) enum Exponent {
    /// a^i
    First(usize),
    /// a^{n-i}
    Last(usize),
    /// a^{n/i}
    Rate(usize),
}

impl Exponent {
    fn power(&self, n: usize) -> usize {
        match *self {
            Exponent::First(i) => i,
            Exponent::Last(i) => n - i,
            Exponent::Rate(i) => n / i,
        }
    }
}
