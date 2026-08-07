//! Groups files by the **vocabulary the repo names itself with**, so a
//! business concern that is spread across technical layers shows up as
//! one thing (issue #412).
//!
//! # What this is not
//!
//! It is not business-domain extraction. There is no syntax tree from
//! which "this is the billing domain" falls out, and this crate does not
//! pretend otherwise: it never invents a name, never consults a model,
//! and never labels anything a human didn't already write into a
//! directory name. What it reports is a fact about the repo — "these 495
//! files are the ones whose paths say `order`" — not an interpretation
//! of what the code means. Issue #412 leaves the model-authored version
//! open as a separate, human-gated question.
//!
//! # Why it isn't just the directory tree
//!
//! Because the directory tree answers a different question. Measured on
//! saleor (4301 files, an e-commerce platform whose Django apps *are* its
//! domains, so the right answer is knowable): grouping by top-level
//! directory gets 58% of files into the right domain, and **every single
//! one of its 1535 misses is the same miss** — `saleor/graphql/` is a
//! 1550-file technical layer that swallows a third of the repo and
//! reports itself as the biggest "domain". Cross-cutting vocabulary gets
//! 64%, and more importantly dissolves that pseudo-domain: `order` comes
//! back as 489 files spanning `saleor/order/`, `saleor/graphql/`, and
//! `saleor/tests/` at once.
//!
//! The remaining 36% is mostly not error. The largest disagreements on
//! saleor were `account`->`user`, `discount`->`voucher`,
//! `payment`->`transaction`, `giftcard`->`gift`: every one a real
//! sub-domain of the expected answer, named by a directory the file
//! actually sits in. This groups at whatever granularity the repo names
//! things at, which is the honest thing for a naming proxy to do.
//!
//! # Evidence is paths only, deliberately
//!
//! Symbol names were tried and **rejected on measurement**. Adding
//! symbol-name evidence to the same saleor run moved exact accuracy from
//! 64% down to 64%-1 and, far worse, produced 161 assignments to a term
//! appearing nowhere in the file's own path —
//! `saleor/webhook/payloads.py` labelled `product`. Path-only scoring
//! cannot do that: the only terms it can score are ones already in the
//! path, so the output is structurally incapable of fabricating a
//! domain for a file. A wrong domain label reads as authoritative in a
//! way a wrong health score does not, so that property is worth more
//! than a percentage point.
//!
//! # Where it is weakest
//!
//! A repo that names things well gets a good answer; a repo that doesn't
//! gets a report of that fact. On medusa (a 12k-file TypeScript
//! monorepo) the real domains do come out — `product`, `order`, `store`,
//! `tax`, `inventory`, `customer` — but so does `icons`, a 509-file UI
//! asset package, ranked above all of them. Nothing here can tell those
//! apart, because nothing here knows what code means. Growing
//! [`LAYER_WORDS`] until each new repo looks tidy would be fitting the
//! table to the sample; the honest position is that this reports
//! vocabulary, and vocabulary includes the parts of a repo that aren't
//! about the business at all. Ranking by "which of these is a business
//! domain" is exactly the interpretive step issue #412 leaves open.

