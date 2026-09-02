//! Centralized Taskfleet/legacy public-input and state-home resolution.
//!
//! During the 0.6/0.7 compatibility window every public branded input has one
//! canonical `TASKFLEET_*` spelling and one deprecated `ORCHESTRATECTL_*`
//! alias.  This module is the only place that reads those variables or chooses
//! between `.taskfleet.toml` and `.orchestratectl.toml`.
//!
//! A default root is **populated** when it is a readable directory containing
//! at least one entry.  Every entry counts as managed: guessing whether an
//! unfamiliar file is ours could discard newer-version state. Empty or absent
//! directories are unpopulated. Inaccessible paths and non-directories fail
//! closed. A sole populated legacy root is adopted in place; no directory is
//! moved and no symlink is created.

use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use crate::error::CliError;

pub const HOME_ENV: &str = "TASKFLEET_HOME";
pub const LEGACY_HOME_ENV: &str = "ORCHESTRATECTL_HOME";
pub const PROFILE_ENV: &str = "TASKFLEET_PROFILE";
pub const LEGACY_PROFILE_ENV: &str = "ORCHESTRATECTL_PROFILE";
pub const HARNESS_ENV: &str = "TASKFLEET_HARNESS";
pub const LEGACY_HARNESS_ENV: &str = "ORCHESTRATECTL_HARNESS";
pub const LOG_ENV: &str = "TASKFLEET_LOG";
pub const LEGACY_LOG_ENV: &str = "ORCHESTRATECTL_LOG";
pub const INTERNAL_SELF_EXEC_ENV: &str = "OCTL_INTERNAL_SELF_EXEC";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HomeSource {
    CanonicalExplicit,
    LegacyExplicit,
    EqualExplicit,
    CanonicalDefault,
    AdoptedLegacyDefault,
    InternalWorker,
}

pub enum RepositorySource<'a> {
    NotApplicable,
    RunCreate(Option<&'a str>),
}

#[derive(Clone, Debug)]
struct RepositoryConfigSelection {
    path: PathBuf,
    bytes: Option<Vec<u8>>,
}

#[derive(Clone, Debug)]
struct Inputs {
    root: PathBuf,
    home_source: HomeSource,
    profile: Option<String>,
    harness: Option<String>,
    log: Option<String>,
    repository_config: Option<RepositoryConfigSelection>,
}

static INPUTS: OnceLock<Inputs> = OnceLock::new();

/// Resolve every branded environment input and the authoritative state root.
/// Called once by the dispatcher after argument parsing and before logging or
/// any command filesystem write. The resolved values are then immutable for
/// this process. This does not fence an unmodified 0.5.1 process: operators
/// must exclude concurrent first establishment as required by ADR 0002.
pub fn initialize(
    internal_worker_root: Option<PathBuf>,
    repository_source: RepositorySource<'_>,
) -> Result<Vec<String>, CliError> {
    if INPUTS.get().is_some() {
        return Ok(Vec::new());
    }
    let mut warnings = Vec::new();
    let (root, home_source) = if let Some(root) = internal_worker_root {
        if !root.is_absolute() {
            return Err(CliError::system(
                "invalid_internal_home",
                "OCTL_INTERNAL_WORKER_STATE_ROOT must be absolute",
            ));
        }
        let root = path_for_use(&root)?;
        validate_selected_home(&root)?;
        (root, HomeSource::InternalWorker)
    } else {
        resolve_root(&mut warnings)?
    };
    let profile = resolve_text(PROFILE_ENV, LEGACY_PROFILE_ENV, true, &mut warnings)?;
    let harness = resolve_text(HARNESS_ENV, LEGACY_HARNESS_ENV, true, &mut warnings)?;
    let log = resolve_text(LOG_ENV, LEGACY_LOG_ENV, false, &mut warnings)?;
    let repository_config = match repository_source {
        RepositorySource::NotApplicable => None,
        RepositorySource::RunCreate(source) => {
            Some(resolve_repository_config(source, &mut warnings)?)
        }
    };
    let _ = INPUTS.set(Inputs {
        root,
        home_source,
        profile,
        harness,
        log,
        repository_config,
    });
    Ok(warnings)
}

pub fn root_dir() -> Result<PathBuf, CliError> {
    if let Some(inputs) = INPUTS.get() {
        return Ok(inputs.root.clone());
    }
    let mut warnings = Vec::new();
    resolve_root(&mut warnings).map(|(root, _)| root)
}

pub fn profile() -> Result<Option<String>, CliError> {
    selected_text(PROFILE_ENV, LEGACY_PROFILE_ENV, |i| &i.profile)
}

