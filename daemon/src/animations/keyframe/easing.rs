use core::borrow::Borrow;
use core::mem::MaybeUninit;

/// Implementation of a 2D curve function for easing between two points
pub trait EasingFunction {
    /// For an X position on the curve, calculate the Y position.
    /// 0.0-1.0 is start and end on both axes but values can go out of bounds.
    ///
    /// # Note
    ///
    /// Because this method has a `&self` argument this trait can be used to both implement a "static" curve function (e.g. a linear interpolation)
    /// or a "dynamic" curve function (e.g. a bezier curve with user defined inputs).
    ///
    /// Since a static curve function will have zero size the size of a `dyn EasingFunction` will be the same size as a vtable.
    /// This also means you can specify a static curve function with only the name of the type (e.g. `ease(EaseInOut, 0.0, 1.0, 0.5)`).
    fn y(&self, x: f64) -> f64;
}

/// Type that can be used with an easing function
pub trait CanTween {
    /// Returns the interpolated value between `from` and `to` at the specified time.
    ///
    /// # Note
    /// This function will always create a new value, so calling it on very large structures is not a good idea.
    fn ease(from: Self, to: Self, time: f64) -> Self;
}

impl CanTween for f32 {
    #[inline]
    fn ease(from: Self, to: Self, time: f64) -> Self {
        from + (to - from) * time as f32
    }
}

impl CanTween for f64 {
    #[inline]
    fn ease(from: Self, to: Self, time: f64) -> Self {
        from + (to - from) * time
    }
}

impl<T: CanTween, const N: usize> CanTween for [T; N] {
    fn ease(from: Self, to: Self, time: f64) -> Self {
        // This is safe, see: https://doc.rust-lang.org/core/mem/union.MaybeUninit.html#initializing-an-array-element-by-element
        let mut result_uninit: [MaybeUninit<T>; N] = unsafe { MaybeUninit::uninit().assume_init() };

        for (i, (f, t)) in IntoIterator::into_iter(from)
            .zip(IntoIterator::into_iter(to))
            .enumerate()
        {
            // Initialize the array while moving elements out of from and to...
            result_uninit[i].write(T::ease(f, t, time));
        }

        let ptr = result_uninit.as_mut_ptr() as *mut [T; N];
        // MaybeUninit<T> is guaranteed to have the same size, alignment, and ABI as T.
        unsafe { ptr.read() }
    }
}

/// Returns the value at a specified X position on the curve between point A and point B.
/// The time argument is expected to stay within a range of 0.0 to 1.0 but bounds checking is not enforced.
#[inline]
pub fn ease_with_unbounded_time<V: CanTween, F: EasingFunction>(
    function: impl Borrow<F>,
    from: V,
    to: V,
    time: f64,
) -> V {
    V::ease(from, to, function.borrow().y(time))
}

/// Returns the value at a specified X position on the curve between point A and point B.
/// Time is limited to a range between 0.0 and 1.0.
#[inline]
pub fn ease<V: CanTween, F: EasingFunction>(
    function: impl Borrow<F>,
    from: V,
    to: V,
    time: f64,
) -> V {
    ease_with_unbounded_time(
        function,
        from,
        to,
        match time {
            _ if time < 0.0 => 0.0,
            _ if time > 1.0 => 1.0,
            _ => time,
        },
    )
}

/// Returns the value at a specified X position on the curve between point A and point B.
/// Time is limited to a range between 0.0 and `max_time`.
#[inline]
pub fn ease_with_scaled_time<V: CanTween, F: EasingFunction>(
    function: impl Borrow<F>,
    from: V,
    to: V,
    time: f64,
    max_time: f64,
) -> V {
    ease(
        function,
        from,
        to,
        match time {
            _ if time < 0.0 => 0.0,
            _ if time > max_time => 1.0,
            _ => time / max_time,
        },
    )
}
