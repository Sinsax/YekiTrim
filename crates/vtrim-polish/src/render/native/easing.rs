//! 与 `templates/index.html` Canvas 渲染器 `E` 对象一致的 easing 函数。

pub fn pow2_out(t: f64) -> f64 {
    1.0 - (1.0 - t) * (1.0 - t)
}

pub fn pow2_in(t: f64) -> f64 {
    t * t
}

pub fn pow2_in_out(t: f64) -> f64 {
    if t < 0.5 {
        2.0 * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
    }
}

pub fn sine_out(t: f64) -> f64 {
    (t * std::f64::consts::PI / 2.0).sin()
}

pub fn sine_in(t: f64) -> f64 {
    1.0 - (t * std::f64::consts::PI / 2.0).cos()
}

pub fn sine_in_out(t: f64) -> f64 {
    (-(std::f64::consts::PI * t).cos() + 1.0) / 2.0
}

pub fn back_out(t: f64, s: Option<f64>) -> f64 {
    let s = s.unwrap_or(1.70158);
    let t = t - 1.0;
    t * t * ((s + 1.0) * t + s) + 1.0
}

pub fn power1_out(t: f64) -> f64 {
    t
}

pub fn none(t: f64) -> f64 {
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_js_endpoints() {
        assert!((pow2_out(0.0) - 0.0).abs() < 1e-12);
        assert!((pow2_out(1.0) - 1.0).abs() < 1e-12);
        assert!((pow2_out(0.5) - 0.75).abs() < 1e-12);

        assert!((sine_out(0.0) - 0.0).abs() < 1e-12);
        assert!((sine_out(1.0) - 1.0).abs() < 1e-12);

        assert!((back_out(0.0, Some(3.0)) - 0.0).abs() < 1e-12);
        assert!((back_out(1.0, Some(3.0)) - 1.0).abs() < 1e-12);
        // JS back.out(3) 在 t=0.3 附近产生轻微 overshoot
        let v = back_out(0.3, Some(3.0));
        assert!(
            v > 1.0 || (v - 1.0).abs() < 1e-9,
            "expected overshoot around 0.3, got {v}"
        );
    }

    #[test]
    fn sine_in_out_matches_js() {
        // JS: -(Math.cos(Math.PI * t) - 1) / 2
        for (t, expected) in [(0.0, 0.0), (0.5, 0.5), (1.0, 1.0)] {
            let actual = sine_in_out(t);
            assert!((actual - expected).abs() < 1e-12, "t={t} actual={actual}");
        }
    }
}