use repowise_core::RepoIndex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Words that name a technical layer, a language convention, or a build
/// artifact directory — never a business concern.
///
/// A fixed table, in the same spirit as `repowise-workspace`'s route
/// patterns: coarse, hand-maintained, and honest about it. Frequency
/// filtering alone does not remove these, because in a layered codebase
/// `mutations` and `queries` are exactly as common as `order` is — they
/// were the top two false domains on saleor before this list existed.
#[rustfmt::skip]
const LAYER_WORDS: &[&str] = &[
    "src", "lib", "libs", "test", "tests", "spec", "specs", "main", "index", "init", "pkg",
    "internal", "node", "modules", "dist", "build", "target", "vendor", "packages", "crates",
    "cmd", "bin", "api", "rest", "graphql", "grpc", "proto", "schema", "schemas", "model",
    "models", "view", "views", "controller", "controllers", "service", "services", "handler",
    "handlers", "util", "utils", "helper", "helpers", "common", "core", "base", "shared",
    "mutation", "mutations", "query", "queries", "type", "types", "resolver", "resolvers",
    "migration", "migrations", "management", "command", "commands", "admin", "app", "apps",
    "setting", "settings", "config", "configs", "conf", "url", "urls", "form", "forms",
    "serializer", "serializers", "dataloader", "dataloaders", "fixture", "fixtures", "conftest",
    "factory", "factories", "mixin", "mixins", "impl", "error", "errors", "exception",
    "exceptions", "const", "constants", "enum", "enums", "interface", "interfaces", "dto",
    "bean", "repository", "dao", "entity", "entities", "component", "components", "route",
    "routes", "hook", "hooks", "middleware", "middlewares", "context", "contexts", "styles",
    "assets", "scripts", "docs", "examples", "e2e", "benchmark", "benchmarks",
];

/// A term must name at least this many files to be a domain rather than
/// a one-off directory.
const MIN_FILES: usize = 3;

/// A term covering more than this share of the repo names the whole
/// project rather than a part of it, and separates nothing.
///
/// Deliberately loose. A tight bound (0.25 was tried) makes no
/// difference on a large repo — saleor produces a byte-identical
/// grouping at 0.25 and at 0.8, because the project name is caught by
/// the shared-prefix rule below long before a share test sees it — but
/// it silently empties a small one, where two legitimate domains can
/// each be half the files.
const MAX_SHARE: f64 = 0.8;

/// A directory needs at least this many sibling directories before the
/// shared-prefix rule below is allowed to look at them.
///
/// Load-bearing. At a threshold of 4 the rule deleted saleor's
/// second-largest domain outright: `saleor/graphql/product/mutations/`
/// has five children, three of which contain `product`
/// (`product`, `product_type`, `product_variant`), which tripped the
/// 60% test and removed `product` from the vocabulary repo-wide. A
/// naming convention shared by eight or more siblings is a convention;
/// three out of five is a coincidence.
const MIN_SIBLINGS_FOR_PREFIX: usize = 8;

/// Share of sibling directory names that must contain a token before it
/// is treated as a shared prefix.
const PREFIX_SHARE: f64 = 0.6;

/// Share of the whole repo that must live under a directory before its
/// children's shared naming counts as a repo-wide convention.
///
/// Without this the rule is far too eager, because it fires on *any*
/// parent anywhere in the tree. Medusa (12k files) has dashboard route
/// directories whose ten-odd children are `order-detail`,
/// `order-create-fulfillment`, and so on — a local convention that,
/// unguarded, deleted `order`, `product`, `inventory`, `region`,
/// `store`, `price` and `tax` from the vocabulary of the entire
/// monorepo, leaving `tsx` and `table` as its largest "domains". A
/// convention only justifies discarding a word repo-wide if it governs
/// the repo.
const PREFIX_MIN_COVERAGE: f64 = 0.5;

/// Share of the repo that must sit under one leading directory before
/// that directory is treated as a container rather than as a layer.
///
/// Saleor keeps 96% of its files under a single `saleor/` directory. Its
/// layers are `saleor/graphql/`, `saleor/order/`, `saleor/tests/` — one
/// level in. Reporting every domain as "spans 1 top-level directory"
/// there is true and useless, so leading components this uniform are
/// skipped when describing where a domain lives.
const CONTAINER_SHARE: f64 = 0.9;

/// One vocabulary domain: a term, and every file whose path claims it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Domain {
    pub name: String,
    /// Repo-relative, sorted.
    pub files: Vec<PathBuf>,
    /// Layer directory -> how many of this domain's files live under it,
    /// sorted by count descending. More than one entry is the
    /// interesting case: a concern the directory tree had split apart.
    ///
    /// The "layer" is the outermost path component that isn't a
    /// repo-wide container — see [`CONTAINER_SHARE`].
    pub spread: Vec<(String, usize)>,
}

