//! One family-wise error rate for everything a verdict tests. Each gate on its own (your
//! corrections, the model's thinking, each agent's failed calls) lets chance through one time in
//! ten, and a verdict names a pattern when any of them does, so on random data the report would
//! cry wolf far more often than that. The Holm-Bonferroni step-down rule keeps the chance of any
//! false alarm at `bands::MAX_P` across all of them, and needs only the p-values.

use super::bands::MAX_P;

/// Which tests survive. `ps` holds each test's p, None only where there is no such test at all (an
/// agent that makes no tool calls has no test of its failed calls): a test that exists but cannot
/// be told is given a p of 1 by its caller, so the family does not depend on how the data looks.
/// The smallest p must beat `MAX_P` over the family's size, the
/// next `MAX_P` over one less, and so on, stopping at the first that fails.
pub fn survivors(ps: &[Option<f64>]) -> Vec<bool> {
    let mut order: Vec<(usize, f64)> = ps
        .iter()
        .enumerate()
        .filter_map(|(i, p)| Some((i, (*p)?)))
        .collect();
    order.sort_by(|a, b| a.1.total_cmp(&b.1));
    let family = order.len();
    let mut out = vec![false; ps.len()];
    for (rank, (i, p)) in order.into_iter().enumerate() {
        if p >= MAX_P / (family - rank) as f64 {
            break;
        }
        out[i] = true;
    }
    out
}

#[cfg(test)]
mod tests;
