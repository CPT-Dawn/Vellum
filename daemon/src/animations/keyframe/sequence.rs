use crate::animations::keyframe::functions::BezierCurve;

use super::Keyframe;
use super::Vector2;

/// A collection of keyframes that can be played back in sequence
pub struct AnimationSequence {
    // NOTE: in the original keyframe crate, this was a Vec. Because we only ever use sequences
    // with exactly 2 keyframes, that was a waste
    sequence: [Keyframe; 2],
    // Current item we're animating
    keyframe: Option<usize>,
    // Current time
    time: f64,
}

impl AnimationSequence {
    fn update_current_keyframe(&mut self) {
        // Common cases, reversing/wrapping
        if !self.sequence.is_empty() && self.time == 0.0 {
            self.keyframe = Some(0);
            return;
        }
        if !self.sequence.is_empty() && self.time == self.duration() {
            self.keyframe = Some(self.sequence.len() - 1);
            return;
        }

        if let Some(k) = self.keyframe {
            if self.keyframes() <= k {
                self.keyframe = None;
            }

            if self.sequence[k].time() > self.time {
                for i in (0..self.keyframe.unwrap_or(0)).rev() {
                    if self.sequence[i].time() <= self.time {
                        self.keyframe = Some(i);
                        return;
                    }

                    self.keyframe = None;
                }
            } else {
                let copy = self.keyframe;
                self.keyframe = None;

                for i in copy.unwrap_or(0)..self.keyframes() {
                    if self.sequence[i].time() > self.time {
                        break;
                    } else {
                        self.keyframe = Some(i)
                    }
                }
            }
        } else if self.keyframes() > 0 {
            self.keyframe = Some(0);
            self.update_current_keyframe();
        }
    }

    /// The number of keyframes in this sequence
    #[inline]
    pub fn keyframes(&self) -> usize {
        self.sequence.len()
    }

    /// The current pair of keyframes that are being animated (current, next)
    ///
    /// # Note
    ///
    /// The following applies if:
    /// * There are no keyframes in this sequence: (`None`, `None`) is returned
    /// * The sequence has not reached the first keyframe: (`None`, current) is returned
    /// * There is only one keyframe in this sequence and the sequence has reached it: (current, `None`) is returned
    /// * The sequence has finished: (current, `None`) is returned
    pub fn pair(&self) -> (Option<&Keyframe>, Option<&Keyframe>) {
        match self.keyframe {
            Some(c) if c == self.sequence.len() - 1 => (Some(&self.sequence[c]), None),
            Some(c) => (Some(&self.sequence[c]), Some(&self.sequence[c + 1])),
            None if !self.sequence.is_empty() => (None, Some(&self.sequence[0])),
            None => (None, None),
        }
    }

    /// The current value of this sequence, use the default if necessary.
    pub fn now(&self) -> f32 {
        match self.pair() {
            (Some(s1), Some(s2)) => s1.tween_to(s2, self.time),
            (Some(s1), None) => s1.value(),
            (None, Some(s2)) => Keyframe::new(
                0.0,
                0.0,
                BezierCurve::from(Vector2 { x: 0.0, y: 0.0 }, Vector2 { x: 1.0, y: 1.0 }),
            )
            .tween_to(s2, self.time),
            (None, None) => 0.0,
        }
    }

    /// Advances this sequence to the exact timestamp.
    ///
    /// Returns the remaining time (i.e. the amount that the specified timestamp went outside the bounds of the total duration of this sequence)
    /// after the operation has completed.
    ///
    /// A value over 0 indicates the sequence is at the finish point.
    /// A value under 0 indicates this sequence is at the start point.
    ///
    /// # Note
    ///
    /// The following applies if:
    /// * The timestamp is negative: the sequence is set to `0.0`
    /// * The timestamp is after the duration of the sequence: the sequence is set to `duration()`
    pub fn advance_to(&mut self, timestamp: f64) -> f64 {
        self.time = match timestamp {
            _ if timestamp < 0.0 => 0.0,
            _ if timestamp > self.duration() => self.duration(),
            _ => timestamp,
        };

        self.update_current_keyframe();
        timestamp - self.time
    }

    /// The length in seconds of this sequence
    #[inline]
    pub fn duration(&self) -> f64 {
        // Keyframe::default means that if we don't have any items in this collection (meaning - 1 is out of bounds) the maximum time will be 0.0
        self.sequence.last().map_or(0.0, Keyframe::time)
    }

    /// If this sequence has finished and is at the end.
    /// It can be reset with `advance_to(0.0)`.
    #[inline]
    pub fn finished(&self) -> bool {
        self.time == self.duration()
    }
}

impl From<[Keyframe; 2]> for AnimationSequence {
    /// Creates a new animation sequence from a vector of keyframes
    fn from(mut sequence: [Keyframe; 2]) -> Self {
        // NOTE: the original keyframe crate sorts the sequence here and then eliminates duplicates
        //
        // Instead of that, we will just do a single compare swap for sorting, and increase the
        // second keyframe's time if it is identical to the first's
        if sequence[0].time() > sequence[1].time() {
            sequence.swap(0, 1);
        } else if sequence[0].time() == sequence[1].time() {
            // NOTE: Keyframe::new guarantees the time is not negative
            let bits = sequence[1].time.to_bits();
            sequence[1].time = f64::from_bits(bits + 4);
        }

        let mut me = AnimationSequence {
            sequence,
            keyframe: None,
            time: 0.0,
        };

        me.update_current_keyframe();

        me
    }
}