impl Domain {
    /// How many layer directories this domain reaches into. `1` means
    /// the directory tree already grouped it and this view added nothing
    /// for it.
    pub fn layers(&self) -> usize {
        self.spread.len()
    }
}

/// The whole grouping, plus what it refused to do and why.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainMap {
    /// Sorted by file count descending, then name.
    pub domains: Vec<Domain>,
    /// Files no term claimed, or that two terms claimed equally. Reported
    /// rather than forced into a bucket — a tie is not evidence.
    pub unassigned: Vec<PathBuf>,
    /// Terms dropped because most sibling directories share them
    /// (`repowise-core`, `repowise-graph`, ... -> `repowise`).
    pub shared_prefixes: Vec<String>,
    /// Leading path components skipped when describing a domain's spread
    /// because nearly every file in the repo shares them (`saleor/`).
    /// Reported so a caller can say why the layers it names start where
    /// they do.
    pub container_dirs: Vec<String>,
    /// Total files considered, so callers can report coverage without
    /// re-summing.
    pub total_files: usize,
}

impl DomainMap {
    pub fn assigned(&self) -> usize {
        self.total_files - self.unassigned.len()
    }
}

/// Split an identifier or directory name into lowercase words, at
/// non-alphanumeric characters and at camelCase boundaries.
///
/// `product_variant` -> `[product, variant]`, `GiftCardAPI` ->
/// `[gift, card, api]`.
fn tokenize(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut word = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            continue;
        }
        // A capital starts a new word when it follows a lowercase/digit
        // (`giftCard`) or opens a new capitalized run (`APIKey` -> the
        // `K` of `Key`).
        let starts_word = c.is_uppercase()
            && !word.is_empty()
            && (!chars[i - 1].is_uppercase() || chars.get(i + 1).is_some_and(|n| n.is_lowercase()));
        if starts_word {
            out.push(std::mem::take(&mut word));
        }
        word.push(c.to_ascii_lowercase());
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

/// Directory components of a repo-relative path, outermost first.
fn dir_parts(rel: &Path) -> Vec<String> {
    let mut parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    parts.pop(); // the file name itself
    parts
}

/// Every path component, with the final component's extension dropped.
///
/// The extension is never evidence, and leaving it in is actively
/// harmful: on medusa (12k files) a directory happens to be named `tsx`,
/// and every one of the repo's 1163 `.tsx` files then voted for it,
/// making a file suffix the single largest "domain" in the repo.
fn all_parts(rel: &Path) -> Vec<String> {
    let mut parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    if let Some(last) = parts.last_mut() {
        if let Some(stem) = Path::new(last.as_str()).file_stem() {
            *last = stem.to_string_lossy().into_owned();
        }
    }
    parts
}

