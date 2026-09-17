//! Benchmark metrics: P/R/F1 per (language, tier) and ECE on `p_true`.

pub mod agent_ab;

use graphengine_parsing::domain::{p_true, ProvenanceSource};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrF1 {
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
}

pub fn pr_f1(true_pos: f64, false_pos: f64, false_neg: f64) -> PrF1 {
    let precision = if true_pos + false_pos == 0.0 {
        0.0
    } else {
        true_pos / (true_pos + false_pos)
    };
    let recall = if true_pos + false_neg == 0.0 {
        0.0
    } else {
        true_pos / (true_pos + false_neg)
    };
    let f1 = if precision + recall == 0.0 {
        0.0
    } else {
        2.0 * precision * recall / (precision + recall)
    };
    PrF1 {
        precision,
        recall,
        f1,
    }
}

/// Expected Calibration Error over equal-width bins of p_true.
pub fn ece(pairs: &[(f64, bool)], bins: usize) -> f64 {
    if pairs.is_empty() || bins == 0 {
        return 0.0;
    }
    let mut sum = 0.0;
    let n = pairs.len() as f64;
    for b in 0..bins {
        let lo = b as f64 / bins as f64;
        let hi = (b + 1) as f64 / bins as f64;
        let in_bin: Vec<&(f64, bool)> = pairs
            .iter()
            .filter(|(p, _)| *p >= lo && (if b + 1 == bins { *p <= hi } else { *p < hi }))
            .collect();
        if in_bin.is_empty() {
            continue;
        }
        let acc = in_bin.iter().filter(|(_, y)| *y).count() as f64 / in_bin.len() as f64;
        let conf = in_bin.iter().map(|(p, _)| *p).sum::<f64>() / in_bin.len() as f64;
        sum += (in_bin.len() as f64 / n) * (acc - conf).abs();
    }
    sum
}

pub fn prior_call_p_true(language: &str, source: ProvenanceSource) -> f64 {
    p_true(language, source, "Call")
}

pub const ECE_RELEASE_GATE: f64 = 0.10;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perfect_calibration_is_zero_ece() {
        let pairs = vec![(1.0, true), (0.0, false), (1.0, true)];
        assert!(ece(&pairs, 2) < 0.01);
    }

    #[test]
    fn ece_gate_constant() {
        assert_eq!(ECE_RELEASE_GATE, 0.10);
    }

    #[test]
    fn pr_f1_balanced() {
        let m = pr_f1(8.0, 2.0, 2.0);
        assert!((m.precision - 0.8).abs() < 1e-9);
        assert!((m.recall - 0.8).abs() < 1e-9);
    }
}
