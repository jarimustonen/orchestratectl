use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    generate_skill_files(&manifest_dir);

    let repo_root = manifest_dir
        .ancestors()
        .find(|p| p.join(".git").exists())
        .map(Path::to_path_buf);

    let commit = Command::new("git")
        .arg("-C")
        .arg(repo_root.as_deref().unwrap_or(&manifest_dir))
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout)
                    .ok()
                    .map(|s| s.trim().to_string())
            } else {
                None
            }
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=ORCHESTRATECTL_GIT_COMMIT={}", commit);

    // Set up rerun triggers so the commit env var refreshes when HEAD
    // moves. In a worktree, `.git` is a *file* whose contents are
    // `gitdir: <path>` pointing at the real per-worktree directory
    // under the main repo's `.git/worktrees/<name>/`. Both layouts are
    // handled.
    if let Some(root) = repo_root {
        let git_path = root.join(".git");
        let git_dir = if git_path.is_dir() {
            Some(git_path)
        } else if git_path.is_file() {
            std::fs::read_to_string(&git_path)
                .ok()
                .and_then(|s| {
                    s.lines()
                        .find_map(|l| l.strip_prefix("gitdir:").map(str::trim).map(str::to_string))
                })
                .map(PathBuf::from)
                .map(|p| if p.is_absolute() { p } else { root.join(p) })
        } else {
            None
        };

        if let Some(git_dir) = git_dir {
            let head_path = git_dir.join("HEAD");
            if let Ok(head) = std::fs::read_to_string(&head_path) {
                println!("cargo:rerun-if-changed={}", head_path.display());
                if let Some(rest) = head.strip_prefix("ref: ") {
                    let ref_path = git_dir.join(rest.trim());
                    println!("cargo:rerun-if-changed={}", ref_path.display());
                }
            }
        }
    }
}

/// Substitute `{{CLI_VERSION}}` in every `skills/<name>/SKILL.template.md`
/// with the crate's Cargo version, writing the result to
/// `$OUT_DIR/skills/<name>/SKILL.md`. Embedded into the binary via
/// `include_str!` at compile time so the shipped skill text always matches
/// the binary it ships with (AGENTS-AI-FIRST-CLI §17).
fn generate_skill_files(manifest_dir: &Path) {
    let cli_version = env!("CARGO_PKG_VERSION");
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let skills_dir = manifest_dir.join("skills");

    let entries = std::fs::read_dir(&skills_dir)
        .unwrap_or_else(|e| panic!("read {}: {}", skills_dir.display(), e));
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let template = path.join("SKILL.template.md");
        if !template.exists() {
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).expect("dirname");
        let body = std::fs::read_to_string(&template)
            .unwrap_or_else(|e| panic!("read {}: {}", template.display(), e));
        let rendered = body.replace("{{CLI_VERSION}}", cli_version);

        let out_skill_dir = out_dir.join("skills").join(name);
        std::fs::create_dir_all(&out_skill_dir)
            .unwrap_or_else(|e| panic!("mkdir {}: {}", out_skill_dir.display(), e));
        let out_file = out_skill_dir.join("SKILL.md");
        std::fs::write(&out_file, &rendered)
            .unwrap_or_else(|e| panic!("write {}: {}", out_file.display(), e));
        println!("cargo:rerun-if-changed={}", template.display());
    }
}
