//! Supply-chain checks on the GitHub Actions workflows (issue #8): every Action is pinned to a
//! full commit SHA, the Rust toolchain is a fixed version, and cargo never rewrites Cargo.lock.

use std::fs;
use std::path::{Path, PathBuf};

fn workflows() -> Vec<(PathBuf, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows");
    let mut files: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("yml" | "yaml")))
        .map(|p| {
            let text = fs::read_to_string(&p).unwrap();
            (p, text)
        })
        .collect();
    files.sort();
    assert!(files.len() >= 2, "expected ci.yml and release.yml");
    files
}

/// `uses:` value of a workflow line, without the trailing comment.
fn uses_value(line: &str) -> Option<&str> {
    let t = line.trim_start().trim_start_matches("- ").trim_start();
    let v = t.strip_prefix("uses:")?;
    Some(v.split(" #").next().unwrap().trim())
}

/// `owner/repo[/path]@<40 hex>` followed by a `# vX` comment, or a local `./` action.
fn is_pinned(line: &str) -> bool {
    let Some(v) = uses_value(line) else {
        return true;
    };
    if v.starts_with("./") {
        return true;
    }
    let Some((_, rev)) = v.split_once('@') else {
        return false;
    };
    let sha = rev.len() == 40 && rev.bytes().all(|b| b.is_ascii_hexdigit());
    let comment = line
        .split_once(" # ")
        .is_some_and(|(_, c)| c.trim().starts_with('v'));
    sha && comment
}

/// Cargo commands in `run:` steps (single-line or block), as trimmed command lines.
fn cargo_commands(text: &str) -> Vec<&str> {
    text.lines()
        .map(|l| l.trim().trim_start_matches("run:").trim())
        .filter(|l| l.starts_with("cargo "))
        .collect()
}

/// `cargo fmt` never resolves dependencies and rejects `--locked`; everything else must pass it.
fn is_locked(cmd: &str) -> bool {
    cmd.starts_with("cargo fmt") || cmd.split_whitespace().any(|w| w == "--locked")
}

/// `toolchain:` inputs of `dtolnay/rust-toolchain` steps.
fn toolchains(text: &str) -> Vec<&str> {
    text.lines()
        .filter_map(|l| l.trim().strip_prefix("toolchain:"))
        .map(|v| v.split(" #").next().unwrap().trim())
        .collect()
}

fn is_fixed_version(v: &str) -> bool {
    let parts: Vec<_> = v.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

#[test]
fn checker_rejects_mutable_refs() {
    assert!(!is_pinned("      - uses: actions/checkout@v4"));
    assert!(!is_pinned("      - uses: dtolnay/rust-toolchain@stable"));
    assert!(!is_pinned("        uses: softprops/action-gh-release@v2"));
    assert!(!is_pinned("      - uses: actions/checkout@main"));
    assert!(!is_pinned("      - uses: actions/checkout"));
    // Short SHA, and SHA without the version comment.
    assert!(!is_pinned(
        "      - uses: actions/checkout@11d5960 # v4.4.0"
    ));
    assert!(!is_pinned(
        "      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262"
    ));
}

#[test]
fn checker_accepts_sha_pins() {
    assert!(is_pinned(
        "      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262 # v4.4.0"
    ));
    assert!(is_pinned("      - uses: ./local-action"));
    assert!(is_pinned("      - name: Test"));
}

#[test]
fn checker_finds_unlocked_cargo() {
    let text = "      - name: Test\n        run: cargo test\n        run: |\n          cargo build --release\n          cargo fmt --check\n";
    assert_eq!(
        cargo_commands(text),
        ["cargo test", "cargo build --release", "cargo fmt --check"]
    );
    assert!(!is_locked("cargo test"));
    assert!(!is_locked("cargo build --release --lockedx"));
    assert!(is_locked("cargo test --locked"));
    assert!(is_locked("cargo fmt --check"));
}

#[test]
fn checker_rejects_floating_toolchains() {
    assert!(!is_fixed_version("stable"));
    assert!(!is_fixed_version("1.99"));
    assert!(!is_fixed_version("nightly-2026-10-01"));
    assert!(is_fixed_version("1.99.1"));
}

#[test]
fn every_action_is_pinned_to_a_commit_sha() {
    let mut bad = Vec::new();
    for (path, text) in workflows() {
        for (i, line) in text.lines().enumerate() {
            if !is_pinned(line) {
                bad.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
            }
        }
    }
    assert!(bad.is_empty(), "unpinned actions:\n{}", bad.join("\n"));
}

#[test]
fn every_cargo_command_is_locked() {
    let mut bad = Vec::new();
    let mut seen = 0;
    for (path, text) in workflows() {
        for cmd in cargo_commands(&text) {
            seen += 1;
            if !is_locked(cmd) {
                bad.push(format!("{}: {cmd}", path.display()));
            }
        }
    }
    assert!(seen >= 6, "found only {seen} cargo commands");
    assert!(
        bad.is_empty(),
        "cargo without --locked:\n{}",
        bad.join("\n")
    );
}

#[test]
fn rust_toolchain_is_a_fixed_version() {
    for (path, text) in workflows() {
        let steps = text.matches("dtolnay/rust-toolchain@").count();
        let versions = toolchains(&text);
        assert_eq!(
            versions.len(),
            steps,
            "{}: each rust-toolchain step needs `toolchain:`",
            path.display()
        );
        for v in versions {
            assert!(is_fixed_version(v), "{}: toolchain {v}", path.display());
        }
    }
}

#[test]
fn workflows_use_the_same_toolchain() {
    let mut all: Vec<String> = workflows()
        .iter()
        .flat_map(|(_, t)| toolchains(t).into_iter().map(String::from))
        .collect();
    all.dedup();
    assert_eq!(all.len(), 1, "toolchains differ: {all:?}");
}

#[test]
fn dependabot_updates_actions() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/dependabot.yml");
    let text = fs::read_to_string(&path).expect(".github/dependabot.yml");
    assert!(text.contains("package-ecosystem: \"github-actions\""));
}

