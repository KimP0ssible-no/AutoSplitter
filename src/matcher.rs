//! Keyword matching against OCR text, on Unicode codepoints with ASCII-only case folding.

fn lower(s: &str) -> Vec<char> {
    s.chars().map(|c| c.to_ascii_lowercase()).collect()
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + usize::from(ca != cb));
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

fn ratio(a: &[char], b: &[char]) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    1.0 - levenshtein(a, b) as f32 / a.len().max(b.len()) as f32
}

/// Similarity of two texts, 0.0 to 1.0.
pub fn fuzzy_ratio(a: &str, b: &str) -> f32 {
    ratio(&lower(a), &lower(b))
}

fn window_match(h: &[char], n: &[char], threshold: f32) -> bool {
    h.len() >= n.len() && h.windows(n.len()).any(|w| w == n || ratio(w, n) >= threshold)
}

/// True when `needle` appears in `haystack`, allowing OCR typos down to `threshold`.
/// A second pass ignores whitespace, because OCR drops spaces between tight letters.
pub fn fuzzy_contains(haystack: &str, needle: &str, threshold: f32) -> bool {
    if needle.is_empty() {
        return true;
    }
    let (h, n) = (lower(haystack), lower(needle));
    if window_match(&h, &n, threshold) {
        return true;
    }
    let strip = |v: &[char]| -> Vec<char> { v.iter().copied().filter(|&c| !matches!(c, ' ' | '\t' | '\r' | '\n' | '\u{3000}')).collect() };
    let (h, n) = (strip(&h), strip(&n));
    !n.is_empty() && window_match(&h, &n, threshold)
}

/// Keeps Han ideographs, CJK punctuation and whitespace: the Chinese model misreads icons as Latin letters.
pub fn keep_cjk_only(s: &str) -> String {
    s.chars()
        .filter(|&c| matches!(c, ' ' | '\t' | '\r' | '\n' | '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}' | '\u{3000}'..='\u{303F}'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_like_the_legacy_matcher() {
        assert!(fuzzy_contains("ENTER THE ESTATE\nFind a way", "enter the estate", 0.92));
        // OCR dropped a space: the whitespace-free pass catches it.
        assert!(fuzzy_contains("Secure the landing area\nFollow theTunnel", "Follow the Tunnel", 0.92));
        // One typo in a long keyword passes, a different objective does not.
        assert!(fuzzy_contains("Neutralize the Lovebirds", "Neutralize the Lovebirdz", 0.92));
        assert!(!fuzzy_contains("Find and save the survivors Search the Bar", "Find and save the survivors Secure the Area", 0.92));
        assert!(!fuzzy_contains("Hard", "Heroic", 0.92));
        assert!(fuzzy_contains("anything", "", 0.92));
        assert!(fuzzy_contains("确保着陆区安全 前往草坪", "确保着陆区安全前往草坪", 0.92));
        assert_eq!(fuzzy_ratio("ABC", "abc"), 1.0);
        assert_eq!(keep_cjk_only("M 进入庄园！"), " 进入庄园");
    }
}
