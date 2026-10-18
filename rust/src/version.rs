//! Version comparison ("1.10" > "1.9"; a "-dev"/"-alpha" suffix is older than none).

pub fn newer(a: &str, b: &str) -> bool {
    let parse = |v: &str| {
        let v = v.trim().trim_start_matches('v');
        let pre = v.contains('-');
        let core = v.split('-').next().unwrap_or("");
        (core.split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>(), pre)
    };
    let (pa, pre_a) = parse(a);
    let (pb, pre_b) = parse(b);
    for i in 0..pa.len().max(pb.len()) {
        let (x, y) = (pa.get(i).copied().unwrap_or(0), pb.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    pre_b && !pre_a
}

#[cfg(test)]
mod tests {
    use super::newer;

    #[test]
    fn versions() {
        assert!(newer("1.10", "1.9"));
        assert!(!newer("1.9", "1.10"));
        assert!(newer("1.10.1", "1.10"));
        assert!(newer("1.10.0", "1.10.0-dev"));
        assert!(!newer("1.10.0-dev", "1.10.0"));
        assert!(newer("2.0.0-alpha.1", "1.10.1"));
        assert!(!newer("1.10.1", "1.10.1"));
    }
}
