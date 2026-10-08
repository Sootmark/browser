//! Lines compared with plaso's, as many times as plaso has each.

use std::collections::BTreeMap;

/// Each line and how many times it occurs.
fn counted<'l>(lines: impl IntoIterator<Item = &'l str>) -> BTreeMap<&'l str, usize> {
    let mut counts = BTreeMap::new();
    for line in lines {
        *counts.entry(line).or_insert(0) += 1;
    }
    counts
}

/// That `got` holds the lines of `expected`, each as many times; the first
/// that differ are shown, with their counts.
pub fn assert_same_lines(got: &[String], expected: &[&str]) {
    let got = counted(got.iter().map(String::as_str));
    let expected = counted(expected.iter().copied());
    let differ = |of: &BTreeMap<&str, usize>, than: &BTreeMap<&str, usize>| {
        of.iter()
            .filter(|(line, count)| than.get(*line) != Some(count))
            .map(|(line, count)| format!("{count}x {line}"))
            .take(10)
            .collect::<Vec<_>>()
    };
    let (missing, extra) = (differ(&expected, &got), differ(&got, &expected));
    assert!(
        missing.is_empty() && extra.is_empty(),
        "expected {missing:#?}\ngot {extra:#?}"
    );
}
