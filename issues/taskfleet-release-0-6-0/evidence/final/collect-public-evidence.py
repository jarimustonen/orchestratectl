#!/usr/bin/env python3
"""Re-query public R10 destinations and emit a sanitized receipt directory."""
import hashlib, json, os, pathlib, shutil, subprocess, sys, tarfile, tempfile, urllib.error, urllib.request
from datetime import datetime, timezone

UA = "taskfleet-r10-evidence/1.0 (https://github.com/jarimustonen/taskfleet)"
REPO = "jarimustonen/taskfleet"
V0 = "57f6dfb83401694399b363de5d3aa88e4541a22c"
V1 = "7e93bd6195fbaf6de0b43d9161228ae2373ab5d1"
TAP = "c9e68594340b2b775d23159a3545d53f15306471"
OLD_TAP = "85ce830378f38cf17283efddd966d5754354e403"
OUT = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else ".")
OUT.mkdir(parents=True, exist_ok=True)

def get(url, expected=200):
    headers = {"User-Agent": UA}
    if url.startswith("https://api.github.com/"):
        token = os.environ.get("GH_TOKEN")
        if not token:
            token = subprocess.run(["gh", "auth", "token"], text=True, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, check=True).stdout.strip()
        headers["Authorization"] = f"Bearer {token}"
    req = urllib.request.Request(url, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            body, status = r.read(), r.status
    except urllib.error.HTTPError as e:
        body, status = e.read(), e.code
    if status != expected:
        raise SystemExit(f"{url}: expected HTTP {expected}, got {status}")
    return body

def json_get(url, expected=200):
    return json.loads(get(url, expected)) if expected == 200 else get(url, expected)

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def run(*args, env=None, expected=0):
    p = subprocess.run(args, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=300)
    if p.returncode != expected:
        raise SystemExit(f"command failed ({p.returncode}, expected {expected}): {' '.join(args)}\n{p.stderr}")
    return p

collected = datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")
crates = []
with tempfile.TemporaryDirectory(prefix="taskfleet-r10-public-") as td:
    temp = pathlib.Path(td)
    for package, pin in (("taskfleet-core", None), ("taskfleet", ("taskfleet-core", "=0.6.1")), ("orchestratectl", ("taskfleet", "=0.6.1"))):
        json_get(f"https://crates.io/api/v1/crates/{package}/0.6.0", 404)
        data = json_get(f"https://crates.io/api/v1/crates/{package}/0.6.1")["version"]
        deps = json_get(f"https://crates.io/api/v1/crates/{package}/0.6.1/dependencies")["dependencies"]
        if data["num"] != "0.6.1" or data["yanked"] or data["repository"] != "https://github.com/jarimustonen/taskfleet":
            raise SystemExit(f"unexpected crates.io metadata for {package}")
        if pin and not any(d["crate_id"] == pin[0] and d["req"] == pin[1] and d["kind"] == "normal" for d in deps):
            raise SystemExit(f"missing exact pin {pin} in {package}")
        archive = temp / f"{package}-0.6.1.crate"
        archive.write_bytes(get(f"https://crates.io/api/v1/crates/{package}/0.6.1/download"))
        if sha(archive) != data["checksum"]:
            raise SystemExit(f"registry checksum mismatch for {package}")
        with tarfile.open(archive, "r:gz") as tf:
            vcs_member = next((m for m in tf.getmembers() if m.name.endswith("/.cargo_vcs_info.json")), None)
            vcs = json.load(tf.extractfile(vcs_member)) if vcs_member else None
        if not vcs or vcs["git"]["sha1"] != V1 or vcs["path_in_vcs"] is None:
            raise SystemExit(f"source commit mismatch for {package}")
        crates.append({"package": package, "version": data["num"], "checksum": data["checksum"], "download_sha256": sha(archive), "yanked": data["yanked"], "repository": data["repository"], "rust_version": data["rust_version"], "exact_pin": ({"package": pin[0], "requirement": pin[1]} if pin else None), "source_commit": vcs["git"]["sha1"], "path_in_vcs": vcs["path_in_vcs"]})

    # GitHub release absence/presence and every uploaded asset digest.
    json_get(f"https://api.github.com/repos/{REPO}/releases/tags/v0.6.0", 404)
    rel = json_get(f"https://api.github.com/repos/{REPO}/releases/tags/v0.6.1")
    if rel["tag_name"] != "v0.6.1" or rel["target_commitish"] != V1 or rel["draft"] or rel["prerelease"]:
        raise SystemExit("unexpected GitHub Release metadata")
    assets = []
    downloaded = {}
    for asset in rel["assets"]:
        path = temp / asset["name"]
        path.write_bytes(get(asset["browser_download_url"]))
        digest = "sha256:" + sha(path)
        if asset.get("digest") != digest:
            raise SystemExit(f"GitHub asset digest mismatch: {asset['name']}")
        assets.append({"id": asset["id"], "name": asset["name"], "size": asset["size"], "content_type": asset["content_type"], "digest": digest, "created_at": asset["created_at"]})
        downloaded[asset["name"]] = path
    expected_names = {"dist-manifest.json", "orchestratectl-installer.sh", "sha256.sum", "source.tar.gz", "source.tar.gz.sha256", "taskfleet-aarch64-apple-darwin.tar.xz", "taskfleet-aarch64-apple-darwin.tar.xz.sha256", "taskfleet-aarch64-unknown-linux-gnu.tar.xz", "taskfleet-aarch64-unknown-linux-gnu.tar.xz.sha256", "taskfleet-installer.sh", "taskfleet-x86_64-unknown-linux-gnu.tar.xz", "taskfleet-x86_64-unknown-linux-gnu.tar.xz.sha256", "taskfleet.rb"}
    if set(downloaded) != expected_names:
        raise SystemExit("unexpected GitHub Release asset set")
    checksum_lines = [line.split() for line in downloaded["sha256.sum"].read_text().splitlines() if line.strip()]
    for digest, name in checksum_lines:
        if name.startswith("*"): name = name[1:]
        if name not in downloaded or sha(downloaded[name]) != digest:
            raise SystemExit(f"sha256.sum mismatch: {name}")

    # Native archive runtime and alias absence.
    archive = downloaded["taskfleet-aarch64-apple-darwin.tar.xz"]
    extract = temp / "archive"
    extract.mkdir()
    with tarfile.open(archive, "r:xz") as tf:
        names = tf.getnames()
        tf.extractall(extract)
    if any(pathlib.PurePosixPath(n).name == "orchestratectl" for n in names):
        raise SystemExit("legacy binary present in canonical archive")
    binary = next(extract.rglob("taskfleet"))
    home = temp / "archive-home"
    home.mkdir()
    env = {"HOME": str(home), "PATH": "/usr/bin:/bin", "TASKFLEET_HOME": str(temp / "archive-state")}
    archive_version = json.loads(run(str(binary), "version", "--output", "json", env=env).stdout)["data"]
    if archive_version["version"] != "0.6.1" or archive_version["commit"] != V1:
        raise SystemExit("archive runtime identity mismatch")

    # Legacy latest-installer stub is inert.
    stub_home = temp / "stub-home"
    stub_home.mkdir()
    stub = run("sh", str(downloaded["orchestratectl-installer.sh"]), env={"HOME": str(stub_home), "PATH": "/usr/bin:/bin"}, expected=1)
    if any(stub_home.iterdir()) or "taskfleet-installer.sh" not in stub.stderr:
        raise SystemExit("legacy installer stub was not inert")

    # Canonical shell installer into an isolated Cargo home.
    shell_home = temp / "shell-home"
    cargo_home = temp / "cargo-home"
    shell_home.mkdir()
    installer_env = dict(os.environ, HOME=str(shell_home), CARGO_HOME=str(cargo_home), TASKFLEET_HOME=str(temp / "shell-state"))
    run("gtimeout", "300", "sh", str(downloaded["taskfleet-installer.sh"]), "--no-modify-path", "--quiet", env=installer_env)
    installed = sorted(p.name for p in (cargo_home / "bin").iterdir())
    if installed != ["taskfleet"]:
        raise SystemExit(f"unexpected shell-installed binaries: {installed}")
    shell_version = json.loads(run(str(cargo_home / "bin/taskfleet"), "version", "--output", "json", env={"HOME": str(shell_home), "PATH": "/usr/bin:/bin", "TASKFLEET_HOME": str(temp / "shell-state")}).stdout)["data"]
    if shell_version["version"] != "0.6.1" or shell_version["commit"] != V1:
        raise SystemExit("shell-installed runtime identity mismatch")

# Public refs (annotated tags are peeled) and tap history/state.
def ls_remote(ref):
    return run("git", "ls-remote", "https://github.com/jarimustonen/taskfleet.git", ref, ref + "^{}").stdout.strip().splitlines()
def peeled(lines):
    pairs = {line.split()[1]: line.split()[0] for line in lines}
    return next((oid for ref, oid in pairs.items() if ref.endswith("^{}")), next(iter(pairs.values())))
refs = {}
for version, commit in (("v0.6.0", V0), ("v0.6.1", V1)):
    tag_lines = ls_remote(f"refs/tags/{version}")
    auth_lines = ls_remote(f"refs/heads/taskfleet-release-authorizations/{version}")
    if peeled(tag_lines) != commit or peeled(auth_lines) != commit:
        raise SystemExit(f"tag/authorization mismatch for {version}")
    tag_pairs = {line.split()[1]: line.split()[0] for line in tag_lines}
    refs[version] = {"commit": commit, "tag_ref": f"refs/tags/{version}", "tag_object_sha": tag_pairs[f"refs/tags/{version}"], "tag_commit_sha": peeled(tag_lines), "authorization_ref": f"refs/heads/taskfleet-release-authorizations/{version}", "authorization_commit_sha": peeled(auth_lines)}

def commit(repo, ref="main"):
    return json_get(f"https://api.github.com/repos/{repo}/commits/{ref}")["sha"]
if commit("jarimustonen/homebrew-taskfleet") != TAP or commit("jarimustonen/homebrew-orchestratectl") != OLD_TAP:
    raise SystemExit("tap head mismatch")
formula = get("https://raw.githubusercontent.com/jarimustonen/homebrew-taskfleet/main/Formula/taskfleet.rb").decode()
get("https://raw.githubusercontent.com/jarimustonen/homebrew-taskfleet/main/Formula/orchestratectl.rb", 404)
old_formula = get("https://raw.githubusercontent.com/jarimustonen/homebrew-orchestratectl/main/Formula/orchestratectl.rb").decode()
if ('version "0.6.1"' not in formula or
    'releases/download/v0.6.1/taskfleet-aarch64-apple-darwin.tar.xz' not in formula or
    'sha256 "4bbf3b023ae0377e8cdca41e07854cbb64165eba82ddc5ae70e1ba90386406a6"' not in formula):
    raise SystemExit("canonical formula version/archive mismatch")
if 'version "0.5.1"' not in old_formula:
    raise SystemExit("old tap changed unexpectedly")
tap_commits = json_get("https://api.github.com/repos/jarimustonen/homebrew-taskfleet/commits?per_page=100")
if len(tap_commits) != 2 or tap_commits[0]["sha"] != TAP:
    raise SystemExit("canonical tap history does not prove direct empty-to-v0.6.1 activation")

receipt = {"schema_version": 1, "collected_at": collected, "user_agent": UA,
 "v0_6_0": {"commit": V0, "public_crates": [{"package": p, "version": "0.6.0", "status": "absent_http_404"} for p in ("taskfleet-core", "taskfleet", "orchestratectl")], "github_release": "absent_http_404", "canonical_formula": "absent_from_two-commit-tap-history", "immutable_refs": refs["v0.6.0"]},
 "v0_6_1": {"commit": V1, "public_crates": crates, "github_release": {"id": rel["id"], "tag_name": rel["tag_name"], "target_commitish": rel["target_commitish"], "name": rel["name"], "draft": rel["draft"], "prerelease": rel["prerelease"], "published_at": rel["published_at"], "url": rel["html_url"], "assets": sorted(assets, key=lambda x: x["name"])}, "immutable_refs": refs["v0.6.1"]},
 "homebrew": {"canonical_tap_head": TAP, "canonical_tap_tree": json_get(f"https://api.github.com/repos/jarimustonen/homebrew-taskfleet/commits/{TAP}")["commit"]["tree"]["sha"], "canonical_formula_version": "0.6.1", "canonical_formula_sha256": hashlib.sha256(formula.encode()).hexdigest(), "canonical_tap_history": [{"sha": c["sha"], "message": c["commit"]["message"], "committed_at": c["commit"]["committer"]["date"]} for c in tap_commits], "old_tap_head": OLD_TAP, "old_formula_version": "0.5.1", "old_formula_sha256": hashlib.sha256(old_formula.encode()).hexdigest(), "orchestratectl_formula_in_canonical_tap": "absent_http_404"},
 "install_checks": {"archive": {"result": "pass", "version": archive_version["version"], "embedded_commit": archive_version["commit"], "orchestratectl_alias": "absent"}, "legacy_installer_stub": {"result": "pass", "exit_code": 1, "home_mutation": "absent", "canonical_url_message": "present"}, "shell_installer": {"result": "pass", "installed_binaries": installed, "version": shell_version["version"], "embedded_commit": shell_version["commit"]}}}
(OUT / "public-state.json").write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n")
print("public release evidence: pass")
