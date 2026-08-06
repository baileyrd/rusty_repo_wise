//! `dry_violation` near-duplicate code detection via Rabin-Karp
//! rolling-hash substring matching over *tokenized* function/method
//! text — catches *partial* duplicates (a function that's mostly
//! identical to another with a few renamed variables or a tweaked
//! constant) that the existing exact-body-hash `DuplicateCode` marker
//! misses entirely: a single differing character breaks a hash match
//! there.
//!
//! Deliberately a **separate** `FindingKind::NearDuplicateCode` rather
//! than folding into `DuplicateCode` — the two answer different
//! questions ("identical" vs "mostly the same") and a pair already
//! caught by `DuplicateCode` (identical `body_hash`) is explicitly
//! excluded here so it's never reported twice under two different
//! finding kinds.
//!
//! Unlike every other marker in this crate, this one re-reads each
//! candidate symbol's source text fresh from disk — `Symbol` doesn't
//! carry raw body text by design (see `repowise-parser::metrics::body_hash`'s
//! own doc comment: only a hash is kept, not the text it was computed
//! from). This is the same "re-read from disk when we need real text"
//! tradeoff `repowise-mcp::get_symbol` and `repowise-adr`'s code-comment/
//! inline-marker sources already make elsewhere in this workspace,
//! rather than growing `Symbol`/`FileRecord` (and every one of their
//! ~40 construction sites workspace-wide) with a large text/hash-list
//! field just for this one marker. A file that's moved/deleted since
//! indexing degrades that file's contribution to empty rather than
//! failing the whole scan.
//!
//! **Tokenized, not raw-character, windows.** Splitting into identifier/
//! punctuation tokens first (rather than sliding a window over raw
//! normalized characters) matters because identifier renames change
//! *length* — `total` -> `sum` shifts every subsequent character
//! position, which would misalign every raw-character window from that
//! point on even though the code is otherwise identical. Windowing over
//! tokens instead means a single renamed identifier only invalidates the
//! windows containing that one token slot, not everything after it.
//!
//! Rabin-Karp bucketing (not brute-force all-pairs comparison): each
//! eligible symbol's token sequence is split into overlapping
//! `WINDOW_TOKENS`-token windows, each hashed with an incremental
//! rolling hash over the per-token hashes. Two symbols only become a
//! "candidate pair" if they share at least one window hash — pairs with
//! nothing in common never get compared at all. Candidate pairs are
//! then scored by what fraction of their (smaller) window count is
//! shared; pairs at or above `MIN_OVERLAP_RATIO` are reported.

use repowise_core::{RepoIndex, Symbol, SymbolKind};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

/// Rolling-hash window size, in tokens. Deliberately small: a
/// near-duplicate typically differs at a handful of scattered
/// identifier/constant tokens, and a wider window would have its
/// coverage cut by every one of those (see the module doc comment) —
/// tuned against a realistic renamed-variable fixture (see this
/// module's own tests) rather than picked arbitrarily.
const WINDOW_TOKENS: usize = 3;

/// Two symbols are reported as near-duplicates once the fraction of
/// their smaller window count that's shared reaches this threshold.
/// Tuned against the same fixture as `WINDOW_TOKENS`: comfortably above
/// the ~5% incidental overlap two unrelated functions share from common
/// punctuation/keyword tokens, comfortably below the ~60% two
/// realistic near-duplicates share.
const MIN_OVERLAP_RATIO: f64 = 0.5;

/// Rolling-hash polynomial base. Not cryptographic — same "trust a
/// cheap hash for equality" tradeoff `body_hash` already makes with
/// `DefaultHasher`; collision risk at this scale is negligible.
const RK_BASE: u64 = 1_000_003;

#[derive(Debug, Clone)]
pub struct NearDuplicateCandidate {
    pub file: PathBuf,
    pub symbol: String,
    pub line: usize,
    pub other_file: PathBuf,
    pub other_symbol: String,
    /// Fraction (0.0-1.0) of the smaller symbol's windows shared with
    /// the other.
    pub overlap_ratio: f64,
}