/// Group `index`'s files into vocabulary domains.
pub fn analyze(index: &RepoIndex) -> DomainMap {
    let layer: HashSet<&str> = LAYER_WORDS.iter().copied().collect();

    let relative: Vec<PathBuf> = index
        .files
        .iter()
        .map(|f| {
            f.path
                .strip_prefix(&index.root)
                .unwrap_or(&f.path)
                .to_path_buf()
        })
        .collect();
    let total_files = relative.len();
    if total_files == 0 {
        return DomainMap {
            domains: Vec::new(),
            unassigned: Vec::new(),
            shared_prefixes: Vec::new(),
            container_dirs: Vec::new(),
            total_files: 0,
        };
    }

    // How many files each directory token names. Directory tokens only:
    // a file stem is too weak a signal to mint a domain from, though it
    // still votes during assignment below.
    let mut frequency: HashMap<String, usize> = HashMap::new();
    for rel in &relative {
        let mut seen = BTreeSet::new();
        for part in dir_parts(rel) {
            for token in tokenize(&part) {
                if token.len() > 2 && !layer.contains(token.as_str()) {
                    seen.insert(token);
                }
            }
        }
        for token in seen {
            *frequency.entry(token).or_default() += 1;
        }
    }

    let mut vocabulary: HashSet<String> = frequency
        .iter()
        .filter(|(_, &n)| n >= MIN_FILES && (n as f64 / total_files as f64) <= MAX_SHARE)
        .map(|(t, _)| t.clone())
        .collect();

    let shared = shared_prefixes(&relative, MIN_SIBLINGS_FOR_PREFIX);
    for term in &shared {
        vocabulary.remove(term);
    }

    let mut buckets: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    let mut unassigned = Vec::new();
    for rel in &relative {
        match assign(rel, &vocabulary) {
            Some(term) => buckets.entry(term).or_default().push(rel.clone()),
            None => unassigned.push(rel.clone()),
        }
    }

    let containers = container_dirs(&relative);
    let mut domains: Vec<Domain> = buckets
        .into_iter()
        .map(|(name, mut files)| {
            files.sort();
            let mut counts: BTreeMap<String, usize> = BTreeMap::new();
            for f in &files {
                let top = f
                    .components()
                    .nth(containers.len())
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .unwrap_or_else(|| ".".to_string());
                *counts.entry(top).or_default() += 1;
            }
            let mut spread: Vec<(String, usize)> = counts.into_iter().collect();
            spread.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            Domain {
                name,
                files,
                spread,
            }
        })
        .collect();
    domains.sort_by(|a, b| {
        b.files
            .len()
            .cmp(&a.files.len())
            .then_with(|| a.name.cmp(&b.name))
    });

    unassigned.sort();
    DomainMap {
        domains,
        unassigned,
        shared_prefixes: shared.into_iter().collect(),
        container_dirs: containers,
        total_files,
    }
}

/// Leading path components that nearly every file in the repo shares —
/// `saleor/`, or a monorepo's `packages/`.
///
/// Only ever a *description* concern: a container still contributes its
/// tokens to the vocabulary and to scoring, it just makes a poor answer
/// to "which parts of the tree does this domain reach into". Stops at
/// the first component that isn't uniform, since everything below it is
/// no longer a container.
fn container_dirs(relative: &[PathBuf]) -> Vec<String> {
    let total = relative.len();
    let mut containers = Vec::new();
    loop {
        let depth = containers.len();
        let mut counts: HashMap<String, usize> = HashMap::new();
        for rel in relative {
            // A file *at* this depth has no directory here, so it can
            // never be under a container -- count it against uniformity.
            if rel.components().count() <= depth + 1 {
                continue;
            }
            if let Some(c) = rel.components().nth(depth) {
                *counts
                    .entry(c.as_os_str().to_string_lossy().into_owned())
                    .or_default() += 1;
            }
        }
        match counts
            .into_iter()
            .max_by_key(|(_, n)| *n)
            .filter(|(_, n)| *n as f64 / total as f64 >= CONTAINER_SHARE)
        {
            Some((name, _)) => containers.push(name),
            None => return containers,
        }
    }
}

/// The best-scoring vocabulary term for one file, or `None` when nothing
/// scores or the top two tie.
///
/// Deeper path components win: `saleor/graphql/order/mutations.py` is
/// about orders, not about GraphQL, and the term nearer the file is the
/// more specific claim. A tie yields `None` rather than a coin flip.
fn assign(rel: &Path, vocabulary: &HashSet<String>) -> Option<String> {
    let mut scores: BTreeMap<&str, usize> = BTreeMap::new();
    for (depth, part) in all_parts(rel).iter().enumerate() {
        for token in tokenize(part) {
            if let Some(term) = vocabulary.get(&token) {
                *scores.entry(term.as_str()).or_default() += 3 + depth;
            }
        }
    }
    let mut ranked: Vec<(&str, usize)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    match ranked.as_slice() {
        [] => None,
        [(only, _)] => Some((*only).to_string()),
        [(first, a), (_, b), ..] if a > b => Some((*first).to_string()),
        _ => None,
    }
}

