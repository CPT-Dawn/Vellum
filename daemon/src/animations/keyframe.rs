//! This is a modified version of the [keyframe](https://github.com/hannesmann/keyframe) crate.
//!
//! We have made it less generic, deleted methods we do not use, and most importantly, deleted the
//! calls to `slice::sort`, as those caused a large amount of unnecessary code to be included in the
//! final binary.
//!
//! Below follows the original keyframe license:
//!
//! MIT License

//! Copyright (c) 2019 Hannes Mann
//!
//! Permission is hereby granted, free of charge, to any person obtaining a copy
//! of this software and associated documentation files (the "Software"), to deal
//! in the Software without restriction, including without limitation the rights
//! to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
//! copies of the Software, and to permit persons to whom the Software is
//! furnished to do so, subject to the following conditions:
//!
//! The above copyright notice and this permission notice shall be included in all
//! copies or substantial portions of the Software.
//!
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
//! IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
//! FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
//! AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
//! LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
//! OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
//! SOFTWARE.

mod easing;
mod sequence;

pub mod functions;

pub use sequence::*;

use easing::{CanTween, EasingFunction, ease_with_scaled_time};
use functions::{BezierCurve, Linear};

#[derive(Debug, Clone, Copy)]
pub struct Vector2 {
    pub x: f32,
    pub y: f32,
}

/// Intermediate step in an animation sequence
pub struct Keyframe {
    value: f32,
    time: f64,
    function: BezierCurve,
}

impl Keyframe {
    /// Creates a new keyframe from the specified values.
    /// If the time value is negative the keyframe will start at 0.0.
    ///
    /// # Arguments
    /// * `value` - The value that this keyframe will be tweened to/from
    /// * `time` - The start time in seconds of this keyframe
    /// * `function` - The easing function to use from the start of this keyframe to the start of the next keyframe
    #[inline]
    pub fn new(value: f32, time: f64, function: BezierCurve) -> Self {
        Keyframe {
            value,
            time: time.max(0.0),
            function,
        }
    }

    /// The value of this keyframe
    #[inline]
    pub fn value(&self) -> f32 {
        self.value
    }

    /// The time in seconds at which this keyframe starts in a sequence
    #[inline]
    pub fn time(&self) -> f64 {
        self.time
    }

    /// Returns the value between this keyframe and the next keyframe at the specified time
    ///
    /// # Note
    ///
    /// The following applies if:
    /// * The requested time is before the start time of this keyframe: the value of this keyframe is returned
    /// * The requested time is after the start time of next keyframe: the value of the next keyframe is returned
    /// * The start time of the next keyframe is before the start time of this keyframe: the value of the next keyframe is returned
    #[inline]
    pub fn tween_to(&self, next: &Keyframe, time: f64) -> f32 {
        match time {
            // If the requested time starts before this keyframe
            time if time < self.time => self.value,
            // If the requested time starts after the next keyframe
            time if time > next.time => next.value,
            // If the next keyframe starts before this keyframe
            _ if next.time < self.time => next.value,

            time => f32::ease(
                self.value,
                next.value,
                self.function.y(ease_with_scaled_time(
                    Linear,
                    0.0,
                    1.0,
                    time - self.time,
                    next.time - self.time,
                )),
            ),
        }
    }
}

impl core::fmt::Display for Keyframe {
    #[inline]
    fn fmt(&self, f: &mut core::fmt::Formatter) -> Result<(), core::fmt::Error> {
        write!(f, "Keyframe at {} s: {}", self.time, self.value)
    }
}

impl core::fmt::Debug for Keyframe {
    #[inline]
    fn fmt(&self, f: &mut core::fmt::Formatter) -> Result<(), core::fmt::Error> {
        write!(
            f,
            "Keyframe {{ value: {}, time: {} }}",
            self.value, self.time
        )
    }
}