pub fn harness() -> Result<Option<String>, CliError> {
    selected_text(HARNESS_ENV, LEGACY_HARNESS_ENV, |i| &i.harness)
}

pub fn log_filter() -> Result<Option<String>, CliError> {
    selected_text(LOG_ENV, LEGACY_LOG_ENV, |i| &i.log)
}

fn selected_text(
    canonical: &str,
    legacy: &str,
    get: impl FnOnce(&Inputs) -> &Option<String>,
) -> Result<Option<String>, CliError> {
    if let Some(inputs) = INPUTS.get() {
        return Ok(get(inputs).clone());
    }
    resolve_text(canonical, legacy, canonical != LOG_ENV, &mut Vec::new())
}

pub fn home_source() -> Result<HomeSource, CliError> {
    INPUTS
        .get()
        .map(|inputs| inputs.home_source)
        .ok_or_else(|| {
            CliError::system(
                "resolver_not_initialized",
                "home resolver was not initialized",
            )
        })
}

fn resolve_root(warnings: &mut Vec<String>) -> Result<(PathBuf, HomeSource), CliError> {
    let canonical = env_path(HOME_ENV)?;
    let legacy = env_path(LEGACY_HOME_ENV)?;
    match (canonical, legacy) {
        (Some(new), Some(old)) => {
            let new_path = path_for_use(&new)?;
            let old_path = path_for_use(&old)?;
            let new_key = normalize(&new)?;
            let old_key = normalize(&old)?;
            if new_key != old_key {
                return Err(CliError::user(
                    "conflicting_home",
                    format!(
                        "{HOME_ENV} ({}) and {LEGACY_HOME_ENV} ({}) resolve to different paths; set exactly one authoritative home",
                        new_path.display(), old_path.display()
                    ),
                ));
            }
            warnings.push(format!(
                "{LEGACY_HOME_ENV} is deprecated; both home variables resolve to {}",
                new_path.display()
            ));
            validate_selected_home(&new_path)?;
            Ok((new_path, HomeSource::EqualExplicit))
        }
        (Some(path), None) => {
            let path = path_for_use(&path)?;
            let _ = normalize(&path)?;
            validate_selected_home(&path)?;
            Ok((path, HomeSource::CanonicalExplicit))
        }
        (None, Some(path)) => {
            let path = path_for_use(&path)?;
            let _ = normalize(&path)?;
            validate_selected_home(&path)?;
            warnings.push(format!(
                "{LEGACY_HOME_ENV} is deprecated; using legacy home {} in place",
                path.display()
            ));
            Ok((path, HomeSource::LegacyExplicit))
        }
        (None, None) => resolve_default_root(warnings),
    }
}

fn resolve_default_root(warnings: &mut Vec<String>) -> Result<(PathBuf, HomeSource), CliError> {
    let home = std::env::var_os("HOME").ok_or_else(|| {
        CliError::system(
            "home_not_set",
            format!("neither {HOME_ENV}, {LEGACY_HOME_ENV}, nor HOME is set"),
        )
    })?;
    if home.is_empty() {
        return Err(CliError::system(
            "home_not_set",
            "HOME is set to an empty string",
        ));
    }
    let canonical = path_for_use(&PathBuf::from(&home).join(".taskfleet"))?;
    let legacy = path_for_use(&PathBuf::from(home).join(".orchestratectl"))?;
    let canonical_populated = populated(&canonical)?;
    let legacy_populated = populated(&legacy)?;
    match (canonical_populated, legacy_populated) {
        (true, true) if normalize(&canonical)? == normalize(&legacy)? => {
            warnings.push(format!(
                "canonical and legacy default homes resolve to the same populated directory {}; using the canonical path",
                canonical.display()
            ));
            Ok((canonical, HomeSource::CanonicalDefault))
        }
        (true, true) => Err(CliError::user(
            "conflicting_state_homes",
            format!(
                "both canonical home {} and legacy home {} contain managed data; refusing to choose or merge them",
                canonical.display(), legacy.display()
            ),
        )),
        (false, true) => {
            warnings.push(format!(
                "adopting populated legacy home {} in place; migrate explicitly before removing compatibility",
                legacy.display()
            ));
            Ok((legacy, HomeSource::AdoptedLegacyDefault))
        }
        (_, false) => Ok((canonical, HomeSource::CanonicalDefault)),
    }
}

fn env_path(name: &str) -> Result<Option<PathBuf>, CliError> {
    match std::env::var_os(name) {
        None => Ok(None),
        Some(value) if value.is_empty() => Err(CliError::user(
            "home_not_set",
            format!("{name} is set to an empty string"),
        )),
        Some(value) => Ok(Some(PathBuf::from(value))),
    }
}

