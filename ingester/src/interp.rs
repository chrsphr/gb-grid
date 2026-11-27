//! Piecewise-linear segment resampling onto a fixed time grid.
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
//! Implemented as a sweep over grid points with a per-point active-set max-key
//! pick. The numpy masked-stamp loop is O(segments × points); this version is
//! O(points + segments): sort the segment indices by key (stable), then walk the
//! sorted list largest-key-first and let each segment paint the still-unclaimed
//! grid points it covers (union-find "next-unassigned" pointer). The first
//! segment in reverse-key order to cover a point is the max-key covering
//! segment — identical to numpy's ascending-stamp / last-overwrite behaviour,
//! including the stable tie-break where the later input row wins.

use std::cmp::Ordering;

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
    let n = segments.len();
    let p = idx.len();
    let mut out = vec![f64::NAN; p];
    if n == 0 || p == 0 {
        return out;
    }

    // Stable-sort the segment indices ascending by key (`slice::sort_by` is
    // stable), so equal-key segments keep their input order. We then claim
    // grid points largest-key-first: the first segment in that order to cover
    // a point is the max-key covering segment, which is the value numpy leaves
    // in place after stamping ascending and letting later rows overwrite.
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| cmp_keys(&keys[a], &keys[b]));

    // `next[j]` = the smallest unassigned grid index >= j (`next[p]` sentinel).
    // Every grid point is claimed exactly once, by the winning segment, so the
    // sweep is O(points + segments) with path-compressed lookups.
    let mut next: Vec<usize> = (0..=p).collect();

    for &i in order.iter().rev() {
        let seg = &segments[i];
        // Half-open cover [time_from, time_to). A `time_to <= time_from`
        // segment has an empty cover and never stamps — numpy's mask
        // `(pts >= t_from) & (pts < t_to)` is empty (and for NaT time_to the
        // `pts < NaT` comparison is False everywhere), so it can never win.
        let lo = idx.partition_point(|t| *t < seg.time_from);
        let hi = idx.partition_point(|t| *t < seg.time_to);
        if hi <= lo {
            continue;
        }
        let span_ns = seg
            .time_to
            .signed_duration_since(seg.time_from)
            .num_nanoseconds()
            .unwrap_or(0);
        let t_from_ns = seg.time_from.and_utc().timestamp_nanos_opt().unwrap_or(0);

        let mut j = find_next(&mut next, lo);
        while j < hi {
            let value = if span_ns <= 0 {
                // Constant `level_from` (unreachable in practice: a span <= 0
                // gives an empty cover, matching numpy's empty-mask skip).
                seg.level_from
            } else {
                // Numpy: frac = offset / span as f64 division of the i64
                // nanosecond counts, then vals = v_from + (v_to - v_from)*frac.
                let pt_ns = idx[j].and_utc().timestamp_nanos_opt().unwrap_or(0);
                let offset = pt_ns - t_from_ns;
                let frac = offset as f64 / span_ns as f64;
                seg.level_from + (seg.level_to - seg.level_from) * frac
            };
            out[j] = value;
            next[j] = j + 1;
            j = find_next(&mut next, j);
        }
    }
    out
}

/// Path-compressed union-find "next unassigned index" lookup.
fn find_next(next: &mut [usize], mut j: usize) -> usize {
    while next[j] != j {
        let k = next[j];
        next[j] = next[k];
        j = k;
    }
    j
}

/// Lexicographic comparison of two per-segment key vectors.
fn cmp_keys(a: &[SortKey], b: &[SortKey]) -> Ordering {
    for (x, y) in a.iter().zip(b.iter()) {
        match cmp_sort_key(x, y) {
            Ordering::Equal => continue,
            other => return other,
        }
    }
    a.len().cmp(&b.len())
}

fn cmp_sort_key(a: &SortKey, b: &SortKey) -> Ordering {
    use SortKey::*;
    match (a, b) {
        (Time(x), Time(y)) => x.cmp(y),
        (Int(x), Int(y)) => x.cmp(y),
        (Float(x), Float(y)) => cmp_f64(*x, *y),
        // Cross-type pairs cannot occur within a single sort column in the
        // Python pipeline (a column is all-datetime or all-int), but keep a
        // deterministic fallback order (Time < Int < Float).
        (Time(_), _) => Ordering::Less,
        (Int(_), Time(_)) => Ordering::Greater,
        (Int(_), Float(_)) => Ordering::Less,
        (Float(_), _) => Ordering::Greater,
    }
}

/// pandas `sort_values` places NaN/NaT *last* (`na_position='last'`, verified
/// empirically), so a NaN Float key sorts above any finite value — the NaN row
/// loses overlaps in the ascending sort and is the *first* row processed in the
/// reverse sweep, so it can only claim points no real key covers.
fn cmp_f64(a: f64, b: f64) -> Ordering {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
    }
}