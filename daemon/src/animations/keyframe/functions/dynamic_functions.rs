use super::super::EasingFunction;
use super::super::Vector2;

pub const SAMPLE_TABLE_SIZE: usize = 20;

const NEWTON_ITERATIONS: usize = 4;
const NEWTON_MIN_SLOPE: f32 = 0.001;
const SUBDIVISION_PRECISION: f32 = 0.0000001;
const SUBDIVISION_MAX_ITERATIONS: usize = 10;

/// User-defined cubic Bézier curve
#[derive(Debug, Clone, Copy)]
pub struct BezierCurve {
    sample_table: [f32; SAMPLE_TABLE_SIZE],
    p1: Vector2,
    p2: Vector2,
}

// Directly translated from https://github.com/gre/bezier-easing
impl BezierCurve {
    #[inline]
    fn a(x1: f32, x2: f32) -> f32 {
        1.0 - 3.0 * x2 + 3.0 * x1
    }
    #[inline]
    fn b(x1: f32, x2: f32) -> f32 {
        3.0 * x2 - 6.0 * x1
    }
    #[inline]
    fn c(x1: f32) -> f32 {
        3.0 * x1
    }

    #[inline]
    fn at(t: f32, x1: f32, x2: f32) -> f32 {
        ((Self::a(x1, x2) * t + Self::b(x1, x2)) * t + Self::c(x1)) * t
    }
    #[inline]
    fn slope(t: f32, x1: f32, x2: f32) -> f32 {
        3.0 * Self::a(x1, x2) * t * t + 2.0 * Self::b(x1, x2) * t + Self::c(x1)
    }

    fn newton_raphson(x: f32, guess: f32, x1: f32, x2: f32) -> f32 {
        let mut guess = guess;

        for _ in 0..NEWTON_ITERATIONS {
            let current_slope = Self::slope(guess, x1, x2);
            if current_slope == 0.0 {
                break;
            }

            let current_x = Self::at(guess, x1, x2) - x;
            guess -= current_x / current_slope;
        }

        guess
    }

    fn binary_subdivide(x: f32, mut a: f32, mut b: f32, x1: f32, x2: f32) -> f32 {
        let mut current_x = 0.0f32;
        let mut current_t = 0.0f32;
        let mut i = 0;

        let mut has_run_once = false;
        while !has_run_once
            || current_x.abs() > SUBDIVISION_PRECISION && i + 1 < SUBDIVISION_MAX_ITERATIONS
        {
            has_run_once = true;
            current_t = a + (b - a) / 2.0;
            current_x = Self::at(current_t, x1, x2) - x;

            if current_x > 0.0 {
                b = current_t;
            } else {
                a = current_t;
            }

            i += 1;
        }

        current_t
    }

    fn t_for_x(&self, x: f32) -> f32 {
        let mut interval_start = 0.0;
        let mut current_sample = 1;
        let last_sample = SAMPLE_TABLE_SIZE - 1;
        let sample_step_size = 1.0 / (SAMPLE_TABLE_SIZE as f32 - 1.0);

        while current_sample != last_sample && self.sample_table[current_sample] <= x {
            interval_start += sample_step_size;
            current_sample += 1;
        }
        current_sample -= 1;

        let dist = (x - self.sample_table[current_sample])
            / (self.sample_table[current_sample + 1] - self.sample_table[current_sample]);
        let guess_for_t = interval_start + dist * sample_step_size;

        match Self::slope(guess_for_t, self.p1.x, self.p2.x) {
            initial_slope if initial_slope >= NEWTON_MIN_SLOPE => {
                Self::newton_raphson(x, guess_for_t, self.p1.x, self.p2.x)
            }
            0.0 => guess_for_t,
            _ => Self::binary_subdivide(
                x,
                interval_start,
                interval_start + sample_step_size,
                self.p1.x,
                self.p2.x,
            ),
        }
    }

    /// Calculates a new cubic Bézier curve. Mimics `transition-timing-function: cubic-bezier` as defined [here](https://www.w3.org/TR/css-easing-1/#cubic-bezier-easing-functions)
    ///
    /// # Arguments
    ///
    /// * `p1` - The first of the two control points
    /// * `p2` - The second of the two control points
    pub fn from(p1: Vector2, p2: Vector2) -> Self {
        let mut arr = [0.0; SAMPLE_TABLE_SIZE];
        for (i, value) in (0..SAMPLE_TABLE_SIZE).enumerate().map(|x| {
            (
                x.0,
                Self::at(x.1 as f32 * SAMPLE_TABLE_SIZE as f32, p1.x, p2.x),
            )
        }) {
            arr[i] = value;
        }

        BezierCurve {
            sample_table: arr,
            p1,
            p2,
        }
    }
}

impl EasingFunction for BezierCurve {
    #[inline]
    fn y(&self, x: f64) -> f64 {
        match x {
            _ if x == 0.0 => 0.0,
            _ if x == 1.0 => 1.0,
            _ => BezierCurve::at(self.t_for_x(x as f32), self.p1.y, self.p2.y) as f64,
        }
    }
}
