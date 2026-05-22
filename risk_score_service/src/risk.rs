//! Wave + wind risk scoring (threshold tables and per-chunk computation).

use std::collections::HashMap;

use ndarray::ArrayD;

/// Lower bounds for bins: `[base, low, medium, high]` — scores 1–4 use the same logic as `riskfct.py`.
#[derive(Clone, Copy)]
pub struct Thresholds {
    pub wave: [f32; 4],
    pub wind: [f32; 4],
}

/// Ship types and thresholds aligned with `reference_script/riskfct.py`.
pub const SHIP_MODELS: &[(&str, Thresholds)] = &[
    (
        "nelayan",
        Thresholds {
            wave: [0.0, 0.5, 1.25, 2.5],
            wind: [0.0, 10.0, 15.0, 21.0],
        },
    ),
    (
        "tongkang",
        Thresholds {
            wave: [0.0, 1.0, 2.0, 3.0],
            wind: [0.0, 15.0, 21.0, 27.0],
        },
    ),
    (
        "ferry",
        Thresholds {
            wave: [0.0, 1.5, 2.5, 4.0],
            wind: [0.0, 21.0, 27.0, 33.0],
        },
    ),
    (
        "kapalbesar",
        Thresholds {
            wave: [0.0, 2.0, 3.5, 5.0],
            wind: [0.0, 27.0, 33.0, 40.0],
        },
    ),
];

#[inline]
pub fn wind_speed_knots(u: f32, v: f32) -> f32 {
    (f64::from(u) * f64::from(u) + f64::from(v) * f64::from(v)).sqrt() as f32
}

#[inline]
pub fn score_from_threshold(value: f32, th: &[f32; 4]) -> u8 {
    if value < th[1] {
        1
    } else if value < th[2] {
        2
    } else if value < th[3] {
        3
    } else {
        4
    }
}

#[inline]
pub fn combine_risk(wave_score: u8, wind_score: u8) -> u8 {
    let total = i32::from(wave_score) + i32::from(wind_score);
    if total < 4 {
        1
    } else if total < 6 {
        2
    } else if total < 8 {
        3
    } else {
        4
    }
}

#[inline]
fn is_hs_invalid(hs: f32, fill: f32) -> bool {
    hs.is_nan() || (if fill.is_nan() { false } else { hs == fill })
}

/// Fills `out` with keys `risk_nelayan`, …, each an `ArrayD<u8>` matching `hs` shape.
///
/// `uwnd` and `vwnd` are expected in **knots** (INACAWO_WAVES and INAWAVES).
pub fn compute_chunk_risks(
    hs: &ArrayD<f32>,
    uw: &ArrayD<f32>,
    vw: &ArrayD<f32>,
    hs_fill: f32,
    out: &mut HashMap<String, ArrayD<u8>>,
) {
    let mut wind = ArrayD::zeros(uw.raw_dim());
    ndarray::Zip::from(&mut wind)
        .and(uw)
        .and(vw)
        .for_each(|w, &u, &v| {
            *w = wind_speed_knots(u, v);
        });

    for (ship, th) in SHIP_MODELS {
        let name = format!("risk_{ship}");
        let mut risk = ArrayD::zeros(hs.raw_dim());
        ndarray::Zip::from(&mut risk)
            .and(hs)
            .and(&wind)
            .for_each(|r, &h, &wspd| {
                if is_hs_invalid(h, hs_fill) || wspd.is_nan() {
                    *r = 0;
                    return;
                }
                let sw = score_from_threshold(h, &th.wave);
                let ww = score_from_threshold(wspd, &th.wind);
                *r = combine_risk(sw, ww);
            });
        out.insert(name, risk);
    }
}
