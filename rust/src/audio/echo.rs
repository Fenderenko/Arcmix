//! Telling "foreign" sound in an app session apart.
//!
//! Sometimes the level meter of one app moves together with another app's sound: e.g. Discord in
//! an empty voice channel "sounds" exactly like a video in Chrome. For ducking that is a false
//! alarm. So every session keeps a short history of levels (dB) and, when a chat session seems to
//! "talk", it is checked whether it repeats another app's level (correlation with a small time
//! shift). A real voice in chat does not correlate with game or video sound.

use std::collections::HashMap;

/// Measurements in the history.
pub const ECHO_LEN: usize = 24;
/// Quieter is silence anyway.
pub const ECHO_MIN_DB: f64 = -80.0;
/// Correlation from which the sound counts as a copy of another.
pub const ECHO_CORR: f64 = 0.85;
/// At least this many measurements to compare.
pub const ECHO_MIN_N: usize = 10;

#[derive(Clone, Copy, Default)]
pub struct LvlHist {
    v: [f64; ECHO_LEN],
    pub n: usize,
    pos: usize,
}

impl LvlHist {
    pub fn push(&mut self, db: f64) {
        self.v[self.pos] = db.clamp(ECHO_MIN_DB, 0.0);
        self.pos = (self.pos + 1) % ECHO_LEN;
        if self.n < ECHO_LEN {
            self.n += 1;
        }
    }

    /// i = 0 — the newest measurement.
    pub fn at(&self, i: usize) -> f64 {
        self.v[(self.pos + 2 * ECHO_LEN - 1 - i) % ECHO_LEN]
    }
}

/// The largest correlation of levels a with levels b when a lags b by 0…max_lag measurements.
/// -1 — nothing to compare (few measurements or one of the levels is nearly flat).
pub fn echo_of(a: &LvlHist, b: &LvlHist, max_lag: usize) -> f64 {
    let mut best = -1.0f64;
    for lag in 0..=max_lag {
        let m = a.n.min(b.n.saturating_sub(lag));
        if b.n < lag || m < ECHO_MIN_N {
            continue;
        }
        let (mut sa, mut sb) = (0.0, 0.0);
        for i in 0..m {
            sa += a.at(i);
            sb += b.at(i + lag);
        }
        let (ma, mb) = (sa / m as f64, sb / m as f64);
        let (mut cov, mut va, mut vb) = (0.0, 0.0, 0.0);
        for i in 0..m {
            let (x, y) = (a.at(i) - ma, b.at(i + lag) - mb);
            cov += x * y;
            va += x * x;
            vb += y * y;
        }
        if va / (m as f64) < 1.0 || vb / (m as f64) < 1.0 {
            continue; // a nearly flat line — correlation tells nothing
        }
        best = best.max(cov / (va * vb).sqrt());
    }
    best
}

/// The session level (h, now db) repeats the whole device sound (ep), and that consists of other
/// apps' sound (post — the loudest of them, with its volume). That is how a session behaves that
/// "listens" to the headset output instead of playing its own sound.
pub fn mirror_evidence(h: Option<&LvlHist>, ep: Option<&LvlHist>, post: Option<&LvlHist>, db: f64) -> bool {
    let (Some(h), Some(ep), Some(post)) = (h, ep, post) else { return false };
    if ep.n < ECHO_MIN_N || post.n < ECHO_MIN_N {
        return false;
    }
    if db < -50.0 || ep.at(0) < -50.0 || post.at(0) < ep.at(0) - 3.0 {
        return false; // quiet, or the device sound is the chat itself, not other apps
    }
    echo_of(h, ep, 2) >= 0.95 && echo_of(post, ep, 2) >= 0.85
}

