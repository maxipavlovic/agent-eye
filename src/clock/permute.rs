//! The chance test for the bad hours. The hours are picked because they look worst, so testing them
//! as if they had been chosen in advance would find a pattern in noise most of the time. Instead the
//! whole procedure (pick the hours, measure how much worse they are) is run again on data where the
//! corrections have been dealt out at random over the prompts. The pattern counts when random
//! dealing rarely does as well as the real data (`MAX_P`). The same is done for the model's
//! thinking. Every shuffle uses a fixed seed, so a report never changes between two looks at the
//! same data.
//!
//! Each prompt (and each thought) is treated as independent, which clusters of related prompts in
//! one sitting are not quite: the test errs a little towards finding a pattern.

use super::bands::{self, Hour};

/// How many random re-dealings the test uses. The smallest p it can give is 1 in this many.
pub const PERMUTATIONS: usize = 1000;
/// Thinking takes more work per round (medians of every thought), so it gets fewer.
const THINK_PERMUTATIONS: usize = 300;
/// The seed of every shuffle.
pub const SEED: u64 = 0x5EED_C10C_4B1D_0001;

/// A small seeded generator (splitmix64): the standard library has none, and the shuffles need
/// nothing stronger.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `0..n` (`n` above 0).
    pub fn below(&mut self, n: usize) -> usize {
        ((u128::from(self.next_u64()) * n as u128) >> 64) as usize
    }

    /// A number in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// How much worse the picked hours are: the two-proportion z of the picked hours against the rest,
/// 0 when no hours were picked, they hold too few corrections or they are not worse. `counts` is
/// `(prompts, corrections)` per hour; `mask` receives the picked hours.
fn band_stat(counts: &[(i64, i64)], mask: &mut [bool]) -> f64 {
    bands::candidate_mask(counts, mask);
    let (bad, rest) = band_sums(counts, mask);
    if bad.0 < bands::MIN_EVENTS || bands::rate(bad.0, bad.1) <= bands::rate(rest.0, rest.1) {
        return 0.0;
    }
    bands::proportion_z(bad, rest).unwrap_or(0.0)
}

/// `(corrections, prompts)` inside and outside the picked hours.
fn band_sums(counts: &[(i64, i64)], mask: &[bool]) -> ((i64, i64), (i64, i64)) {
    let (mut bad, mut rest) = ((0, 0), (0, 0));
    for ((n, c), picked) in counts.iter().zip(mask) {
        let side = if *picked { &mut bad } else { &mut rest };
        side.0 += c;
        side.1 += n;
    }
    (bad, rest)
}

/// How much more the worst hours are corrected than the other hours (as a ratio, at most
/// `MAX_RATIO`), when random dealing of the same corrections over the same prompts does as well
/// less than one time in ten. None when no hours stand out or chance explains them.
pub fn band_gate(hours: &[Hour], seed: u64) -> Option<f64> {
    let counts: Vec<(i64, i64)> = hours.iter().map(|h| (h.prompts, h.corrections)).collect();
    let mut mask = vec![false; counts.len()];
    let observed = band_stat(&counts, &mut mask);
    if observed <= 0.0 {
        return None;
    }
    let (bad, rest) = band_sums(&counts, &mask);
    let corrections = counts.iter().map(|(_, c)| *c).sum::<i64>() as usize;
    let mut slots: Vec<usize> = counts
        .iter()
        .enumerate()
        .flat_map(|(h, (n, _))| std::iter::repeat_n(h, *n as usize))
        .collect();
    let mut dealt: Vec<(i64, i64)> = counts.iter().map(|(n, _)| (*n, 0)).collect();
    let mut rng = Rng::new(seed);
    let mut as_good = 0;
    for _ in 0..PERMUTATIONS {
        dealt.iter_mut().for_each(|d| d.1 = 0);
        // the first `corrections` slots after a partial shuffle are a random choice of prompts
        for i in 0..corrections {
            let j = i + rng.below(slots.len() - i);
            slots.swap(i, j);
            dealt[slots[i]].1 += 1;
        }
        // a tolerance for the same z computed in a different order
        if band_stat(&dealt, &mut mask) >= observed - 1e-9 {
            as_good += 1;
            if too_common(as_good, PERMUTATIONS) {
                return None;
            }
        }
    }
    let (r1, r2) = (bands::rate(bad.0, bad.1), bands::rate(rest.0, rest.1));
    Some(if r2 == 0.0 {
        bands::MAX_RATIO
    } else {
        (r1 / r2).min(bands::MAX_RATIO)
    })
}

/// Has chance done as well often enough, in `as_good` of `rounds` shuffles (counting the real data
/// as one more), that p can no longer be under `MAX_P`?
fn too_common(as_good: usize, rounds: usize) -> bool {
    (as_good + 1) as f64 / (rounds + 1) as f64 >= bands::MAX_P
}

/// The chance that thoughts in the hours (`inside`) have a median this far from the median of the
/// others (as a ratio, either way) when the hours do not matter: the thoughts are dealt out again at
/// random and the medians compared. `None` when either side is empty or the others' median is 0.
/// Returns 1 early once p can no longer be under `MAX_P`.
pub fn median_shift_p(inside: &[f64], outside: &[f64], seed: u64) -> Option<f64> {
    let (n_in, mut pool) = (inside.len(), [inside, outside].concat());
    let original = pool.clone();
    let observed = shift(&mut pool.clone(), n_in)?;
    let mut rng = Rng::new(seed);
    let mut as_good = 0;
    for _ in 0..THINK_PERMUTATIONS {
        pool.copy_from_slice(&original);
        for i in 0..n_in {
            let j = i + rng.below(pool.len() - i);
            pool.swap(i, j);
        }
        if shift(&mut pool, n_in).is_some_and(|s| s >= observed - 1e-9) {
            as_good += 1;
            if too_common(as_good, THINK_PERMUTATIONS) {
                return Some(1.0);
            }
        }
    }
    Some((as_good + 1) as f64 / (THINK_PERMUTATIONS + 1) as f64)
}

/// How far apart the medians of `pool[..n_in]` and `pool[n_in..]` are, as the size of the log of
/// their ratio. Reorders each part.
fn shift(pool: &mut [f64], n_in: usize) -> Option<f64> {
    let (a, b) = pool.split_at_mut(n_in);
    let (ma, mb) = (median(a)?, median(b)?);
    (mb > 0.0).then(|| (ma.max(1e-9) / mb).ln().abs())
}

fn median(v: &mut [f64]) -> Option<f64> {
    let mid = v.len() / 2;
    (!v.is_empty()).then(|| *v.select_nth_unstable_by(mid, f64::total_cmp).1)
}

#[cfg(test)]
mod tests;
