//! Piecewise-linear segment resampling onto a fixed time grid.
//!
//! IMPLEMENT THIS MODULE. Do **not** change the public items below — they are
//! consumed by `materialize.rs`. You may add private helpers.
//!
//! Port of `_interp_segments` in `src/gb_grid/analytics.py`:
//! - Segments are `[time_from, time_to)` half-open, ramping linearly from
//!   `level_from` to `level_to`. A span <= 0 yields the constant `level_from`.
//! - Where multiple segments cover a grid point, the segment that sorts *later*
//!   in `keys` (ascending, **stable**) wins — the caller's `keys` encode the
//!   Python `order_cols` (BOALF: `[acceptance_time, time_from]`; MELS:
//!   `[notification_sequence, time_from]`; PN: `[time_from]`).
//! - Values: `level_from + (level_to - level_from) * (offset_ns / span_ns)`,
//!   identical to the numpy expression so results match bit-for-bit.
//! - Grid points outside any segment → NaN (`f64::NAN`).
//!
//! Implement as a sweep over grid points with a per-point active-set max-key
//! pick (two pointers over the stable-sorted segments) — the numpy masked-stamp
//! loop is O(segments × points); the sweep is O(points + segments).

use chrono::NaiveDateTime;

/// Sort key for overlap arbitration. `keys[i]` is the per-segment key vector
/// compared lexicographically (ascending, stable tie-break by original index).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SortKey {
    Time(NaiveDateTime),
    Int(i64),
    Float(f64),
}

#[derive(Clone, Debug)]
pub struct Segment {
    pub time_from: NaiveDateTime,
    pub time_to: NaiveDateTime,
    pub level_from: f64,
    pub level_to: f64,
}

/// Resample `segments` onto the grid `idx`, returning one value per grid point
/// (NaN outside any segment). `keys` must have one entry per segment.
pub fn interp_segments(
    segments: &[Segment],
    keys: &[Vec<SortKey>],
    idx: &[NaiveDateTime],
) -> Vec<f64> {
    todo!("implement interp_segments")
}