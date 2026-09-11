pub mod analysis;
pub mod coverage;

/// Canonical CRAP score. Coverage is a fraction between zero and one.
pub fn crap_score(complexity: usize, coverage: f64) -> f64 {
    let cc = complexity as f64;
    cc * cc * (1.0 - coverage).powi(3) + cc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_scores() {
        assert_eq!(crap_score(4, 0.0), 20.0);
        assert_eq!(crap_score(4, 1.0), 4.0);
        assert_eq!(crap_score(4, 0.5), 6.0);
        assert!((crap_score(7, 0.45) - 15.152375).abs() < 1e-10);
    }
}