/// The device level above the sum of other apps' levels on it (an estimate of the chat voice when
/// the chat session meter shows the whole device sound).
pub fn residual_db(ep: &HashMap<String, f64>, sum: &HashMap<String, f64>, dev: &str) -> f64 {
    let Some(&e) = ep.get(dev).filter(|v| !v.is_nan()) else { return -120.0 }; // NaN — no level this step
    let el = 10f64.powf(e / 20.0);
    let ol = sum.get(dev).copied().unwrap_or(0.0);
    if el <= ol * 1.4 {
        return -120.0; // within what other apps play
    }
    20.0 * (el - ol).log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Small deterministic generator for the tests (xorshift).
    struct Rng(u64);
    impl Rng {
        fn new(seed: u64) -> Self {
            Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1)
        }
        fn f(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    /// A chat session repeating another app's sound (at once or a bit later) is a copy.
    #[test]
    fn detects_copy() {
        let mut r = Rng::new(1);
        let src: Vec<f64> = (0..60).map(|i| if i > 30 { -30.0 + 12.0 * r.f() } else { -80.0 }).collect();
        for lag in [0usize, 1, 3] {
            let (mut other, mut chat) = (LvlHist::default(), LvlHist::default());
            for i in 0..src.len() {
                other.push(src[i]);
                chat.push(if i >= lag { src[i - lag] - 2.0 } else { -80.0 });
            }
            let c = echo_of(&chat, &other, 4);
            assert!(c >= ECHO_CORR, "lag {lag}: corr {c:.2}, want copy");
        }
    }

    /// A real voice in chat over a game sounding all the time is not a copy.
    #[test]
    fn ignores_real_voice() {
        let mut r = Rng::new(2);
        let mut worst = -1.0f64;
        for _ in 0..200 {
            let (mut game, mut chat) = (LvlHist::default(), LvlHist::default());
            for i in 0..60 {
                game.push(-25.0 + 10.0 * r.f());
                chat.push(if i > 45 { -28.0 + 14.0 * ((i as f64) * 0.9 + r.f()).sin().abs() } else { -80.0 });
            }
            worst = worst.max(echo_of(&chat, &game, 4));
        }
        assert!(worst < ECHO_CORR, "real voice looked like a copy: corr {worst:.2}");
    }

    /// Silence or a flat level — nothing to compare.
    #[test]
    fn flat() {
        let (mut a, mut b) = (LvlHist::default(), LvlHist::default());
        for i in 0..30 {
            a.push(-80.0);
            b.push(-30.0 + (i % 5) as f64);
        }
        assert_eq!(echo_of(&a, &b, 4), -1.0);
    }

    fn mirror_run(seed: u64, ticks: usize, g: &dyn Fn(&mut Rng, usize) -> (f64, f64, f64)) -> usize {
        let mut r = Rng::new(seed);
        let (mut h, mut ep, mut post) = (LvlHist::default(), LvlHist::default(), LvlHist::default());
        let mut n = 0;
        for i in 0..ticks {
            let (c, e, p) = g(&mut r, i);
            h.push(c);
            ep.push(e);
            post.push(p);
            if mirror_evidence(Some(&h), Some(&ep), Some(&post), c) {
                n += 1;
            }
        }
        n
    }

    #[test]
    fn mirror_of_music_while_ducking() {
        // music whose volume we move ourselves (ducking), and the chat session repeats all sound
        let n = mirror_run(3, 400, &|r, i| {
            let pre = -12.0 + 8.0 * r.f();
            let vol = if (i / 40) % 2 == 1 { -14.0 } else { 0.0 };
            let p = pre + vol;
            let e = p + 0.3 * r.f();
            (e - 0.5 + 0.4 * r.f(), e, p)
        });
        assert!(n >= 200, "mirror recognised only {n} of 400 ticks");
    }

    #[test]
    fn mirror_not_real_voice() {
        fn voice(r: &mut Rng, i: usize) -> f64 {
            if (i / 12) % 3 == 2 { -80.0 + r.f() } else { -22.0 + 12.0 * ((i as f64) * 0.7 + r.f()).sin().abs() }
        }
        let cases: [(&str, &dyn Fn(&mut Rng, usize) -> (f64, f64, f64)); 4] = [
            ("voice alone", &|r, i| {
                let v = voice(r, i);
                (v, v, -120.0)
            }),
            ("voice over louder game", &|r, i| {
                let (v, g) = (voice(r, i), -10.0 + 6.0 * r.f());
                (v, v.max(g) + 1.0, g)
            }),
            ("voice over quiet game", &|r, i| {
                let (v, g) = (voice(r, i), -40.0 + 6.0 * r.f());
                (v, v.max(g) + 0.5, g)
            }),
            ("voice and game equally loud", &|r, i| {
                let (v, g) = (voice(r, i), -20.0 + 10.0 * r.f());
                (v, v.max(g) + 1.0, g)
            }),
        ];
        for (name, g) in cases {
            let worst = (0..50).map(|seed| mirror_run(seed, 400, g)).max().unwrap();
            assert!(worst < 12, "{name}: real voice would be taken for a mirror ({worst} ticks)");
        }
    }

    #[test]
    fn residual() {
        let lin = |db: f64| 10f64.powf(db / 20.0);
        let m = |k: &str, v: f64| HashMap::from([(k.to_string(), v)]);
        assert_eq!(residual_db(&m("h", -7.0), &m("h", lin(-7.0)), "h"), -120.0, "music only");
        assert_eq!(residual_db(&m("h", -4.0), &m("h", 2.0 * lin(-10.0)), "h"), -120.0, "two apps");
        assert!(residual_db(&m("h", -9.0), &m("h", lin(-30.0)), "h") >= -12.0, "voice over quiet music");
        assert!((residual_db(&m("h", -20.0), &HashMap::new(), "h") + 20.0).abs() < 0.01, "alone");
        assert_eq!(residual_db(&HashMap::new(), &HashMap::new(), "h"), -120.0, "no meter");
    }
}
