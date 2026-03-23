mod dynamic_functions;
pub use dynamic_functions::*;

/// Linear interpolation from point A to point B
#[derive(Copy, Clone, Debug, Default)]
pub struct Linear;
impl super::EasingFunction for Linear {
    #[inline]
    fn y(&self, x: f64) -> f64 {
        x
    }
}