struct Eligible<'a> {
    sym: &'a Symbol,
    /// Deduplicated rolling-hash window values for this symbol's
    /// normalized text.
    windows: Vec<u64>,
}

/// Find every pair of `Function`/`Method` symbols across `index` whose
/// normalized text overlaps by at least `MIN_OVERLAP_RATIO`, excluding
/// pairs already caught by the exact-body-hash `DuplicateCode` marker.
/// Each flagged symbol gets its own `NearDuplicateCandidate` pointing at
/// the other half of the pair.
fn collect_eligible(index: &RepoIndex) -> Vec<Eligible<'_>> {
    let mut eligible: Vec<Eligible> = Vec::new();
    for file in &index.files {
        let Ok(source) = std::fs::read_to_string(&file.path) else {
            continue;
        };
        let lines: Vec<&str> = source.lines().collect();
        for sym in &file.symbols {
            if !matches!(sym.kind, SymbolKind::Function | SymbolKind::Method) {
                continue;
            }
            // Reuses body_hash's own "long enough to be a meaningful
            // signal" gate rather than inventing a second threshold.
            if sym.body_hash.is_none() {
                continue;
            }
            let Some(text) = symbol_text(&lines, sym) else {
                continue;
            };
            let tokens: Vec<u64> = tokenize(&text).iter().map(|t| token_hash(t)).collect();
            let mut windows = rolling_windows(&tokens);
            if windows.is_empty() {
                continue;
            }
            // Sorted so `shared_window_count` can intersect two symbols
            // with a linear merge instead of building a set per pair.
            windows.sort_unstable();
            eligible.push(Eligible { sym, windows });
        }
    }
    eligible
}

pub fn find_near_duplicates(index: &RepoIndex) -> Vec<NearDuplicateCandidate> {
    let eligible = collect_eligible(index);

    let mut buckets: HashMap<u64, Vec<usize>> = HashMap::new();
    for (i, e) in eligible.iter().enumerate() {
        for &w in &e.windows {
            buckets.entry(w).or_default().push(i);
        }
    }

    // Candidate pairs come only from *discriminative* buckets (issue
    // #398).
    //
    // The module's original claim -- "pairs with nothing in common
    // never get compared at all" -- was true but useless in practice.
    // At `WINDOW_TOKENS == 3` almost every pair of functions shares
    // *something*: `) { let`, `; if (`, `self . foo`. Measured on this
    // repo, the single largest window bucket held 1308 of 2089 eligible
    // symbols, and enumerating every bucket's pairs cost 16.9M
    // pair-visits -- so the scan degenerated to repeated all-pairs work
    // and took ~12s, which was essentially the entire cost of
    // `/api/health` and `/api/files`.
    //
    // A window shared by a large fraction of all symbols is boilerplate
    // and carries no signal, so it is skipped for *candidate discovery*.
    // It is NOT skipped for scoring: the overlap ratio below is computed
    // by exact intersection of the two symbols' full window sets, so any
    // pair that becomes a candidate gets exactly the ratio it always got.
    //
    // This does change what the marker reports, and the change is not
    // small: on this repo it drops 7,246 of 25,510 pairs (28%). Every
    // dropped pair is one whose *entire* shared window set is repo-wide
    // boilerplate, and that turns out to be a precision win rather than a
    // loss. Such pairs are short functions where `min(|windows|)` is tiny
    // enough that shared boilerplate alone clears `MIN_OVERLAP_RATIO` --
    // the highest-ratio casualty on this repo is 0.88 between
    //
    //     fn an_unrelated_repo_reports_no_dependencies() {
    //         let dir = tempfile::tempdir().unwrap();
    //         let root = dir.path().canonicalize().unwrap();
    //         ...
    //
    // and a different crate's four-line `returns_empty_when_...` test.
    // Those are not near-duplicates in any sense a reader would accept.
    //
    // No pair is *added*: the result is a strict subset of what the
    // previous implementation produced (verified by diffing both outputs
    // over this repo).
    //
    // The cap is also what makes this fast, not the restructuring around
    // it. Measured with the cap disabled, computing exact intersections
    // for all 16.9M candidate pairs takes 18.6s -- slower than the 12.4s
    // it replaced. With the cap it is 2.9s.
    let cap = bucket_candidate_cap(eligible.len());
    let mut candidates: HashSet<(usize, usize)> = HashSet::new();
    for idxs in buckets.values() {
        if idxs.len() < 2 || idxs.len() > cap {
            continue;
        }
        for a in 0..idxs.len() {
            for b in (a + 1)..idxs.len() {
                let (i, j) = (idxs[a].min(idxs[b]), idxs[a].max(idxs[b]));
                if i == j {
                    continue;
                }
                candidates.insert((i, j));
            }
        }
    }

    let mut out = Vec::new();
    for (i, j) in candidates {
        let a = &eligible[i];
        let b = &eligible[j];
        let count = shared_window_count(&a.windows, &b.windows);
        if a.sym.body_hash == b.sym.body_hash {
            continue; // already an exact duplicate; DuplicateCode covers it
        }
        let denom = a.windows.len().min(b.windows.len());
        if denom == 0 {
            continue;
        }
        let ratio = count as f64 / denom as f64;
        if ratio < MIN_OVERLAP_RATIO {
            continue;
        }
        out.push(NearDuplicateCandidate {
            file: a.sym.file.clone(),
            symbol: a.sym.name.clone(),
            line: a.sym.start_line,
            other_file: b.sym.file.clone(),
            other_symbol: b.sym.name.clone(),
            overlap_ratio: ratio,
        });
        out.push(NearDuplicateCandidate {
            file: b.sym.file.clone(),
            symbol: b.sym.name.clone(),
            line: b.sym.start_line,
            other_file: a.sym.file.clone(),
            other_symbol: a.sym.name.clone(),
            overlap_ratio: ratio,
        });
    }

    out.sort_by(|x, y| {
        x.file
            .cmp(&y.file)
            .then(x.line.cmp(&y.line))
            .then(x.other_symbol.cmp(&y.other_symbol))
    });
    out
}