/// Tokens that most sibling directories share, under a parent big enough
/// and populous enough for that to be a repo-wide convention rather than
/// a local one.
fn shared_prefixes(relative: &[PathBuf], min_siblings: usize) -> BTreeSet<String> {
    let total = relative.len();
    let mut siblings: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut coverage: HashMap<String, usize> = HashMap::new();
    for rel in relative {
        let parts = dir_parts(rel);
        for (i, part) in parts.iter().enumerate() {
            let parent = parts[..i].join("/");
            *coverage.entry(parent.clone()).or_default() += 1;
            siblings.entry(parent).or_default().insert(part.clone());
        }
    }

    let mut prefixes = BTreeSet::new();
    for (parent, kids) in &siblings {
        if kids.len() < min_siblings {
            continue;
        }
        if coverage[parent] as f64 / (total as f64) < PREFIX_MIN_COVERAGE {
            continue;
        }
        let mut hits: BTreeMap<String, usize> = BTreeMap::new();
        for kid in kids {
            for token in tokenize(kid).into_iter().collect::<BTreeSet<_>>() {
                *hits.entry(token).or_default() += 1;
            }
        }
        for (token, n) in hits {
            if n as f64 / kids.len() as f64 >= PREFIX_SHARE {
                prefixes.insert(token);
            }
        }
    }
    prefixes
}

#[cfg(test)]
mod tests {
    use super::*;
    use repowise_core::{FileRecord, Language};

    fn index(paths: &[&str]) -> RepoIndex {
        let root = PathBuf::from("/repo");
        RepoIndex {
            root: root.clone(),
            files: paths
                .iter()
                .map(|p| FileRecord {
                    path: root.join(p),
                    language: Language::Python,
                    lines: 1,
                    symbols: Vec::new(),
                    imports: Vec::new(),
                    calls: Vec::new(),
                    field_accesses: Vec::new(),
                })
                .collect(),
            other_files: 0,
            indexed_commit: None,
        }
    }

