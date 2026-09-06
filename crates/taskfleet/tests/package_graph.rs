use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

#[test]
fn release_topology_has_exact_ordered_and_independent_legs() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let topology: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("release/taskfleet-release.json")).expect("release topology"),
    )
    .expect("release topology JSON");
    assert_eq!(topology["schema_version"], 1);
    assert_eq!(topology["activation"], "ready");
    assert_eq!(topology["repository"], "jarimustonen/taskfleet");
    assert_eq!(topology["owners"], serde_json::json!(["jarimustonen"]));
    assert_eq!(
        topology["crates_io"]["legs"],
        serde_json::json!([
            {"package":"taskfleet-core","manifest":"crates/taskfleet-core/Cargo.toml","depends_on":null},
            {"package":"taskfleet","manifest":"crates/taskfleet/Cargo.toml","depends_on":"taskfleet-core"},
            {"package":"orchestratectl","manifest":"compat/orchestratectl/Cargo.toml","depends_on":"taskfleet"}
        ])
    );
    assert_eq!(
        topology["distribution"],
        serde_json::json!([
            {"package":"taskfleet","registry":"gh-releases","workflow":"release.yml"},
            {"package":"taskfleet","registry":"homebrew","workflow":"release.yml"}
        ])
    );
}

#[test]
fn active_distribution_is_taskfleet_only_and_structurally_authorized() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let distribution: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("release/taskfleet-distribution.json"))
            .expect("distribution topology"),
    )
    .expect("distribution topology JSON");
    assert_eq!(distribution["schema_version"], 1);
    assert_eq!(distribution["activation"], "ready");
    assert_eq!(
        distribution["cargo_dist"]["version"],
        serde_json::json!("0.28.2")
    );
    assert_eq!(distribution["cargo_dist"]["trigger"], "tag-push");
    assert_eq!(
        distribution["cargo_dist"]["apps"],
        serde_json::json!(["taskfleet"])
    );
    assert_eq!(
        distribution["cargo_dist"]["tap"],
        "jarimustonen/homebrew-taskfleet"
    );
    assert_eq!(
        distribution["cargo_dist"]["tap_secret_state"],
        "active-proven-r10"
    );
    assert_eq!(distribution["cargo_dist"]["pr_run_mode"], "skip");
    assert_eq!(
        distribution["cargo_dist"]["activation_gate"],
        "scripts/verify-release-tag-authorization.sh"
    );
    assert_eq!(
        distribution["cargo_dist"]["authorization"],
        "wrapper-ref-exact-tag-main-green-ci"
    );
    assert_eq!(
        distribution["cargo_dist"]["release_tag_ruleset"],
        22_234_415
    );
    assert_eq!(
        distribution["cargo_dist"]["authorization_ref_ruleset"],
        22_234_417
    );
    assert_eq!(distribution["cargo_dist"]["macos_runner"], "macOS");
    assert_eq!(
        distribution["cargo_dist"]["stub_sha256"],
        "6d171a7e0e4be8dec9518d6a888ea73400c0ccebf0a0d2f68b0f41cf5414653b"
    );
    assert_eq!(distribution["old_tap"]["activation"], "blocked-r11");

    let workflow = std::fs::read_to_string(root.join(".github/workflows/release.yml"))
        .expect("generated release workflow");
    assert!(workflow.contains("push:\n    tags:"));
    assert!(!workflow.contains("pull_request:"));
    assert!(!workflow.contains("workflow_dispatch:"));
    assert!(!workflow.contains("secrets: inherit"));
    assert!(!workflow.contains("custom-taskfleet-release-gate"));
    assert_eq!(
        workflow
            .matches("name: \"Require wrapper-authorized exact-main release tag\"")
            .count(),
        1
    );
    assert!(workflow.contains("run: \"./scripts/verify-release-tag-authorization.sh\""));
    assert_eq!(
        workflow
            .matches("repository: \"jarimustonen/homebrew-taskfleet\"")
            .count(),
        1
    );
    assert!(!workflow.contains("repository: \"jarimustonen/homebrew-orchestratectl\""));
    assert_eq!(
        workflow
            .matches("token: ${{ secrets.HOMEBREW_TAP_TOKEN }}")
            .count(),
        1
    );

    let build_setup = std::fs::read_to_string(root.join(".github/build-setup.yml"))
        .expect("cargo-dist build setup");
    assert!(build_setup.contains("name: Require wrapper-authorized exact-main release tag"));
    assert!(build_setup.contains("run: ./scripts/verify-release-tag-authorization.sh"));
    assert!(workflow.contains("\"contents\": \"write\""));
}

#[test]
fn normalized_workspace_graph_has_one_engine_and_one_compatibility_binary() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new(env!("CARGO"))
        .current_dir(&root)
        .args(["metadata", "--locked", "--no-deps", "--format-version", "1"])
        .output()
        .expect("cargo metadata");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("metadata JSON");
    let packages = metadata["packages"].as_array().expect("packages");
    let by_name: BTreeMap<_, _> = packages
        .iter()
        .map(|package| (package["name"].as_str().unwrap(), package))
        .collect();
    assert_eq!(
        by_name.keys().copied().collect::<Vec<_>>(),
        ["orchestratectl", "taskfleet", "taskfleet-core"]
    );

    let core = by_name["taskfleet-core"];
    assert!(core["targets"]
        .as_array()
        .unwrap()
        .iter()
        .all(|target| target["kind"] != serde_json::json!(["bin"])));

    let canonical = by_name["taskfleet"];
    assert_eq!(binary_names(canonical), ["taskfleet"]);
    assert_eq!(canonical["metadata"]["dist"]["dist"], true);
    assert!(
        canonical["metadata"].get("taskfleet").is_none(),
        "canonical package must not carry a stale pre-cut marker"
    );
    assert_exact_dependency(canonical, "taskfleet-core");

    let compatibility = by_name["orchestratectl"];
    assert_eq!(binary_names(compatibility), ["orchestratectl"]);
    assert_eq!(compatibility["metadata"]["dist"]["dist"], false);
    assert!(compatibility["metadata"]["taskfleet"]
        .get("pre-cut")
        .is_none());
    assert_eq!(
        compatibility["metadata"]["taskfleet"]["release-window"],
        "0.6.x-0.7.x"
    );
    assert_exact_dependency(compatibility, "taskfleet");

    assert!(
        !by_name.contains_key("octl-core"),
        "no old core wrapper may exist"
    );
}

fn binary_names(package: &serde_json::Value) -> Vec<&str> {
    package["targets"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|target| target["kind"] == serde_json::json!(["bin"]))
        .map(|target| target["name"].as_str().unwrap())
        .collect()
}

fn assert_exact_dependency(package: &serde_json::Value, name: &str) {
    let dependency = package["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|dependency| dependency["name"] == name)
        .unwrap_or_else(|| panic!("missing {name} dependency"));
    assert_eq!(dependency["req"], format!("={}", env!("CARGO_PKG_VERSION")));
}
