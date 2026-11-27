//! Pure unit tests for `interp_segments` (no DB).

use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime};

use gb_grid_ingest::interp::{interp_segments, Segment, SortKey};

fn ts(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> NaiveDateTime {
    NaiveDateTime::new(
        NaiveDate::from_ymd_opt(y, mo, d).unwrap(),
        NaiveTime::from_hms_opt(h, mi, s).unwrap(),
    )
}

fn grid(start: NaiveDateTime, step_min: i64, n: usize) -> Vec<NaiveDateTime> {
    (0..n)
        .map(|i| start + Duration::minutes(step_min * i as i64))
        .collect()
}

fn assert_close(a: f64, b: f64) {
    if a.is_nan() && b.is_nan() {
        return;
    }
    assert!(
        (a - b).abs() <= 1e-12 * (1.0 + a.abs().max(b.abs())),
        "expected {b}, got {a}"
    );
}

fn assert_all(out: &[f64], expected: &[f64]) {
    assert_eq!(out.len(), expected.len());
    for (a, b) in out.iter().zip(expected) {
        assert_close(*a, *b);
    }
}

#[test]
fn single_ramp_exact_linear_values() {
    let seg = Segment {
        time_from: ts(2026, 5, 1, 12, 0, 0),
        time_to: ts(2026, 5, 1, 13, 0, 0),
        level_from: 100.0,
        level_to: 120.0,
    };
    let idx = grid(ts(2026, 5, 1, 12, 0, 0), 10, 6); // 12:00..12:50, all < 13:00
    let out = interp_segments(&[seg], &[vec![SortKey::Time(ts(2026, 5, 1, 12, 0, 0))]], &idx);

    let span = 3600.0;
    let expected: Vec<f64> = (0..6)
        .map(|i| 100.0 + (120.0 - 100.0) * ((i as f64 * 600.0) / span))
        .collect();
    assert_all(&out, &expected);
    // Spot-check exact literals.
    assert_close(out[0], 100.0);
    assert_close(out[1], 100.0 + 20.0 / 6.0); // 12:10
    assert_close(out[5], 100.0 + 20.0 * 5.0 / 6.0); // 12:50
}

#[test]
fn zero_and_negative_span_cover_nothing() {
    // time_from == time_to: half-open [t, t) is empty → NaN everywhere.
    let zero = Segment {
        time_from: ts(2026, 5, 1, 12, 0, 0),
        time_to: ts(2026, 5, 1, 12, 0, 0),
        level_from: 7.0,
        level_to: 7.0,
    };
    // time_to < time_from: empty cover too.
    let neg = Segment {
        time_from: ts(2026, 5, 1, 12, 0, 0),
        time_to: ts(2026, 5, 1, 11, 0, 0),
        level_from: 7.0,
        level_to: 7.0,
    };
    let idx = grid(ts(2026, 5, 1, 11, 55, 0), 5, 6);
    for seg in [zero, neg] {
        let out = interp_segments(&[seg.clone()], &[vec![SortKey::Time(seg.time_from)]], &idx);
        assert!(out.iter().all(|v| v.is_nan()), "expected all NaN, got {out:?}");
    }
}

#[test]
fn overlap_later_key_wins() {
    let a = Segment {
        time_from: ts(2026, 5, 1, 12, 0, 0),
        time_to: ts(2026, 5, 1, 13, 0, 0),
        level_from: 100.0,
        level_to: 100.0,
    };
    let b = Segment {
        time_from: ts(2026, 5, 1, 12, 10, 0),
        time_to: ts(2026, 5, 1, 12, 40, 0),
        level_from: 50.0,
        level_to: 50.0,
    };
    let idx = grid(ts(2026, 5, 1, 12, 0, 0), 5, 12); // 12:00..12:55
    let out = interp_segments(
        &[a.clone(), b.clone()],
        &[
            vec![SortKey::Time(ts(2026, 5, 1, 12, 0, 0))],
            vec![SortKey::Time(ts(2026, 5, 1, 12, 20, 0))],
        ],
        &idx,
    );
    // 12:00-12:05: A only. 12:10-12:35: both, B later key wins. 12:40-12:55: A only.
    let expected = [
        100.0, 100.0, 50.0, 50.0, 50.0, 50.0, 50.0, 50.0, 100.0, 100.0, 100.0, 100.0,
    ];
    assert_all(&out, &expected);
}

#[test]
fn stable_tie_break_later_input_wins() {
    let a = Segment {
        time_from: ts(2026, 5, 1, 12, 0, 0),
        time_to: ts(2026, 5, 1, 13, 0, 0),
        level_from: 100.0,
        level_to: 100.0,
    };
    let b = Segment {
        time_from: ts(2026, 5, 1, 12, 0, 0),
        time_to: ts(2026, 5, 1, 13, 0, 0),
        level_from: 80.0,
        level_to: 80.0,
    };
    let idx = grid(ts(2026, 5, 1, 12, 0, 0), 5, 4);
    let key = vec![SortKey::Time(ts(2026, 5, 1, 12, 0, 0))];

    // Input [a, b]: equal keys, later input (b) wins.
    let out = interp_segments(&[a.clone(), b.clone()], &[key.clone(), key.clone()], &idx);
    assert_all(&out, &[80.0; 4]);

    // Input [b, a]: equal keys, later input (a) wins.
    let out = interp_segments(&[b.clone(), a.clone()], &[key.clone(), key], &idx);
    assert_all(&out, &[100.0; 4]);
}

#[test]
fn outside_all_segments_is_nan() {
    let seg = Segment {
        time_from: ts(2026, 5, 1, 12, 0, 0),
        time_to: ts(2026, 5, 1, 12, 10, 0),
        level_from: 5.0,
        level_to: 5.0,
    };
    // Grid points before, inside, after, and well beyond the segment.
    let idx = [
        ts(2026, 5, 1, 11, 55, 0),
        ts(2026, 5, 1, 12, 0, 0),
        ts(2026, 5, 1, 12, 5, 0),
        ts(2026, 5, 1, 12, 15, 0),
        ts(2026, 5, 1, 13, 0, 0),
    ];
    let out = interp_segments(
        &[seg],
        &[vec![SortKey::Time(ts(2026, 5, 1, 12, 0, 0))]],
        &idx,
    );
    assert!(out[0].is_nan());
    assert_close(out[1], 5.0);
    assert_close(out[2], 5.0);
    assert!(out[3].is_nan());
    assert!(out[4].is_nan());
}

#[test]
fn keys_order_lexicographic_time_then_int() {
    // First key decides: Int(2) > Int(1), so seg_b wins over seg_a/seg_c.
    let a = Segment {
        time_from: ts(2026, 5, 1, 12, 0, 0),
        time_to: ts(2026, 5, 1, 13, 0, 0),
        level_from: 10.0,
        level_to: 10.0,
    };
    let b = Segment {
        time_from: ts(2026, 5, 1, 12, 0, 0),
        time_to: ts(2026, 5, 1, 13, 0, 0),
        level_from: 30.0,
        level_to: 30.0,
    };
    let c = Segment {
        time_from: ts(2026, 5, 1, 12, 0, 0),
        time_to: ts(2026, 5, 1, 13, 0, 0),
        level_from: 20.0,
        level_to: 20.0,
    };
    let idx = grid(ts(2026, 5, 1, 12, 15, 0), 5, 1);

    // a: [Int(1), Time(12:00)]; b: [Int(2), Time(12:00)]; c: [Int(1), Time(12:30)].
    let out = interp_segments(
        &[a.clone(), b, c.clone()],
        &[
            vec![SortKey::Int(1), SortKey::Time(ts(2026, 5, 1, 12, 0, 0))],
            vec![SortKey::Int(2), SortKey::Time(ts(2026, 5, 1, 12, 0, 0))],
            vec![SortKey::Int(1), SortKey::Time(ts(2026, 5, 1, 12, 30, 0))],
        ],
        &idx,
    );
    assert_close(out[0], 30.0);

    // c (equal Int(1), later Time(12:30)) beats a (Time(12:00)).
    let out = interp_segments(
        &[a, c],
        &[
            vec![SortKey::Int(1), SortKey::Time(ts(2026, 5, 1, 12, 0, 0))],
            vec![SortKey::Int(1), SortKey::Time(ts(2026, 5, 1, 12, 30, 0))],
        ],
        &idx,
    );
    assert_close(out[0], 20.0);
}

#[test]
fn nan_level_propagates() {
    let seg_from_nan = Segment {
        time_from: ts(2026, 5, 1, 12, 0, 0),
        time_to: ts(2026, 5, 1, 12, 30, 0),
        level_from: f64::NAN,
        level_to: 100.0,
    };
    let seg_to_nan = Segment {
        time_from: ts(2026, 5, 1, 12, 0, 0),
        time_to: ts(2026, 5, 1, 12, 30, 0),
        level_from: 100.0,
        level_to: f64::NAN,
    };
    let idx = grid(ts(2026, 5, 1, 12, 0, 0), 15, 3); // 12:00, 12:15, 12:30 (12:30 outside)
    for seg in [seg_from_nan, seg_to_nan] {
        let out = interp_segments(
            &[seg],
            &[vec![SortKey::Time(ts(2026, 5, 1, 12, 0, 0))]],
            &idx,
        );
        assert!(out[0].is_nan(), "covered point should be NaN, got {}", out[0]);
        assert!(out[1].is_nan(), "covered point should be NaN, got {}", out[1]);
        assert!(out[2].is_nan(), "outside point should be NaN");
    }
}

#[test]
fn empty_segments_yield_all_nan() {
    let idx = grid(ts(2026, 5, 1, 12, 0, 0), 5, 3);
    let out = interp_segments(&[], &[], &idx);
    assert!(out.iter().all(|v| v.is_nan()));
}

#[test]
fn bitwise_matches_reference_python() {
    // Cross-validated against the real `_interp_segments` from analytics.py
    // (pandas 3.0.2 / numpy 2.4.4): identical inputs produce these exact f64
    // values. Using `==` (not approx) to verify bit-for-bit parity.
    let a = Segment {
        time_from: ts(2026, 5, 1, 12, 0, 0),
        time_to: ts(2026, 5, 1, 12, 30, 0),
        level_from: 100.0,
        level_to: 100.0,
    };
    let b = Segment {
        time_from: ts(2026, 5, 1, 12, 10, 0),
        time_to: ts(2026, 5, 1, 12, 20, 0),
        level_from: 50.0,
        level_to: 80.0,
    };
    let c = Segment {
        time_from: ts(2026, 5, 1, 12, 15, 0),
        time_to: ts(2026, 5, 1, 12, 45, 0),
        level_from: 200.0,
        level_to: 100.0,
    };
    let d = Segment {
        time_from: ts(2026, 5, 1, 12, 0, 0),
        time_to: ts(2026, 5, 1, 12, 25, 0),
        level_from: 90.0,
        level_to: 90.0,
    };
    let segments = [a, b, c, d];
    let keys: Vec<Vec<SortKey>> = [
        ts(2026, 5, 1, 12, 0, 0),
        ts(2026, 5, 1, 12, 10, 0),
        ts(2026, 5, 1, 12, 30, 0),
        ts(2026, 5, 1, 12, 5, 0),
    ]
    .iter()
    .map(|k| vec![SortKey::Time(*k)])
    .collect();
    let idx = grid(ts(2026, 5, 1, 12, 0, 0), 5, 12);
    let out = interp_segments(&segments, &keys, &idx);
    let expected = [
        90.0,
        90.0,
        50.0,
        200.0,
        183.33333333333334,
        166.66666666666669,
        150.0,
        133.33333333333334,
        116.66666666666666,
        f64::NAN,
        f64::NAN,
        f64::NAN,
    ];
    assert_eq!(out.len(), expected.len());
    for (i, (got, want)) in out.iter().zip(expected).enumerate() {
        if want.is_nan() {
            assert!(got.is_nan(), "index {i}: expected NaN, got {got}");
        } else {
            assert_eq!(got.to_bits(), want.to_bits(), "index {i}");
        }
    }
}