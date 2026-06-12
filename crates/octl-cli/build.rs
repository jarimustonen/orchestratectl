use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .ancestors()
        .find(|p| p.join(".git").exists())
        .map(|p| p.to_path_buf());

    let commit = Command::new("git")
        .arg("-C")
        .arg(workspace_root.as_deref().unwrap_or(&manifest_dir))
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

    if let Some(root) = workspace_root {
        let head_path = root.join(".git").join("HEAD");
        if let Ok(head) = std::fs::read_to_string(&head_path) {
            println!("cargo:rerun-if-changed={}", head_path.display());
            if let Some(rest) = head.strip_prefix("ref: ") {
                let ref_path = root.join(".git").join(rest.trim());
                println!("cargo:rerun-if-changed={}", ref_path.display());
            }
        }
    }
}