/// Tags with a suffix (`v0.2.1-rc.1`) publish as pre-releases, so `releases/latest` (linked from
/// the README) never points at a test build.
#[test]
fn suffixed_tags_publish_as_prerelease() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/release.yml");
    let text = fs::read_to_string(path).unwrap();
    assert!(
        text.lines()
            .any(|l| l.trim() == "prerelease: ${{ contains(github.ref_name, '-') }}"),
        "release.yml must mark suffixed tags as pre-releases"
    );
}

/// Byte budget behind the README's "under 1 MB" promise (issue #18).
const SIZE_LIMIT: &str = "$limit = 1000000";

fn line_index(text: &str, pred: impl Fn(&str) -> bool) -> Option<usize> {
    text.lines().position(|l| pred(l.trim()))
}

#[test]
fn binary_size_is_checked_after_the_release_build() {
    let mut checked = 0;
    // Only workflows that build the app; pages.yml builds the website and runs no cargo.
    for (path, text) in workflows()
        .into_iter()
        .filter(|(_, text)| !cargo_commands(text).is_empty())
    {
        checked += 1;
        let build = line_index(&text, |l| {
            l.trim_start_matches("run:").trim() == "cargo build --release --locked"
        })
        .unwrap_or_else(|| panic!("{}: no release build", path.display()));
        let check = line_index(&text, |l| l == "- name: Check binary size")
            .unwrap_or_else(|| panic!("{}: no size check", path.display()));
        assert!(
            check > build,
            "{}: size check must follow the build",
            path.display()
        );
        assert!(
            text.lines().any(|l| l.trim() == SIZE_LIMIT),
            "{}: expected `{SIZE_LIMIT}`",
            path.display()
        );
    }
    assert!(
        checked >= 2,
        "ci.yml and release.yml must both check the size"
    );
}

#[test]
fn release_checks_size_before_packaging() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/release.yml");
    let text = fs::read_to_string(path).unwrap();
    let check = line_index(&text, |l| l == "- name: Check binary size").expect("size check");
    let package = line_index(&text, |l| l == "- name: Package").expect("package step");
    assert!(check < package);
}

fn release_yml() -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/release.yml"))
        .unwrap()
}

/// Signed build provenance for the published files (issue #19).
#[test]
fn release_attests_exe_and_zip_between_package_and_publish() {
    let text = release_yml();
    let attest = line_index(&text, |l| l.starts_with("uses: actions/attest@"))
        .or_else(|| line_index(&text, |l| l.starts_with("- uses: actions/attest@")))
        .expect("actions/attest step");
    let package = line_index(&text, |l| l == "- name: Package").unwrap();
    let publish = line_index(&text, |l| l == "- name: Publish").unwrap();
    assert!(package < attest && attest < publish);
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    assert!(lines.contains(&"dist/claude-codex-taskbar.exe"));
    assert!(lines.contains(&"dist/*.zip"));
}

#[test]
fn release_has_only_the_permissions_attestation_needs() {
    let text = release_yml();
    for p in ["contents: write", "id-token: write", "attestations: write"] {
        assert!(
            text.lines().any(|l| l.trim() == p),
            "release.yml needs `{p}`"
        );
    }
    assert!(!text.contains("artifact-metadata"));
    assert!(
        text.lines()
            .any(|l| l.trim() == "create-storage-record: false")
    );
}

#[test]
fn ci_cannot_mint_signing_tokens() {
    let ci =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/ci.yml"))
            .unwrap();
    assert!(!ci.contains("id-token"));
    assert!(!ci.contains("attestations"));
}

#[test]
fn docs_explain_attestation_verification() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cmd =
        "gh attestation verify claude-codex-taskbar.exe --repo chiarorosa/claude-codex-taskbar";
    for doc in ["README.md", "SECURITY.md"] {
        let text = fs::read_to_string(root.join(doc)).unwrap();
        assert!(text.contains(cmd), "{doc} must show `{cmd}`");
    }
}

/// Cargo dependencies get update PRs too. Minor and patch bumps are grouped, but the `windows`
/// crate (a 0.x crate whose "minor" bumps break the API) and real majors come one per PR.
#[test]
fn dependabot_updates_cargo_with_windows_kept_separate() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/dependabot.yml");
    let text = fs::read_to_string(path).unwrap();
    let cargo = text
        .split("- package-ecosystem:")
        .find(|block| block.trim_start().starts_with("\"cargo\""))
        .expect("a cargo ecosystem entry");
    let lines: Vec<&str> = cargo.lines().map(str::trim).collect();
    assert!(lines.contains(&"directory: \"/\""));
    assert!(lines.contains(&"interval: \"monthly\""));
    assert!(lines.contains(&"update-types: [\"minor\", \"patch\"]"));
    assert!(lines.contains(&"exclude-patterns: [\"windows\", \"windows-*\"]"));
}
