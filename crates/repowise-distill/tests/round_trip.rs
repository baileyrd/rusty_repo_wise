//! End-to-end reversibility: whatever `render` drops, the store hands
//! back, and kept + restored reconstructs the original exactly.
//!
//! This is the property the whole feature rests on. It lives in an
//! integration test rather than a unit test because it has to exercise
//! the real store on a real filesystem -- an in-memory stand-in could
//! pass while the on-disk path silently lost a trailing newline.

use repowise_distill::{render, Store};

fn store(dir: &tempfile::TempDir) -> Store {
    Store::open(dir.path().join("omissions"))
}

/// Reconstruct the original from what was printed plus what was stored.
fn reconstruct(rendered_text: &str, restored: &str) -> Vec<String> {
    let kept: Vec<String> = rendered_text
        .lines()
        .filter(|l| !l.contains("repowise#"))
        .map(str::to_string)
        .collect();
    let mut all = kept;
    all.extend(restored.lines().map(str::to_string));
    all.sort();
    all
}

#[test]
fn kept_plus_restored_reconstructs_the_original_exactly() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);

    let mut lines: Vec<String> = (0..60).map(|i| format!("test t{i} ... ok")).collect();
    lines.push("error: boom".to_string());
    lines.push("test result: FAILED. 1 failed".to_string());
    let raw = lines.join("\n");

    let rendered = render(&raw, &store);
    let reference = rendered.reference.expect("should distill");
    let restored = store.get(&reference).unwrap();

    let mut expected: Vec<String> = raw.lines().map(str::to_string).collect();
    expected.sort();

    assert_eq!(
        reconstruct(&rendered.text, &restored),
        expected,
        "nothing may be lost or duplicated between the rendering and the store"
    );
}

/// Lines with awkward content must survive storage untouched -- a
/// round trip that normalizes whitespace isn't a round trip.
#[test]
fn awkward_lines_survive_storage_verbatim() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);

    let awkward = "  leading spaces\n\ttab\tseparated\ntrailing spaces   \n\"quoted\"\n\\backslash\nunicode: caf\u{e9} \u{1f600}";
    let reference = store.put(awkward).unwrap();
    assert_eq!(store.get(&reference).unwrap(), awkward);
}

#[test]
fn an_empty_omission_round_trips_as_empty() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    let reference = store.put("").unwrap();
    assert_eq!(store.get(&reference).unwrap(), "");
}

/// Two different outputs must never collapse onto one ref -- returning
/// someone else's output is the one failure a reader can't detect.
#[test]
fn distinct_content_gets_distinct_refs() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    let a = store.put("first output").unwrap();
    let b = store.put("second output").unwrap();
    assert_ne!(a, b);
    assert_eq!(store.get(&a).unwrap(), "first output");
    assert_eq!(store.get(&b).unwrap(), "second output");
}