fn resolve_text(
    canonical: &str,
    legacy: &str,
    trim: bool,
    warnings: &mut Vec<String>,
) -> Result<Option<String>, CliError> {
    let normalize_value = |value: String| {
        if trim {
            let value = value.trim().to_string();
            (!value.is_empty()).then_some(value)
        } else {
            Some(value)
        }
    };
    let new = env_utf8(canonical)?.and_then(&normalize_value);
    let old = env_utf8(legacy)?.and_then(&normalize_value);
    match (new, old) {
        (Some(new), Some(old)) if new != old => Err(CliError::user(
            "conflicting_environment",
            format!("{canonical} and {legacy} have different values; set only {canonical}"),
        )),
        (Some(value), Some(_)) => {
            warnings.push(format!(
                "{legacy} is deprecated; both variables have the same value"
            ));
            Ok(Some(value))
        }
        (Some(value), None) => Ok(Some(value)),
        (None, Some(value)) => {
            warnings.push(format!("{legacy} is deprecated; use {canonical}"));
            Ok(Some(value))
        }
        (None, None) => Ok(None),
    }
}

fn env_utf8(name: &str) -> Result<Option<String>, CliError> {
    match std::env::var(name) {
        Ok(value) if value.is_empty() => Ok(None),
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(CliError::user(
            "invalid_environment",
            format!("environment variable {name} is not valid UTF-8"),
        )),
    }
}

/// Normalize path spelling without requiring the final path to exist. Existing
/// paths are canonicalized (therefore resolving symlinks and the filesystem's
/// case rules). For a missing path, the nearest existing ancestor is
/// canonicalized and the lexical remainder is appended. Relative inputs are
/// anchored to the invocation cwd.
fn path_for_use(path: &Path) -> Result<PathBuf, CliError> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map_err(|e| {
                CliError::system("home_unreadable", format!("read current directory: {e}"))
            })
            .map(|cwd| cwd.join(path))
    }
}

fn normalize(path: &Path) -> Result<PathBuf, CliError> {
    let absolute = path_for_use(path)?;
    // Canonicalize the original spelling first: lexical `link/..` reduction
    // is not equivalent when `link` is a symlink. Only use lexical cleanup
    // after the full path is known not to exist.
    match std::fs::canonicalize(&absolute) {
        Ok(path) => return Ok(path),
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            return Err(CliError::system(
                "home_unreadable",
                format!("resolve home path {}: {e}", absolute.display()),
            ));
        }
        Err(_) => {}
    }
    let lexical = lexical_normalize(&absolute);
    let mut missing: Vec<OsString> = Vec::new();
    let mut ancestor = absolute.as_path();
    loop {
        match std::fs::canonicalize(ancestor) {
            Ok(mut base) => {
                for component in missing.iter().rev() {
                    base.push(component);
                }
                return Ok(base);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let Some(name) = ancestor.file_name() else {
                    return Ok(lexical);
                };
                missing.push(name.to_os_string());
                let Some(parent) = ancestor.parent() else {
                    return Ok(lexical);
                };
                ancestor = parent;
            }
            Err(e) => {
                return Err(CliError::system(
                    "home_unreadable",
                    format!("resolve home path {}: {e}", lexical.display()),
                ));
            }
        }
    }
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let _ = out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn validate_selected_home(path: &Path) -> Result<(), CliError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => match std::fs::metadata(path) {
            Ok(target) if target.is_dir() => std::fs::read_dir(path).map(|_| ()).map_err(|e| {
                CliError::system(
                    "home_unreadable",
                    format!("could not inspect state home {}: {e}", path.display()),
                )
            }),
            Ok(_) => Err(CliError::user(
                "invalid_home",
                format!(
                    "state home {} is a symlink to a non-directory",
                    path.display()
                ),
            )),
            Err(e) => Err(CliError::system(
                "home_unreadable",
                format!(
                    "state home {} is a dangling or inaccessible symlink: {e}",
                    path.display()
                ),
            )),
        },
        Ok(metadata) if !metadata.is_dir() => Err(CliError::user(
            "invalid_home",
            format!(
                "state home {} exists but is not a directory",
                path.display()
            ),
        )),
        Ok(_) => std::fs::read_dir(path).map(|_| ()).map_err(|e| {
            CliError::system(
                "home_unreadable",
                format!("could not inspect state home {}: {e}", path.display()),
            )
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(CliError::system(
            "home_unreadable",
            format!("could not inspect state home {}: {e}", path.display()),
        )),
    }
}