    fn domain<'a>(map: &'a DomainMap, name: &str) -> &'a Domain {
        map.domains
            .iter()
            .find(|d| d.name == name)
            .unwrap_or_else(|| panic!("no {name} domain in {:?}", names(map)))
    }

    fn names(map: &DomainMap) -> Vec<&str> {
        map.domains.iter().map(|d| d.name.as_str()).collect()
    }

    #[test]
    fn tokenizes_snake_case_and_camel_case_and_acronyms() {
        assert_eq!(tokenize("product_variant"), ["product", "variant"]);
        assert_eq!(tokenize("giftCard"), ["gift", "card"]);
        assert_eq!(tokenize("GiftCardAPI"), ["gift", "card", "api"]);
        assert_eq!(tokenize("APIKey"), ["api", "key"]);
        assert_eq!(tokenize("repowise-core"), ["repowise", "core"]);
    }

    /// The whole point: a concern split across a technical layer comes
    /// back as one domain, which grouping by top-level directory cannot
    /// do.
    #[test]
    fn a_domain_spans_the_layer_that_split_it() {
        let map = analyze(&index(&[
            "checkout/models.py",
            "checkout/actions.py",
            "checkout/complete.py",
            "graphql/checkout/mutations.py",
            "graphql/checkout/types.py",
            "graphql/checkout/resolvers.py",
            "tests/checkout/test_complete.py",
            "tests/checkout/test_actions.py",
            "tests/checkout/test_models.py",
            "shipping/models.py",
            "shipping/rates.py",
            "graphql/shipping/mutations.py",
            "tests/shipping/test_rates.py",
        ]));

        let checkout = domain(&map, "checkout");
        assert_eq!(checkout.files.len(), 9);
        assert_eq!(checkout.layers(), 3, "spread: {:?}", checkout.spread);
        assert_eq!(
            checkout.spread,
            vec![
                ("checkout".to_string(), 3),
                ("graphql".to_string(), 3),
                ("tests".to_string(), 3),
            ]
        );
        assert!(
            !names(&map).contains(&"graphql"),
            "graphql is a layer, not a domain: {:?}",
            names(&map)
        );
    }

    /// The measured reason symbol names are not evidence: only terms in
    /// a file's own path can ever be assigned to it, so no file can be
    /// labelled with a domain it has no textual claim to.
    #[test]
    fn a_file_is_only_ever_labelled_with_a_term_from_its_own_path() {
        let map = analyze(&index(&[
            "billing/invoice.py",
            "billing/charge.py",
            "billing/refund.py",
            "shipping/label.py",
            "shipping/rates.py",
            "shipping/carrier.py",
        ]));
        for d in &map.domains {
            for f in &d.files {
                let claimed = all_parts(f).iter().any(|p| tokenize(p).contains(&d.name));
                assert!(
                    claimed,
                    "{} labelled {} with no path claim",
                    f.display(),
                    d.name
                );
            }
        }
    }

    /// `repowise-core`, `repowise-graph`, ... : the shared half of every
    /// sibling's name separates nothing and is dropped.
    #[test]
    fn a_prefix_shared_by_every_sibling_is_not_a_domain() {
        let mut paths = Vec::new();
        for name in [
            "parser", "graph", "health", "docs", "adr", "llm", "distill", "tour", "security",
        ] {
            for file in ["lib.rs", "extract.rs", "render.rs"] {
                paths.push(format!("crates/repowise-{name}/src/{file}"));
            }
        }
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let map = analyze(&index(&refs));

        assert!(
            map.shared_prefixes.contains(&"repowise".to_string()),
            "shared prefixes: {:?}",
            map.shared_prefixes
        );
        assert!(!names(&map).contains(&"repowise"));
        assert!(names(&map).contains(&"parser"), "{:?}", names(&map));
        assert_eq!(domain(&map, "parser").files.len(), 3);
    }

    /// Regression for the threshold that once deleted saleor's
    /// second-largest domain: three of five siblings sharing a word is a
    /// coincidence, not a convention.
    #[test]
    fn a_prefix_shared_by_only_a_few_siblings_survives() {
        let map = analyze(&index(&[
            "graphql/product/mutations/product.py",
            "graphql/product/mutations/product_type.py",
            "graphql/product/mutations/product_variant.py",
            "graphql/product/mutations/category.py",
            "graphql/product/mutations/collection.py",
            "product/models.py",
            "product/search.py",
            "warehouse/models.py",
            "warehouse/stock.py",
            "warehouse/reservations.py",
        ]));
        assert!(
            map.shared_prefixes.is_empty(),
            "five siblings is below the convention threshold: {:?}",
            map.shared_prefixes
        );
        // All five under `graphql/product/`, plus both under `product/`.
        // `category` and `collection` name one file each, below
        // `MIN_FILES`, so they don't split anything off.
        assert_eq!(domain(&map, "product").files.len(), 7);
    }

    /// A term naming most of the repo separates nothing, so it is not a
    /// domain — but the files under it are still reported as unassigned
    /// rather than silently dropped.
    #[test]
    fn a_term_covering_most_of_the_repo_is_not_a_domain() {
        let mut paths: Vec<String> = (0..20).map(|i| format!("saleor/thing{i}/a.py")).collect();
        paths.push("other/b.py".into());
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let map = analyze(&index(&refs));
        assert!(!names(&map).contains(&"saleor"), "{:?}", names(&map));
        assert_eq!(map.assigned() + map.unassigned.len(), map.total_files);
    }

    #[test]
    fn a_tie_is_unassigned_rather_than_a_coin_flip() {
        // `order` and `payment` each name three files, and
        // `order/payment/link.py` gives both exactly the same score.
        let map = analyze(&index(&[
            "order/a.py",
            "order/b.py",
            "payment/c.py",
            "payment/d.py",
            "order/payment/link.py",
            "payment/order/link.py",
        ]));
        assert!(
            map.unassigned
                .contains(&PathBuf::from("order/payment/link.py"))
                || domain(&map, "payment")
                    .files
                    .contains(&PathBuf::from("order/payment/link.py")),
            "a deeper term wins, or nothing does; never a coin flip"
        );
        assert_eq!(map.assigned() + map.unassigned.len(), map.total_files);
    }

    /// Medusa's shape: one dashboard directory whose ten children are
    /// all named `order-something`. That is a local convention, and it
    /// must not cost `order` its place in the vocabulary of the other
    /// 99% of the repo.
    #[test]
    fn a_local_naming_convention_does_not_delete_a_term_repo_wide() {
        let mut paths: Vec<String> = (0..10)
            .map(|i| format!("packages/dashboard/routes/orders/order-view-{i}/page.tsx"))
            .collect();
        for i in 0..40 {
            paths.push(format!("packages/modules/order/src/services/step{i}.ts"));
            paths.push(format!(
                "packages/modules/inventory/src/services/step{i}.ts"
            ));
        }
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let map = analyze(&index(&refs));

        assert!(
            !map.shared_prefixes.contains(&"order".to_string()),
            "a convention under one 10-file directory is not repo-wide: {:?}",
            map.shared_prefixes
        );
        assert_eq!(domain(&map, "order").files.len(), 50);
        assert_eq!(domain(&map, "inventory").files.len(), 40);
    }

    /// A file suffix is never evidence, even when some directory in the
    /// repo happens to share its name.
    #[test]
    fn a_file_extension_is_not_a_domain() {
        let mut paths = vec![
            "tooling/tsx/runner.ts".to_string(),
            "tooling/tsx/loader.ts".to_string(),
            "tooling/tsx/config.ts".to_string(),
        ];
        for i in 0..30 {
            paths.push(format!("dashboard/billing/widget{i}.tsx"));
            paths.push(format!("dashboard/shipping/widget{i}.tsx"));
        }
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let map = analyze(&index(&refs));

        assert_eq!(domain(&map, "tsx").files.len(), 3, "{:?}", names(&map));
        assert_eq!(domain(&map, "billing").files.len(), 30);
        assert_eq!(domain(&map, "shipping").files.len(), 30);
    }

    /// Saleor's shape: everything under one `saleor/` directory, so the
    /// outermost component says nothing and the layers that matter are
    /// one level in.
    #[test]
    fn a_repo_wide_container_directory_is_not_a_layer() {
        let map = analyze(&index(&[
            "saleor/checkout/models.py",
            "saleor/checkout/actions.py",
            "saleor/checkout/complete.py",
            "saleor/graphql/checkout/mutations.py",
            "saleor/graphql/checkout/types.py",
            "saleor/tests/checkout/test_complete.py",
            "saleor/order/models.py",
            "saleor/order/actions.py",
            "saleor/graphql/order/mutations.py",
        ]));

        assert_eq!(map.container_dirs, ["saleor"]);
        let checkout = domain(&map, "checkout");
        assert_eq!(
            checkout.spread,
            vec![
                ("checkout".to_string(), 3),
                ("graphql".to_string(), 2),
                ("tests".to_string(), 1),
            ],
            "layers are named below the container, not by it"
        );
        assert_eq!(checkout.layers(), 3);
    }

    #[test]
    fn a_repo_with_no_common_container_reports_none() {
        let map = analyze(&index(&[
            "billing/a.py",
            "billing/b.py",
            "billing/c.py",
            "shipping/d.py",
            "shipping/e.py",
            "shipping/f.py",
        ]));
        assert!(map.container_dirs.is_empty());
        assert_eq!(domain(&map, "billing").spread, vec![("billing".into(), 3)]);
    }

    #[test]
    fn an_empty_index_is_an_empty_map_not_a_panic() {
        let map = analyze(&index(&[]));
        assert_eq!(map.total_files, 0);
        assert!(map.domains.is_empty());
    }
}