/// Largest bucket still used to propose candidate pairs.
///
/// Proportional to the symbol count rather than a fixed number, so the
/// meaning ("this window is boilerplate *for this repo*") holds whether
/// the index has 200 symbols or 200,000. The floor keeps small repos --
/// where every bucket is small and the scan is cheap anyway -- behaving
/// exactly as before.
fn bucket_candidate_cap(symbol_count: usize) -> usize {
    const SHARE_DIVISOR: usize = 32; // ~3% of eligible symbols
    const FLOOR: usize = 64;
    (symbol_count / SHARE_DIVISOR).max(FLOOR)
}

/// `|a ∩ b|` for two sorted, deduplicated window lists.
///
/// This is exactly the count the previous bucket-accumulation produced
/// -- a pair's tally there was the number of distinct windows both
/// symbols appeared under -- so scoring is unchanged, only cheaper and
/// no longer dependent on which buckets were visited.
fn shared_window_count(a: &[u64], b: &[u64]) -> usize {
    let (mut i, mut j, mut n) = (0, 0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                n += 1;
                i += 1;
                j += 1;
            }
        }
    }
    n
}

/// A symbol's full line span (declaration through closing brace), 1:1
/// with `start_line`/`end_line`. Clamped defensively in case the file
/// on disk has shrunk since indexing.
fn symbol_text(lines: &[&str], sym: &Symbol) -> Option<String> {
    if sym.start_line == 0 || sym.start_line > lines.len() {
        return None;
    }
    let end = sym.end_line.min(lines.len());
    if end < sym.start_line {
        return None;
    }
    Some(lines[(sym.start_line - 1)..end].join("\n"))
}