fn populated(path: &Path) -> Result<bool, CliError> {
    validate_selected_home(path)?;
    match std::fs::metadata(path) {
        Ok(metadata) if !metadata.is_dir() => Err(CliError::user(
            "invalid_home",
            format!(
                "state home {} exists but is not a directory",
                path.display()
            ),
        )),
        Ok(_) => std::fs::read_dir(path)
            .map_err(|e| {
                CliError::system(
                    "home_unreadable",
                    format!("could not inspect state home {}: {e}", path.display()),
                )
            })?
            .next()
            .transpose()
            .map(|entry| entry.is_some())
            .map_err(|e| {
                CliError::system(
                    "home_unreadable",
                    format!("could not inspect state home {}: {e}", path.display()),
                )
            }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(CliError::system(
            "home_unreadable",
            format!("could not inspect state home {}: {e}", path.display()),
        )),
    }
}

/// Return the repository config bytes frozen by dispatcher preflight.
pub fn repository_config() -> Result<(PathBuf, Option<Vec<u8>>), CliError> {
    INPUTS
        .get()
        .and_then(|inputs| inputs.repository_config.as_ref())
        .map(|selection| (selection.path.clone(), selection.bytes.clone()))
        .ok_or_else(|| {
            CliError::system(
                "resolver_not_initialized",
                "repository config was not resolved during dispatcher preflight",
            )
        })
}

fn resolve_repository_config(
    source_repo: Option<&str>,
    warnings: &mut Vec<String>,
) -> Result<RepositoryConfigSelection, CliError> {
    let root = repository_root(source_repo)?;
    let canonical = root.join(".taskfleet.toml");
    let legacy = root.join(".orchestratectl.toml");
    let new = read_optional(&canonical)?;
    let old = read_optional(&legacy)?;
    match (new, old) {
        (Some(new), Some(old)) if new != old => Err(CliError::user(
            "conflicting_repository_config",
            format!(
                "{} and {} differ; keep one authoritative repository config",
                canonical.display(),
                legacy.display()
            ),
        )),
        (Some(new), Some(_)) => {
            warnings.push(format!(
                "{} is deprecated; both repository config files are byte-identical",
                legacy.display()
            ));
            Ok(RepositoryConfigSelection {
                path: canonical,
                bytes: Some(new),
            })
        }
        (Some(new), None) => Ok(RepositoryConfigSelection {
            path: canonical,
            bytes: Some(new),
        }),
        (None, None) => Ok(RepositoryConfigSelection {
            path: canonical,
            bytes: None,
        }),
        (None, Some(old)) => {
            warnings.push(format!(
                "{} is deprecated; use {}",
                legacy.display(),
                canonical.display()
            ));
            Ok(RepositoryConfigSelection {
                path: legacy,
                bytes: Some(old),
            })
        }
    }
}

fn repository_root(source_repo: Option<&str>) -> Result<PathBuf, CliError> {
    let start = source_repo
        .map_or_else(std::env::current_dir, |raw| Ok(PathBuf::from(raw)))
        .map_err(|e| CliError::system("io_error", format!("read current directory: {e}")))?;
    if !start.is_dir() {
        return Err(CliError::user(
            "invalid_source_repo",
            format!(
                "source repository {} is not an existing directory",
                start.display()
            ),
        )
        .with_invalid_value(start.display().to_string()));
    }
    let canonical = start.canonicalize().map_err(|e| {
        CliError::system(
            "source_repo_unreadable",
            format!("resolve source repository {}: {e}", start.display()),
        )
    })?;
    for ancestor in canonical.ancestors() {
        let marker = ancestor.join(".git");
        match marker.try_exists() {
            Ok(true) => return Ok(ancestor.to_path_buf()),
            Ok(false) => {}
            Err(e) => {
                return Err(CliError::system(
                    "source_repo_unreadable",
                    format!("inspect repository marker {}: {e}", marker.display()),
                ));
            }
        }
    }
    Ok(canonical)
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, CliError> {
    const MAX_REPOSITORY_CONFIG_BYTES: u64 = 64 * 1024;
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(CliError::system(
                "repository_config_unreadable",
                format!(
                    "could not inspect repository config {}: {e}",
                    path.display()
                ),
            ));
        }
    };
    if !metadata.file_type().is_file() || metadata.len() > MAX_REPOSITORY_CONFIG_BYTES {
        return Err(CliError::user(
            "invalid_repository_config",
            format!(
                "repository config {} must be a regular file no larger than {MAX_REPOSITORY_CONFIG_BYTES} bytes",
                path.display()
            ),
        ));
    }
    std::fs::read(path).map(Some).map_err(|e| {
        CliError::system(
            "repository_config_unreadable",
            format!("could not read repository config {}: {e}", path.display()),
        )
    })
}

pub fn warnings_suppressed() -> bool {
    std::env::var_os(INTERNAL_SELF_EXEC_ENV).is_some_and(|v| v == OsStr::new("1"))
}
