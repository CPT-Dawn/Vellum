//! This lazy clock implementation was directly inspired by
//! [niri](https://github.com/YaLTeR/niri)'s clock implementation

use core::cell::UnsafeCell;
use core::sync::atomic::AtomicBool;
use core::sync::atomic::Ordering;
use rustix::time::{ClockId, Timespec, clock_gettime};

static CLOCK: LazyClock = LazyClock {
    in_use: AtomicBool::new(false),
    cleared: AtomicBool::new(true),
    time: UnsafeCell::new(Timespec {
        tv_sec: 0,
        tv_nsec: 0,
    }),
};

struct LazyClock {
    in_use: AtomicBool,
    cleared: AtomicBool,
    time: UnsafeCell<Timespec>,
}

unsafe impl Sync for LazyClock {}

pub fn get() -> Timespec {
    assert!(
        !CLOCK.in_use.swap(true, Ordering::Relaxed),
        "Global clock has multiple borrows, which should be impossible"
    );

    // SAFETY: we ensure there is only one thing accessing the UnsafeCell data with the assert
    // above. This means we are free to read/write it as much as we want

    if CLOCK.cleared.load(Ordering::Relaxed) {
        CLOCK.cleared.store(false, Ordering::Relaxed);
        let new_clock = clock_gettime(ClockId::Monotonic);
        unsafe { CLOCK.time.get().write(new_clock) };
    }

    // make sure to read the data before setting to clock to not in use anymore
    let ret = unsafe { CLOCK.time.get().read() };
    CLOCK.in_use.store(false, Ordering::Relaxed);
    ret
}

pub fn reset() {
    CLOCK.cleared.store(true, Ordering::Relaxed);
}