/// Split into identifier/number runs and single-character punctuation
/// tokens, discarding whitespace entirely (its presence/width isn't
/// meaningful for near-duplicate comparison — reindentation alone
/// shouldn't change the result).
fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for c in text.chars() {
        if c.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else if c.is_alphanumeric() || c == '_' {
            current.push(c);
        } else {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            tokens.push(c.to_string());
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn token_hash(token: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    token.hash(&mut hasher);
    hasher.finish()
}

/// Every distinct `WINDOW_TOKENS`-token window's rolling hash, computed
/// incrementally over per-token hashes (classic Rabin-Karp: add the
/// entering token's contribution, subtract the leaving one's) rather
/// than re-hashing each window from scratch.
fn rolling_windows(tokens: &[u64]) -> Vec<u64> {
    if tokens.len() < WINDOW_TOKENS {
        return Vec::new();
    }

    let mut high_order = 1u64;
    for _ in 0..WINDOW_TOKENS - 1 {
        high_order = high_order.wrapping_mul(RK_BASE);
    }

    let mut hashes = std::collections::HashSet::new();
    let mut hash: u64 = 0;
    for &t in &tokens[..WINDOW_TOKENS] {
        hash = hash.wrapping_mul(RK_BASE).wrapping_add(t);
    }
    hashes.insert(hash);

    for i in WINDOW_TOKENS..tokens.len() {
        let leaving = tokens[i - WINDOW_TOKENS];
        let entering = tokens[i];
        hash = hash.wrapping_sub(leaving.wrapping_mul(high_order));
        hash = hash.wrapping_mul(RK_BASE).wrapping_add(entering);
        hashes.insert(hash);
    }

    hashes.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use repowise_core::{FileRecord, ImportRef, Language};
    use std::path::Path;

    fn write(dir: &Path, name: &str, contents: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn function_symbol(file: &Path, name: &str, start_line: usize, end_line: usize) -> Symbol {
        Symbol {
            id: Symbol::make_id(file, name, start_line),
            name: name.to_string(),
            kind: SymbolKind::Function,
            file: file.to_path_buf(),
            start_line,
            end_line,
            parent: None,
            complexity: 1,
            max_nesting_depth: 0,
            bumpy_road_bumps: 0,
            complex_conditionals: Vec::new(),
            io_in_loop: Vec::new(),
            string_concat_in_loop: Vec::new(),
            resource_construction_in_loop: Vec::new(),
            lock_in_loop: Vec::new(),
            list_insert_zero_in_loop: Vec::new(),
            json_parse_in_loop: Vec::new(),
            regex_compile_in_loop: Vec::new(),
            nested_loop_with_io: Vec::new(),
            nested_loop_quadratic: Vec::new(),
            serial_await_in_loop: Vec::new(),
            pd_concat_in_loop: Vec::new(),
            blocking_sync_in_async: Vec::new(),
            blocking_io_under_lock: Vec::new(),
            array_spread_in_reduce: Vec::new(),
            sql_cartesian_join: Vec::new(),
            defer_in_loop: Vec::new(),
            goroutine_in_unbounded_loop: Vec::new(),
            membership_test_in_loop: Vec::new(),
            sync_io_calls: Vec::new(),
            param_count: 1,
            primitive_param_count: 0,
            // Distinct per symbol (keyed off start_line) so two
            // different symbols never accidentally look like an
            // exact-duplicate pair here -- only the "is this long
            // enough to matter" `is_some()` gate matters to
            // find_near_duplicates, but an accidental collision between
            // two symbols' hashes would wrongly trip its "already an
            // exact duplicate" exclusion.
            body_hash: Some(start_line as u64),
        }
    }

    fn index_with(files: Vec<FileRecord>, root: &Path) -> RepoIndex {
        RepoIndex {
            root: root.to_path_buf(),
            files,
            other_files: 0,
            indexed_commit: None,
        }
    }

    const BODY_A: &str = "fn process_widget(count: i32) -> i32 {\n    let mut total = 0;\n    for i in 0..count {\n        total += i * 2;\n        if total > 1000 {\n            total -= 1000;\n        }\n    }\n    total + 1\n}\n";

    // Same shape as BODY_A with the accumulator variable renamed and one
    // constant tweaked -- a realistic "copy-pasted and lightly edited"
    // near-duplicate, not a byte-for-byte clone.
    const BODY_B: &str = "fn process_gadget(count: i32) -> i32 {\n    let mut sum = 0;\n    for i in 0..count {\n        sum += i * 2;\n        if sum > 1000 {\n            sum -= 1000;\n        }\n    }\n    sum + 2\n}\n";

    const BODY_UNRELATED: &str = "fn greet(name: &str) -> String {\n    let mut out = String::new();\n    out.push_str(\"Hello, \");\n    out.push_str(name);\n    out.push('!');\n    out\n}\n";

    /// Writes `first` then `second` (separated by a blank line) into one
    /// file, returning the path plus each half's exact 1-indexed
    /// `(start_line, end_line)` span computed from the actual text
    /// rather than hand-counted (error-prone once bodies change).
    fn write_two_functions(
        root: &Path,
        name: &str,
        first: &str,
        second: &str,
    ) -> (PathBuf, (usize, usize), (usize, usize)) {
        let path = write(root, name, &format!("{first}\n{second}"));
        let first_lines = first.lines().count();
        let first_span = (1, first_lines);
        let second_start = first_lines + 2; // +1 for the blank separator line, +1 to move past it
        let second_span = (second_start, second_start + second.lines().count() - 1);
        (path, first_span, second_span)
    }

    #[test]
    fn flags_a_near_duplicate_pair_with_renamed_variables() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let (path, a_span, b_span) = write_two_functions(&root, "widgets.rs", BODY_A, BODY_B);

        let a = function_symbol(&path, "process_widget", a_span.0, a_span.1);
        let b = function_symbol(&path, "process_gadget", b_span.0, b_span.1);

        let file = FileRecord {
            path: path.clone(),
            language: Language::Rust,
            lines: b_span.1,
            symbols: vec![a, b],
            imports: Vec::<ImportRef>::new(),
            calls: Vec::new(),
            field_accesses: Vec::new(),
        };

        let candidates = find_near_duplicates(&index_with(vec![file], &root));
        assert_eq!(candidates.len(), 2);
        assert!(candidates
            .iter()
            .any(|c| c.symbol == "process_widget" && c.other_symbol == "process_gadget"));
        assert!(candidates
            .iter()
            .all(|c| c.overlap_ratio >= MIN_OVERLAP_RATIO));
    }

    #[test]
    fn does_not_flag_genuinely_different_functions() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let (path, a_span, b_span) = write_two_functions(&root, "mixed.rs", BODY_A, BODY_UNRELATED);

        let a = function_symbol(&path, "process_widget", a_span.0, a_span.1);
        let b = function_symbol(&path, "greet", b_span.0, b_span.1);

        let file = FileRecord {
            path: path.clone(),
            language: Language::Rust,
            lines: b_span.1,
            symbols: vec![a, b],
            imports: Vec::new(),
            calls: Vec::new(),
            field_accesses: Vec::new(),
        };

        assert!(find_near_duplicates(&index_with(vec![file], &root)).is_empty());
    }

    #[test]
    fn excludes_pairs_already_caught_by_exact_duplicate_hash() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        // Byte-for-byte identical bodies (aside from the fn name/line).
        let (path, a_span, b_span) = write_two_functions(&root, "identical.rs", BODY_A, BODY_A);

        let mut a = function_symbol(&path, "process_widget", a_span.0, a_span.1);
        let mut b = function_symbol(&path, "process_widget_copy", b_span.0, b_span.1);
        a.body_hash = Some(42);
        b.body_hash = Some(42); // identical body_hash -> already a DuplicateCode pair

        let file = FileRecord {
            path: path.clone(),
            language: Language::Rust,
            lines: b_span.1,
            symbols: vec![a, b],
            imports: Vec::new(),
            calls: Vec::new(),
            field_accesses: Vec::new(),
        };

        assert!(find_near_duplicates(&index_with(vec![file], &root)).is_empty());
    }

    #[test]
    fn skips_symbols_too_short_to_have_a_body_hash() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = write(&root, "short.rs", "fn one() { 1 }\nfn two() { 1 }\n");

        let mut a = function_symbol(&path, "one", 1, 1);
        let mut b = function_symbol(&path, "two", 2, 2);
        a.body_hash = None;
        b.body_hash = None;

        let file = FileRecord {
            path: path.clone(),
            language: Language::Rust,
            lines: 2,
            symbols: vec![a, b],
            imports: Vec::new(),
            calls: Vec::new(),
            field_accesses: Vec::new(),
        };

        assert!(find_near_duplicates(&index_with(vec![file], &root)).is_empty());
    }

    /// The scoring path must stay exact (issue #398): a pair's overlap
    /// count is the number of distinct windows both symbols carry, and
    /// swapping bucket-accumulation for set intersection must not have
    /// changed it.
    #[test]
    fn shared_window_count_is_a_plain_set_intersection() {
        assert_eq!(shared_window_count(&[1, 2, 3], &[2, 3, 4]), 2);
        assert_eq!(shared_window_count(&[1, 2, 3], &[4, 5]), 0);
        assert_eq!(shared_window_count(&[], &[1]), 0);
        assert_eq!(shared_window_count(&[1, 2], &[1, 2]), 2);
    }

    /// The candidate cap scales with the index rather than being a
    /// fixed number, so "this window is boilerplate" keeps meaning the
    /// same thing at 200 symbols and at 200,000 -- with a floor so
    /// small repos, where every bucket is small anyway, are unaffected.
    #[test]
    fn the_candidate_cap_scales_with_the_index_but_never_below_the_floor() {
        assert_eq!(
            bucket_candidate_cap(0),
            64,
            "floor applies to an empty index"
        );
        assert_eq!(bucket_candidate_cap(100), 64, "still the floor");
        assert_eq!(
            bucket_candidate_cap(2089),
            65,
            "this repo, just over the floor"
        );
        assert_eq!(
            bucket_candidate_cap(200_000),
            6250,
            "scales on a large index"
        );
    }

    /// A genuine near-duplicate must survive the cap. Both halves here
    /// sit in a corpus of many unrelated functions that share plenty of
    /// boilerplate windows with each other, so the over-cap buckets are
    /// populated and the pair has to be found through its own
    /// distinctive windows.
    #[test]
    fn a_real_near_duplicate_survives_a_corpus_full_of_boilerplate() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        let mut files = Vec::new();

        // The pair worth finding.
        let (path, a_span, b_span) = write_two_functions(&root, "widgets.rs", BODY_A, BODY_B);
        files.push(FileRecord {
            path: path.clone(),
            language: Language::Rust,
            lines: b_span.1,
            symbols: vec![
                function_symbol(&path, "process_widget", a_span.0, a_span.1),
                function_symbol(&path, "process_gadget", b_span.0, b_span.1),
            ],
            imports: Vec::new(),
            calls: Vec::new(),
            field_accesses: Vec::new(),
        });

        // 200 short functions that differ only in a name, so their
        // shared boilerplate windows land in buckets far above the cap.
        for i in 0..200 {
            let body = format!(
                "fn boilerplate_{i}() {{\n    let dir = tempfile::tempdir().unwrap();\n    let root = dir.path().canonicalize().unwrap();\n    assert!(check(&root).is_empty());\n}}\n"
            );
            let name = format!("boiler{i}.rs");
            let p = write(&root, &name, &body);
            files.push(FileRecord {
                path: p.clone(),
                language: Language::Rust,
                lines: 5,
                symbols: vec![function_symbol(&p, &format!("boilerplate_{i}"), 1, 5)],
                imports: Vec::new(),
                calls: Vec::new(),
                field_accesses: Vec::new(),
            });
        }

        let found = find_near_duplicates(&index_with(files, &root));
        assert!(
            found
                .iter()
                .any(|c| c.symbol == "process_widget" && c.other_symbol == "process_gadget"),
            "the real pair must survive a boilerplate-heavy corpus, got {} candidates",
            found.len()
        );
    }

    #[test]
    fn a_file_missing_from_disk_since_indexing_is_skipped_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let missing = root.join("gone.rs");

        let sym = function_symbol(&missing, "process_widget", 1, 9);
        let file = FileRecord {
            path: missing,
            language: Language::Rust,
            lines: 9,
            symbols: vec![sym],
            imports: Vec::new(),
            calls: Vec::new(),
            field_accesses: Vec::new(),
        };

        assert!(find_near_duplicates(&index_with(vec![file], &root)).is_empty());
    }
}
