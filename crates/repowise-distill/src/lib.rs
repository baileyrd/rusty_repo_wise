//! Reversible output distillation -- the core of `repowise distill`.
//!
//! Runs a command, drops the noise, keeps everything a reader scanning
//! for problems needs, and stores what it dropped so nothing is ever
//! actually lost. See [`filter`] for the invariants and [`store`] for
//! why storage happens before any marker is rendered.
//!
//! # What this crate deliberately doesn't do
//!
//! No per-tool filters yet beyond the general engine. The reference
//! ships eleven (pytest, cargo, eslint, terraform, ...); those are
//! follow-up work, and the shape they plug into is [`filter::distill`].
//! What lands here is the part everything else depends on: the engine,
//! the store, and the marker format that `expand`, the rewrite hook and
//! the savings report all read.

pub mod filter;
pub mod ledger;
pub mod rewrite;
pub mod store;

pub use filter::{distill, Distilled};
pub use rewrite::{decide, Decision, SkipReason};
pub use store::{parse_ref, Missing, Store};

/// Render the inline marker that stands in for omitted content.
///
/// Carries the ref, how much was dropped, and -- crucially -- the exact
/// command to get it back. A marker that only said "230 lines omitted"
/// would be a statement of loss; naming the recovery command is what
/// makes it a statement of relocation.
pub fn marker(reference: &str, omitted_lines: usize) -> String {
    format!(
        "[repowise#{reference}: {omitted_lines} line(s) omitted; \
         restore: repowise expand {reference}]"
    )
}

/// The result of distilling one command's output.
pub struct Rendered {
    pub text: String,
    /// `None` when the output was printed unchanged.
    pub reference: Option<String>,
    pub omitted_lines: usize,
}

/// Distill `raw` and store what was dropped, falling back to raw output
/// on any problem.
///
/// The fallback is not an error path to be tidied away later -- it is
/// the contract. A filter bug, a full disk, or a read-only store must
/// all produce the original output, because "no compaction" is a fine
/// outcome and "wrong output" never is.
pub fn render(raw: &str, store: &Store) -> Rendered {
    let unchanged = || Rendered {
        text: raw.to_string(),
        reference: None,
        omitted_lines: 0,
    };

    let distilled = distill(raw);
    if distilled.is_lossless() {
        return unchanged();
    }

    let omitted_text = distilled.omitted.join("\n");
    // Storage first. A marker whose content isn't on disk is a broken
    // promise, so a store failure means we print the original instead.
    let Ok(reference) = store.put(&omitted_text) else {
        return unchanged();
    };

    let mut text = distilled.kept.join("\n");
    text.push('\n');
    text.push_str(&marker(&reference, distilled.omitted.len()));

    // Net-positive check, counting the marker. Compacting 12 lines into
    // 11 plus a marker is a loss dressed as a saving.
    if text.len() >= raw.len() {
        return unchanged();
    }

    Rendered {
        text,
        reference: Some(reference),
        omitted_lines: distilled.omitted.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("omissions"));
        (dir, store)
    }

    #[test]
    fn a_marker_names_the_command_that_reverses_it() {
        let m = marker("a1b2c3d4e5f6", 230);
        assert!(m.contains("repowise#a1b2c3d4e5f6"));
        assert!(
            m.contains("repowise expand a1b2c3d4e5f6"),
            "a marker must say how to get the content back: {m}"
        );
        assert!(m.contains("230"));
    }

    /// End-to-end reversibility: whatever the rendering drops, the
    /// store hands back verbatim.
    #[test]
    fn dropped_content_round_trips_through_the_store() {
        let (_d, store) = store();
        let mut lines: Vec<String> = (0..60).map(|i| format!("test t{i} ... ok")).collect();
        lines.push("error: the one that matters".to_string());
        let raw = lines.join("\n");

        let rendered = render(&raw, &store);
        let reference = rendered.reference.expect("should have distilled");

        assert!(rendered.text.contains("error: the one that matters"));
        assert!(rendered.text.contains(&reference));

        let restored = store.get(&reference).unwrap();
        for line in restored.lines() {
            assert!(
                raw.contains(line),
                "expanded content must come from the original: {line:?}"
            );
        }
        // Kept + restored covers every original line.
        let restored_count = restored.lines().count();
        let kept_count = rendered.text.lines().count() - 1; // minus the marker
        assert_eq!(kept_count + restored_count, raw.lines().count());
    }

    #[test]
    fn short_output_is_printed_unchanged_with_no_ref() {
        let (_d, store) = store();
        let raw = "just\na\nfew\nlines\n";
        let rendered = render(raw, &store);
        assert_eq!(rendered.text, raw);
        assert!(rendered.reference.is_none());
        assert_eq!(rendered.omitted_lines, 0);
    }

    /// The net-positive rule: if the marker costs more than the
    /// omission saves, print the original.
    #[test]
    fn a_non_improving_distillation_falls_back_to_raw() {
        let (_d, store) = store();
        // Long enough to attempt distillation, but the omitted lines
        // are so short that the marker outweighs them.
        let raw = (0..MIN_LINES_PLUS)
            .map(|_| "ok")
            .collect::<Vec<_>>()
            .join("\n");
        let rendered = render(&raw, &store);
        assert_eq!(rendered.text, raw, "should not have grown the output");
        assert!(rendered.reference.is_none());
    }

    const MIN_LINES_PLUS: usize = 21;

    /// A store it cannot write to must not stop the command from
    /// producing output.
    #[test]
    fn an_unwritable_store_degrades_to_raw_output() {
        // A store rooted at a path that exists as a *file* can never be
        // created as a directory.
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, "not a directory").unwrap();
        let store = Store::open(blocker.join("omissions"));

        let raw = (0..60)
            .map(|i| format!("test t{i} ... ok"))
            .collect::<Vec<_>>()
            .join("\n");
        let rendered = render(&raw, &store);

        assert_eq!(
            rendered.text, raw,
            "a store failure must fall back to raw, never to a dangling marker"
        );
        assert!(rendered.reference.is_none());
    }
}
