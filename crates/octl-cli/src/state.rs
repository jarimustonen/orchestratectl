//! Explicit, quiescent state-root migration (ADR 0002 R3).
//!
//! The migration lock and receipt live under `$HOME/.taskfleet-migrations`,
//! deliberately outside both roots. Current binaries take the shared global
//! lock for every ordinary command; migration takes it exclusively. This does
//! not and cannot fence an already-running 0.5.1 binary or an open descriptor,
//! so the operator must stop old binaries and exclude concurrent invocations.

use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use chrono::Utc;
use fs4::FileExt;
use octl_core::{read_all_events, read_manifest, read_node, RunId, RunLock, RunPaths};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::CliError;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::from_core;

const ADMIN_DIR: &str = ".taskfleet-migrations";
const GLOBAL_LOCK: &str = "state.lock";
const RECEIPT_SCHEMA: u32 = 1;
const EXCLUSION: &str = "Operator-enforced exclusion is required: this lock cannot fence already-running orchestratectl 0.5.1 processes, future lock acquisitions by those binaries, or open file descriptors.";

#[derive(Debug, Clone, clap::Subcommand)]
pub enum StateAction {
    /// Atomically move a quiescent legacy state root to its canonical path.
    Migrate {
        /// Exact absolute, normalized legacy root. Symlinks are refused.
        #[arg(long)]
        source: PathBuf,
        /// Exact absolute, normalized canonical root. It must be absent.
        #[arg(long)]
        destination: PathBuf,
        /// Validate and emit the complete plan without writing a receipt or moving data.
        #[arg(long)]
        dry_run: bool,
    },
    /// Rename a verified migration back before its first canonical write.
    Rollback {
        /// Exact absolute legacy path to restore.
        #[arg(long)]
        source: PathBuf,
        /// Exact absolute canonical root produced by `state migrate`.
        #[arg(long)]
        destination: PathBuf,
        /// Validate and emit the rollback plan without moving data.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ReceiptState {
    Prepared,
    Renamed,
    Verified,
    RollbackPrepared,
    CanonicalWriteStarted,
    RolledBack,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Receipt {
    schema_version: u32,
    source: PathBuf,
    destination: PathBuf,
    source_device: u64,
    tree_sha256: String,
    state: ReceiptState,
    created_at: chrono::DateTime<Utc>,
    updated_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct MigrationPayload {
    operation: &'static str,
    dry_run: bool,
    /// Canon §11 planning mutations; empty for an applied result.
    would: Vec<PlannedMutation>,
    source: PathBuf,
    destination: PathBuf,
    receipt: PathBuf,
    state: ReceiptState,
    tree_sha256: String,
    run_count: usize,
    checks: MigrationChecks,
    first_canonical_write: &'static str,
    rollback_allowed: bool,
    operator_exclusion: &'static str,
}

#[derive(Debug, Serialize)]
struct MigrationChecks {
    same_filesystem: bool,
    destination_absent: bool,
    atomic_whole_root_rename: bool,
}

#[derive(Debug, Serialize)]
struct PlannedMutation {
    action: &'static str,
    resource: &'static str,
    from: PathBuf,
    to: PathBuf,
    known_effects: serde_json::Value,
    unknown_until_apply: Vec<&'static str>,
}

/// Shared external fence held for an ordinary command's whole lifetime.
pub struct CommandFence(File);

pub fn command_fence() -> Result<CommandFence, CliError> {
    let paths = admin_paths()?;
    ensure_admin_dir(&paths.dir, true)?;
    let file = open_lock(&paths.lock)?;
    match <File as FileExt>::try_lock_shared(&file) {
        Ok(()) => {}
        Err(fs4::TryLockError::WouldBlock) => {
            return Err(CliError::user(
                "migration_in_progress",
                "state migration holds the external fence; retry after it completes",
            ));
        }
        Err(fs4::TryLockError::Error(error)) => {
            return Err(io("acquire shared migration lock", &paths.lock, error));
        }
    }
    Ok(CommandFence(file))
}

impl Drop for CommandFence {
    fn drop(&mut self) {
        let _ = <File as FileExt>::unlock(&self.0);
    }
}

/// Close the rollback window before any ordinary command attempts a write in a
/// verified canonical root. Dispatcher calls this immediately before logging,
/// whose directory/file creation is the first canonical write in normal use.
pub fn mark_canonical_write_started(root: &Path) -> Result<(), CliError> {
    let paths = admin_paths()?;
    ensure_admin_dir(&paths.dir, false)?;
    let entries = match std::fs::read_dir(&paths.dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(io("scan migration receipts", &paths.dir, e)),
    };
    let mut receipts = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| io("scan migration receipts", &paths.dir, e))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("receipt-") || !name.ends_with(".json") {
            continue;
        }
        receipts.push((entry.path(), read_receipt(&entry.path())?));
    }
    // Pass one is read-only and checks every receipt. No directory iteration
    // order can close one rollback window before a later split-root conflict.
    for (_, receipt) in &receipts {
        if receipt.state != ReceiptState::RolledBack
            && path_exists(&receipt.source)?
            && path_exists(&receipt.destination)?
        {
            return Err(CliError::user(
                "conflicting_state_homes",
                format!(
                    "legacy root {} was recreated after migration to {}; refusing dual roots",
                    receipt.source.display(),
                    receipt.destination.display()
                ),
            ));
        }
    }
    let normalized_root = match root.canonicalize() {
        Ok(root) => root,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io("normalize selected state root", root, error)),
    };
    // Pass two advances at most the receipt whose physical destination is the
    // selected root, including symlink/alternate spellings.
    for (path, mut receipt) in receipts {
        let source = receipt
            .source
            .canonicalize()
            .unwrap_or_else(|_| receipt.source.clone());
        let destination = receipt
            .destination
            .canonicalize()
            .unwrap_or_else(|_| receipt.destination.clone());
        let selected_pair_root = source == normalized_root || destination == normalized_root;
        if selected_pair_root
            && matches!(
                receipt.state,
                ReceiptState::Prepared | ReceiptState::Renamed | ReceiptState::RollbackPrepared
            )
        {
            let command = if receipt.state == ReceiptState::RollbackPrepared {
                "state rollback"
            } else {
                "state migrate"
            };
            return Err(CliError::user(
                "migration_recovery_required",
                format!("migration receipt is {:?}; resume `{command}` before any ordinary command writes this root", receipt.state),
            ));
        }
        if destination == normalized_root && receipt.state == ReceiptState::Verified {
            receipt.state = ReceiptState::CanonicalWriteStarted;
            receipt.updated_at = Utc::now();
            write_receipt(&path, &receipt)?;
        }
    }
    Ok(())
}

pub fn dispatch(
    action: StateAction,
    spec: &OutputSpec,
    warnings: &[String],
) -> Result<(), CliError> {
    match action {
        StateAction::Migrate {
            source,
            destination,
            dry_run,
        } => migrate(&source, &destination, dry_run, spec, warnings),
        StateAction::Rollback {
            source,
            destination,
            dry_run,
        } => rollback(&source, &destination, dry_run, spec, warnings),
    }
}

fn migrate(
    source: &Path,
    destination: &Path,
    dry_run: bool,
    spec: &OutputSpec,
    warnings: &[String],
) -> Result<(), CliError> {
    let source = exact_root_path(source, "source")?;
    let destination = exact_root_path(destination, "destination")?;
    validate_root_relationship(&source, &destination, spec)?;
    let admin = admin_paths()?;
    validate_admin_outside_roots(&admin.dir, &source, &destination)?;
    validate_output_outside_admin(spec, &admin.dir)?;
    ensure_admin_dir(&admin.dir, !dry_run)?;
    let lock = if dry_run {
        acquire_migration_lock_readonly(&admin.lock)?
    } else {
        Some(acquire_migration_lock(&admin.lock)?)
    };
    let receipt_path = receipt_path(&admin.dir, &source, &destination);
    let existing = read_receipt_optional(&receipt_path)?;
    let is_new_migration = existing.is_none();
    let was_verified = existing
        .as_ref()
        .is_some_and(|receipt| receipt.state == ReceiptState::Verified);

    let source_exists = path_exists(&source)?;
    let destination_exists = path_exists(&destination)?;
    if source_exists && destination_exists {
        return Err(CliError::user(
            "conflicting_state_homes",
            format!(
                "both {} and {} exist; refusing to choose, merge, or overwrite",
                source.display(),
                destination.display()
            ),
        ));
    }

    let (source_device, validated, mut receipt) = if let Some(receipt) = existing {
        validate_receipt_identity(&receipt, &source, &destination)?;
        match receipt.state {
            ReceiptState::CanonicalWriteStarted => {
                return Err(CliError::user("migration_already_committed", "the canonical root has crossed its first-write boundary; migration is fix-forward only"));
            }
            ReceiptState::RollbackPrepared => {
                return Err(CliError::user("migration_rollback_in_progress", "rollback intent is durable; resume with the identical `state rollback` command"));
            }
            ReceiptState::RolledBack => {
                return Err(CliError::user("migration_rolled_back", "this migration receipt is rolled back; inspect the receipt before a new migration"));
            }
            ReceiptState::Prepared | ReceiptState::Renamed | ReceiptState::Verified => {}
        }
        let current_root = if destination_exists {
            &destination
        } else {
            &source
        };
        if device(current_root)? != receipt.source_device {
            return Err(CliError::user(
                "migration_device_changed",
                "current state root is not on the filesystem recorded by the receipt",
            ));
        }
        let validated = validate_quiescent_root(current_root)?;
        if validated.tree_sha256 != receipt.tree_sha256 {
            return Err(CliError::user(
                "migration_verification_failed",
                "state bytes differ from the durable migration receipt; refusing recovery",
            ));
        }
        (receipt.source_device, validated, receipt)
    } else {
        if !source_exists {
            return Err(CliError::user(
                "migration_source_not_found",
                format!("source root {} does not exist", source.display()),
            ));
        }
        if destination_exists {
            return Err(CliError::user(
                "migration_destination_exists",
                format!(
                    "destination {} already exists without a matching receipt",
                    destination.display()
                ),
            ));
        }
        let source_device = device(&source)?;
        let destination_device = device(
            destination
                .parent()
                .expect("normalized destination has parent"),
        )?;
        require_same_filesystem(source_device, destination_device)?;
        let validated = validate_quiescent_root(&source)?;
        let now = Utc::now();
        let receipt = Receipt {
            schema_version: RECEIPT_SCHEMA,
            source: source.clone(),
            destination: destination.clone(),
            source_device,
            tree_sha256: validated.tree_sha256.clone(),
            state: ReceiptState::Prepared,
            created_at: now,
            updated_at: now,
        };
        (source_device, validated, receipt)
    };
    let hash = validated.tree_sha256.clone();
    let run_count = validated.run_count;
    let _lock = lock;
    let destination_device = device(destination.parent().expect("parent"))?;
    require_same_filesystem(source_device, destination_device)?;
    if dry_run {
        return emit_payload(
            "migrate",
            true,
            &receipt_path,
            &receipt,
            run_count,
            true,
            !destination_exists,
            spec,
            warnings,
        );
    }

    if was_verified && !source_exists {
        // Idempotent verification of an already-complete move must never
        // transiently downgrade the durable state to `renamed`.
        return emit_payload(
            "migrate",
            false,
            &receipt_path,
            &receipt,
            run_count,
            true,
            false,
            spec,
            warnings,
        );
    }
    if is_new_migration {
        write_receipt(&receipt_path, &receipt)?;
    }
    if source_exists {
        if let Err(error) = rename_noreplace(&source, &destination) {
            if is_new_migration {
                remove_receipt(&receipt_path)?;
            }
            return Err(error);
        }
        sync_parent(&source)?;
        sync_parent(&destination)?;
    } else {
        // Recovery may observe `Prepared` after the rename but before either
        // parent fsync. Re-sync both before advancing the durable receipt.
        sync_parent(&source)?;
        sync_parent(&destination)?;
    }
    receipt.state = ReceiptState::Renamed;
    receipt.updated_at = Utc::now();
    write_receipt(&receipt_path, &receipt)?;
    let after_hash = tree_hash(&destination)?;
    if after_hash != hash {
        return Err(CliError::user(
            "migration_verification_failed",
            "destination bytes or run inventory differ after rename; refusing to advance receipt",
        ));
    }
    receipt.state = ReceiptState::Verified;
    receipt.updated_at = Utc::now();
    write_receipt(&receipt_path, &receipt)?;
    emit_payload(
        "migrate",
        false,
        &receipt_path,
        &receipt,
        run_count,
        true,
        false,
        spec,
        warnings,
    )
}

fn rollback(
    source: &Path,
    destination: &Path,
    dry_run: bool,
    spec: &OutputSpec,
    warnings: &[String],
) -> Result<(), CliError> {
    let source = exact_root_path(source, "source")?;
    let destination = exact_root_path(destination, "destination")?;
    validate_root_relationship(&source, &destination, spec)?;
    let admin = admin_paths()?;
    validate_admin_outside_roots(&admin.dir, &source, &destination)?;
    validate_output_outside_admin(spec, &admin.dir)?;
    ensure_admin_dir(&admin.dir, false)?;
    let _lock = if dry_run {
        acquire_migration_lock_readonly(&admin.lock)?
    } else {
        Some(acquire_migration_lock(&admin.lock)?)
    };
    let receipt_path = receipt_path(&admin.dir, &source, &destination);
    let mut receipt = read_receipt(&receipt_path)?;
    validate_receipt_identity(&receipt, &source, &destination)?;
    match receipt.state {
        ReceiptState::CanonicalWriteStarted => {
            return Err(CliError::user("rollback_forbidden_after_canonical_write", "rollback is permanently forbidden after the first canonical write; repair or fix forward in the canonical root"));
        }
        ReceiptState::Verified | ReceiptState::RollbackPrepared => {}
        ReceiptState::Prepared | ReceiptState::Renamed | ReceiptState::RolledBack => {
            return Err(CliError::user(
                "rollback_not_ready",
                format!(
                    "rollback requires a verified or rollback-prepared receipt, found {:?}",
                    receipt.state
                ),
            ));
        }
    }
    let source_exists = path_exists(&source)?;
    let destination_exists = path_exists(&destination)?;
    if source_exists && destination_exists {
        return Err(CliError::user(
            "conflicting_state_homes",
            "both rollback roots exist; refusing split truth",
        ));
    }
    if !source_exists && !destination_exists {
        return Err(CliError::user(
            "migration_roots_missing",
            "neither rollback root exists; manual recovery is required",
        ));
    }
    let current_root = if destination_exists {
        &destination
    } else {
        &source
    };
    if device(current_root)? != receipt.source_device
        || device(source.parent().expect("normalized source has parent"))? != receipt.source_device
    {
        return Err(CliError::user(
            "migration_device_changed",
            "rollback root/target is not on the filesystem recorded by the receipt",
        ));
    }
    let validated = validate_quiescent_root(current_root)?;
    if validated.tree_sha256 != receipt.tree_sha256 {
        return Err(CliError::user(
            "migration_verification_failed",
            "canonical bytes differ from the receipt; rollback is forbidden",
        ));
    }
    let run_count = validated.run_count;
    if dry_run {
        return emit_payload(
            "rollback",
            true,
            &receipt_path,
            &receipt,
            run_count,
            true,
            !source_exists,
            spec,
            warnings,
        );
    }
    if receipt.state == ReceiptState::Verified {
        receipt.state = ReceiptState::RollbackPrepared;
        receipt.updated_at = Utc::now();
        write_receipt(&receipt_path, &receipt)?;
    }
    if destination_exists {
        rename_noreplace(&destination, &source)?;
    }
    // Recovery from RollbackPrepared + source-present repeats both parent
    // fsyncs before completing the receipt.
    sync_parent(&source)?;
    sync_parent(&destination)?;
    receipt.state = ReceiptState::RolledBack;
    receipt.updated_at = Utc::now();
    write_receipt(&receipt_path, &receipt)?;
    emit_payload(
        "rollback",
        false,
        &receipt_path,
        &receipt,
        run_count,
        true,
        false,
        spec,
        warnings,
    )
}

// The envelope deliberately keeps each independently useful verification fact
// explicit for machine callers; bundling call-site context would obscure it.
#[allow(clippy::too_many_arguments)]
fn emit_payload(
    operation: &'static str,
    dry_run: bool,
    receipt_path: &Path,
    receipt: &Receipt,
    run_count: usize,
    same_filesystem: bool,
    destination_absent: bool,
    spec: &OutputSpec,
    warnings: &[String],
) -> Result<(), CliError> {
    let would = if dry_run {
        vec![PlannedMutation {
            action: "rename",
            resource: "state_root",
            from: if operation == "rollback" {
                receipt.destination.clone()
            } else {
                receipt.source.clone()
            },
            to: if operation == "rollback" {
                receipt.source.clone()
            } else {
                receipt.destination.clone()
            },
            known_effects: serde_json::json!({
                "atomic_whole_root_rename": true,
                "receipt_transition": if operation == "rollback" { "rolled_back" } else { "verified" }
            }),
            unknown_until_apply: vec!["rename_and_fsync_io_outcome"],
        }]
    } else {
        Vec::new()
    };
    let payload = MigrationPayload {
        operation, dry_run, would, source: receipt.source.clone(), destination: receipt.destination.clone(), receipt: receipt_path.to_path_buf(), state: receipt.state.clone(), tree_sha256: receipt.tree_sha256.clone(), run_count,
        checks: MigrationChecks { same_filesystem, destination_absent, atomic_whole_root_rename: true },
        first_canonical_write: "the durable canonical-write-started receipt transition immediately before the first attempted event append, projection/config/skill/supervisor metadata write, or canonical log creation",
        rollback_allowed: receipt.state == ReceiptState::Verified,
        operator_exclusion: EXCLUSION,
    };
    if spec.format == OutputFormat::Text {
        println!("operation: {}", payload.operation);
        println!("dry_run: {}", payload.dry_run);
        println!("source: {}", payload.source.display());
        println!("destination: {}", payload.destination.display());
        println!("receipt: {}", payload.receipt.display());
        println!("state: {:?}", payload.state);
        println!("tree_sha256: {}", payload.tree_sha256);
        println!("run_count: {}", payload.run_count);
        println!("same_filesystem: {}", payload.checks.same_filesystem);
        println!("destination_absent: {}", payload.checks.destination_absent);
        println!(
            "atomic_whole_root_rename: {}",
            payload.checks.atomic_whole_root_rename
        );
        for mutation in &payload.would {
            println!(
                "would: {} {} {} -> {}",
                mutation.action,
                mutation.resource,
                mutation.from.display(),
                mutation.to.display()
            );
        }
        println!("first_canonical_write: {}", payload.first_canonical_write);
        println!("rollback_allowed: {}", payload.rollback_allowed);
        println!("operator_exclusion: {}", payload.operator_exclusion);
        output::emit_text_warnings(warnings);
        Ok(())
    } else {
        output::emit_envelope(&payload, spec, warnings)
    }
}

struct ValidatedRoot {
    tree_sha256: String,
    run_count: usize,
    _run_guards: Vec<RunLock>,
}

struct AdminPaths {
    dir: PathBuf,
    lock: PathBuf,
}
fn admin_paths() -> Result<AdminPaths, CliError> {
    let home = match std::env::var_os("HOME").filter(|home| !home.is_empty()) {
        Some(home) => PathBuf::from(home),
        None => account_home()?,
    };
    let home = home
        .canonicalize()
        .map_err(|e| io("resolve HOME", &home, e))?;
    let dir = home.join(ADMIN_DIR);
    Ok(AdminPaths {
        lock: dir.join(GLOBAL_LOCK),
        dir,
    })
}
fn ensure_admin_dir(path: &Path, create: bool) -> Result<(), CliError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(CliError::user(
                "invalid_migration_admin",
                format!(
                    "migration admin path {} must be a real directory, not a symlink",
                    path.display()
                ),
            ));
        }
        Ok(_) => {
            let canonical = path
                .canonicalize()
                .map_err(|e| io("normalize migration admin directory", path, e))?;
            if canonical != path {
                return Err(CliError::user(
                    "invalid_migration_admin",
                    format!(
                        "migration admin path {} resolves to {}",
                        path.display(),
                        canonical.display()
                    ),
                ));
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound && create => {
            match std::fs::create_dir(path) {
                Ok(()) => sync_parent(path)?,
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                    // A concurrent ordinary command won the first-use race.
                    ensure_admin_dir(path, false)?;
                }
                Err(error) => return Err(io("create migration admin directory", path, error)),
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io("inspect migration admin directory", path, error)),
    }
    Ok(())
}

#[cfg(unix)]
fn account_home() -> Result<PathBuf, CliError> {
    use std::ffi::CStr;
    let mut record = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut result = std::ptr::null_mut();
    let mut buffer = vec![0u8; 16 * 1024];
    let rc = unsafe {
        libc::getpwuid_r(
            libc::geteuid(),
            record.as_mut_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &raw mut result,
        )
    };
    if rc != 0 || result.is_null() {
        return Err(CliError::system(
            "home_not_set",
            "HOME is unset and the current Unix account has no home directory",
        ));
    }
    let record = unsafe { record.assume_init() };
    let bytes = unsafe { CStr::from_ptr(record.pw_dir) }.to_bytes();
    use std::os::unix::ffi::OsStrExt;
    Ok(PathBuf::from(std::ffi::OsStr::from_bytes(bytes)))
}

#[cfg(not(unix))]
fn account_home() -> Result<PathBuf, CliError> {
    Err(CliError::system(
        "home_not_set",
        "HOME is required for the external migration fence and receipt",
    ))
}

fn open_lock(path: &Path) -> Result<File, CliError> {
    let mut opts = OpenOptions::new();
    opts.create(true).read(true).write(true).truncate(false);
    octl_core::nofollow(&mut opts);
    opts.open(path)
        .map_err(|e| io("open migration lock", path, e))
}
fn acquire_migration_lock_readonly(path: &Path) -> Result<Option<File>, CliError> {
    let mut opts = OpenOptions::new();
    opts.read(true).write(true).truncate(false);
    octl_core::nofollow(&mut opts);
    match opts.open(path) {
        Ok(file) => try_migration_lock(file, path).map(Some),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io("open migration lock", path, e)),
    }
}
fn acquire_migration_lock(path: &Path) -> Result<File, CliError> {
    try_migration_lock(open_lock(path)?, path)
}
fn try_migration_lock(file: File, path: &Path) -> Result<File, CliError> {
    match <File as FileExt>::try_lock(&file) {
        Ok(()) => Ok(file),
        Err(fs4::TryLockError::WouldBlock) => Err(CliError::user(
            "migration_lock_held",
            "another Taskfleet command or migration holds the external migration lock",
        )),
        Err(fs4::TryLockError::Error(error)) => Err(io("acquire migration lock", path, error)),
    }
}
fn receipt_path(dir: &Path, source: &Path, destination: &Path) -> PathBuf {
    let mut h = Sha256::new();
    h.update(source.as_os_str().as_encoded_bytes());
    h.update([0]);
    h.update(destination.as_os_str().as_encoded_bytes());
    dir.join(format!("receipt-{:x}.json", h.finalize()))
}

fn validate_root_relationship(
    source: &Path,
    destination: &Path,
    spec: &OutputSpec,
) -> Result<(), CliError> {
    if source == destination || source.starts_with(destination) || destination.starts_with(source) {
        return Err(CliError::user(
            "invalid_migration_roots",
            "source and destination must be distinct, non-nested roots",
        ));
    }
    if let Some(output) = &spec.file {
        let absolute = normalize_output_path(output)?;
        if absolute.starts_with(source) || absolute.starts_with(destination) {
            return Err(CliError::user(
                "migration_output_inside_state_root",
                "migration output files must remain outside source and destination roots",
            )
            .with_invalid_value(output.display().to_string()));
        }
    }
    Ok(())
}

fn validate_output_outside_admin(spec: &OutputSpec, admin: &Path) -> Result<(), CliError> {
    if let Some(output) = &spec.file {
        let absolute = normalize_output_path(output)?;
        if absolute.starts_with(admin) {
            return Err(CliError::user(
                "migration_output_inside_admin",
                "migration output files must remain outside the receipt/lock directory",
            )
            .with_invalid_value(output.display().to_string()));
        }
    }
    Ok(())
}

fn normalize_output_path(path: &Path) -> Result<PathBuf, CliError> {
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(CliError::user(
            "invalid_output_path",
            "migration output path must not contain `..`; provide its normalized spelling",
        )
        .with_invalid_value(path.display().to_string()));
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| io("resolve output path", path, e))?
            .join(path)
    };
    if let Ok(canonical) = absolute.canonicalize() {
        return Ok(canonical);
    }
    let mut lexical = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                lexical.pop();
            }
            other => lexical.push(other.as_os_str()),
        }
    }
    let mut missing = Vec::new();
    let mut ancestor = lexical.as_path();
    loop {
        match ancestor.canonicalize() {
            Ok(mut base) => {
                for component in missing.iter().rev() {
                    base.push(component);
                }
                return Ok(base);
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                let name = ancestor.file_name().ok_or_else(|| {
                    CliError::user("invalid_output_path", "output path cannot be normalized")
                })?;
                missing.push(name.to_os_string());
                ancestor = ancestor.parent().ok_or_else(|| {
                    CliError::user("invalid_output_path", "output path has no parent")
                })?;
            }
            Err(error) => return Err(io("normalize output path", &absolute, error)),
        }
    }
}

fn validate_admin_outside_roots(
    admin: &Path,
    source: &Path,
    destination: &Path,
) -> Result<(), CliError> {
    if admin.starts_with(source)
        || admin.starts_with(destination)
        || source.starts_with(admin)
        || destination.starts_with(admin)
    {
        return Err(CliError::user(
            "migration_admin_inside_state_root",
            format!(
                "external migration lock/receipt directory {} must be outside both roots",
                admin.display()
            ),
        ));
    }
    Ok(())
}

fn exact_root_path(path: &Path, field: &str) -> Result<PathBuf, CliError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => exact_existing_root(path, field),
        Err(e) if e.kind() == ErrorKind::NotFound => exact_missing_root(path, field),
        Err(e) => Err(io("inspect migration root", path, e)),
    }
}
fn exact_existing_root(path: &Path, field: &str) -> Result<PathBuf, CliError> {
    if !path.is_absolute() {
        return Err(CliError::user(
            "invalid_migration_path",
            format!("--{field} must be an absolute normalized path"),
        )
        .with_invalid_value(path.display().to_string()));
    }
    let meta =
        std::fs::symlink_metadata(path).map_err(|e| io("inspect migration root", path, e))?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(CliError::user(
            "invalid_migration_path",
            format!("--{field} must name a real directory, not a symlink or non-directory"),
        )
        .with_invalid_value(path.display().to_string()));
    }
    let normalized = path
        .canonicalize()
        .map_err(|e| io("normalize migration root", path, e))?;
    if normalized != path {
        return Err(CliError::user(
            "non_normalized_migration_path",
            format!(
                "--{field} must exactly equal its normalized path {}",
                normalized.display()
            ),
        )
        .with_invalid_value(path.display().to_string()));
    }
    Ok(normalized)
}
fn exact_missing_root(path: &Path, field: &str) -> Result<PathBuf, CliError> {
    if !path.is_absolute() {
        return Err(CliError::user(
            "invalid_migration_path",
            format!("--{field} must be an absolute normalized path"),
        )
        .with_invalid_value(path.display().to_string()));
    }
    match std::fs::symlink_metadata(path) {
        Ok(_) => return exact_existing_root(path, field),
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(io("inspect migration root", path, e)),
    }
    let parent = path.parent().ok_or_else(|| {
        CliError::user("invalid_migration_path", format!("--{field} has no parent"))
    })?;
    let normalized_parent = parent
        .canonicalize()
        .map_err(|e| io("normalize migration parent", parent, e))?;
    let normalized = normalized_parent.join(path.file_name().ok_or_else(|| {
        CliError::user(
            "invalid_migration_path",
            format!("--{field} has no final component"),
        )
    })?);
    if normalized != path {
        return Err(CliError::user(
            "non_normalized_migration_path",
            format!(
                "--{field} must exactly equal its normalized path {}",
                normalized.display()
            ),
        )
        .with_invalid_value(path.display().to_string()));
    }
    Ok(normalized)
}
fn path_exists(path: &Path) -> Result<bool, CliError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
        Err(e) => Err(io("inspect migration root", path, e)),
    }
}

fn validate_quiescent_root(root: &Path) -> Result<ValidatedRoot, CliError> {
    let runs = root.join("runs");
    let entries = match std::fs::read_dir(&runs) {
        Ok(v) => v,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return tree_hash(root).map(|tree_sha256| ValidatedRoot {
                tree_sha256,
                run_count: 0,
                _run_guards: Vec::new(),
            });
        }
        Err(e) => return Err(io("read runs directory", &runs, e)),
    };
    let mut run_dirs = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| io("read runs directory", &runs, e))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| CliError::user("corrupt_state", "run directory name is not UTF-8"))?;
        let id =
            RunId::parse_str(&name).map_err(|e| CliError::user("corrupt_state", e.to_string()))?;
        if !entry
            .file_type()
            .map_err(|e| io("inspect run directory", &entry.path(), e))?
            .is_dir()
        {
            return Err(CliError::user(
                "corrupt_state",
                format!("run path {} is not a directory", entry.path().display()),
            ));
        }
        run_dirs.push((id, entry.path()));
    }
    run_dirs.sort_by(|a, b| a.0.cmp(&b.0));
    let mut guards = Vec::new();
    for (id, dir) in &run_dirs {
        let paths = RunPaths::from_validated(dir.clone(), id.clone()).map_err(from_core)?;
        match std::fs::symlink_metadata(paths.lock()) {
            Ok(metadata) if metadata.is_file() => {
                match RunLock::try_acquire_existing(&paths.lock()).map_err(from_core)? {
                    Some(guard) => guards.push(guard),
                    None => {
                        return Err(CliError::user(
                            "run_lock_held",
                            format!(
                                "run {id} is being read or written; migration requires quiescence"
                            ),
                        ));
                    }
                }
            }
            Ok(_) => {
                return Err(CliError::user(
                    "corrupt_state",
                    format!("run {id} lock is not a regular file"),
                ));
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                // 0.5.1 fixtures can legitimately lack a lock. This cannot
                // fence a future old writer, so operator exclusion remains a
                // mandatory precondition; every present lock is retained.
            }
            Err(error) => return Err(io("inspect run lock", &paths.lock(), error)),
        }
        let manifest = read_manifest(&paths).map_err(from_core)?;
        if !manifest.status.is_terminal() {
            return Err(CliError::user(
                "nonterminal_run",
                format!(
                    "run {} is {:?}; every run must be terminal",
                    id, manifest.status
                ),
            ));
        }
        match crate::supervise::pid_file::classify_pid_record(&paths.supervisor_pid()) {
            crate::supervise::pid_file::PidRecord::Present { pid, start_time }
                if crate::supervise::pid_file::pid_live_with_identity(pid, start_time) =>
            {
                return Err(CliError::user(
                    "live_supervisor",
                    format!("run {id} has live supervisor pid {pid}"),
                ))
            }
            crate::supervise::pid_file::PidRecord::Unreadable => {
                return Err(CliError::user(
                    "unverifiable_supervisor",
                    format!("run {id} supervisor pid record is unreadable"),
                ))
            }
            _ => {}
        }
        let events = read_all_events(&paths.events()).map_err(from_core)?;
        for event in &events {
            octl_core::validate_event(&paths, event).map_err(from_core)?;
        }
        if events.last().map_or(0, |e| e.seq) != manifest.applied_seq {
            return Err(CliError::user(
                "unapplied_events",
                format!(
                    "run {} applied_seq {} does not equal final event sequence {}",
                    id,
                    manifest.applied_seq,
                    events.last().map_or(0, |e| e.seq)
                ),
            ));
        }
        let nodes = match std::fs::read_dir(paths.nodes_dir()) {
            Ok(nodes) => Some(nodes),
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(error) => return Err(io("read node projections", &paths.nodes_dir(), error)),
        };
        for entry in nodes.into_iter().flatten() {
            let entry = entry.map_err(|e| io("read node projections", &paths.nodes_dir(), e))?;
            if entry.path().extension().and_then(|v| v.to_str()) != Some("json") {
                return Err(CliError::user(
                    "corrupt_state",
                    format!("unexpected node entry {}", entry.path().display()),
                ));
            }
            let stem = entry
                .path()
                .file_stem()
                .and_then(|v| v.to_str())
                .ok_or_else(|| CliError::user("corrupt_state", "invalid node filename"))?
                .to_string();
            let node_id = octl_core::NodeId::parse_str(&stem)
                .map_err(|e| CliError::user("corrupt_state", e.to_string()))?;
            let node = read_node(&paths, &node_id).map_err(from_core)?;
            if !node.status.is_terminal() {
                return Err(CliError::user(
                    "nonterminal_node",
                    format!("run {} node {} is {:?}", id, node_id, node.status),
                ));
            }
            if node.pending_merge.is_some() {
                return Err(CliError::user(
                    "pending_merge",
                    format!("run {id} node {node_id} has a pending merge transaction"),
                ));
            }
            if let Some(raw_pid) = node.agent_pid {
                let pid = u32::try_from(raw_pid).map_err(|_| {
                    CliError::user(
                        "unverifiable_worker",
                        format!("run {id} node {node_id} has invalid worker pid {raw_pid}"),
                    )
                })?;
                if process_identity_live(pid, node.agent_pid_start_time.as_ref()) {
                    return Err(CliError::user(
                        "live_worker",
                        format!("run {id} node {node_id} has live worker pid {pid}"),
                    ));
                }
            }
            // `Node::supervisor_pid` is legacy projection metadata with no
            // process-start identity and may be stale/recycled. The
            // identity-bearing `supervisor.pid` record checked above is the
            // authoritative live-supervisor proof.
        }
    }
    let tree_sha256 = tree_hash(root)?;
    Ok(ValidatedRoot {
        tree_sha256,
        run_count: run_dirs.len(),
        _run_guards: guards,
    })
}
fn process_identity_live(pid: u32, expected: Option<&chrono::DateTime<Utc>>) -> bool {
    if !crate::supervise::pid_file::pid_alive(pid) {
        return false;
    }
    match expected {
        Some(t) => crate::supervise::watchdog::pid_start_time(pid)
            .is_none_or(|actual| actual.abs_diff(t.timestamp().max(0) as u64) <= 1),
        None => true,
    }
}

fn tree_hash(root: &Path) -> Result<String, CliError> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort();
    let mut h = Sha256::new();
    for rel in files {
        h.update(rel.as_os_str().as_encoded_bytes());
        h.update([0]);
        let bytes = std::fs::read(root.join(&rel))
            .map_err(|e| io("hash state file", &root.join(&rel), e))?;
        h.update((bytes.len() as u64).to_le_bytes());
        h.update(bytes);
    }
    Ok(format!("{:x}", h.finalize()))
}
fn collect_files(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), CliError> {
    for entry in std::fs::read_dir(dir).map_err(|e| io("walk state root", dir, e))? {
        let entry = entry.map_err(|e| io("walk state root", dir, e))?;
        let ty = entry
            .file_type()
            .map_err(|e| io("inspect state entry", &entry.path(), e))?;
        if ty.is_symlink() {
            return Err(CliError::user(
                "symlink_in_state_root",
                format!(
                    "state entry {} is a symlink; migration refuses aliases",
                    entry.path().display()
                ),
            ));
        }
        if ty.is_dir() {
            collect_files(root, &entry.path(), out)?;
        } else if ty.is_file() {
            out.push(
                entry
                    .path()
                    .strip_prefix(root)
                    .expect("walked below root")
                    .to_path_buf(),
            );
        } else {
            return Err(CliError::user(
                "unsupported_state_entry",
                format!(
                    "state entry {} is not a regular file or directory",
                    entry.path().display()
                ),
            ));
        }
    }
    Ok(())
}

fn require_same_filesystem(source_device: u64, destination_device: u64) -> Result<(), CliError> {
    if source_device == destination_device {
        Ok(())
    } else {
        Err(CliError::user(
            "cross_device_migration_unsupported",
            "source and destination parent are on different filesystems; only atomic same-filesystem rename is supported",
        ))
    }
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
fn rename_noreplace(from: &Path, to: &Path) -> Result<(), CliError> {
    use std::os::unix::ffi::OsStrExt;
    let from_c = std::ffi::CString::new(from.as_os_str().as_bytes())
        .map_err(|_| CliError::user("invalid_migration_path", "source path contains a NUL byte"))?;
    let to_c = std::ffi::CString::new(to.as_os_str().as_bytes()).map_err(|_| {
        CliError::user(
            "invalid_migration_path",
            "destination path contains a NUL byte",
        )
    })?;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let rc = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from_c.as_ptr(),
            libc::AT_FDCWD,
            to_c.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    #[cfg(target_os = "macos")]
    let rc = unsafe { libc::renamex_np(from_c.as_ptr(), to_c.as_ptr(), libc::RENAME_EXCL) };
    if rc == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == ErrorKind::AlreadyExists {
        return Err(CliError::user(
            "migration_destination_exists",
            format!(
                "rename target {} appeared before the atomic move; nothing was replaced",
                to.display()
            ),
        ));
    }
    if error.raw_os_error() == Some(libc::EXDEV) {
        return Err(CliError::user(
            "cross_device_migration_unsupported",
            "atomic no-replace rename reported a cross-filesystem move",
        ));
    }
    if error
        .raw_os_error()
        .is_some_and(|code| [libc::ENOSYS, libc::EINVAL, libc::ENOTSUP].contains(&code))
    {
        return Err(CliError::user(
            "migration_atomic_rename_unsupported",
            format!(
                "filesystem/kernel does not support atomic no-replace directory rename: {error}"
            ),
        ));
    }
    Err(io(
        "atomically rename state root without replacement",
        to,
        error,
    ))
}

#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
fn rename_noreplace(_from: &Path, _to: &Path) -> Result<(), CliError> {
    Err(CliError::system(
        "migration_platform_unsupported",
        "atomic no-replace directory rename is unavailable on this platform",
    ))
}

#[cfg(unix)]
fn device(path: &Path) -> Result<u64, CliError> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path)
        .map(|m| m.dev())
        .map_err(|e| io("read filesystem identity", path, e))
}
#[cfg(not(unix))]
fn device(_path: &Path) -> Result<u64, CliError> {
    Err(CliError::system(
        "migration_platform_unsupported",
        "state migration currently requires a Unix filesystem",
    ))
}
fn sync_parent(path: &Path) -> Result<(), CliError> {
    let p = path
        .parent()
        .ok_or_else(|| CliError::system("io_error", "migration path has no parent"))?;
    File::open(p)
        .and_then(|f| f.sync_all())
        .map_err(|e| io("fsync migration parent", p, e))
}
fn read_receipt_optional(path: &Path) -> Result<Option<Receipt>, CliError> {
    let mut options = OpenOptions::new();
    options.read(true);
    octl_core::nofollow(&mut options);
    match options.open(path).and_then(|mut file| {
        use std::io::Read as _;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(bytes)
    }) {
        Ok(bytes) => parse_receipt(path, &bytes).map(Some),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io("read migration receipt", path, e)),
    }
}
fn read_receipt(path: &Path) -> Result<Receipt, CliError> {
    read_receipt_optional(path)?.ok_or_else(|| {
        CliError::user(
            "migration_receipt_not_found",
            format!("no migration receipt at {}", path.display()),
        )
    })
}
fn parse_receipt(path: &Path, b: &[u8]) -> Result<Receipt, CliError> {
    let r: Receipt = serde_json::from_slice(b).map_err(|e| {
        CliError::user(
            "corrupt_migration_receipt",
            format!("parse {}: {}", path.display(), e),
        )
    })?;
    if r.schema_version != RECEIPT_SCHEMA {
        return Err(CliError::user(
            "unsupported_migration_receipt",
            format!("receipt schema {} is unsupported", r.schema_version),
        ));
    }
    Ok(r)
}
fn validate_receipt_identity(r: &Receipt, s: &Path, d: &Path) -> Result<(), CliError> {
    if r.source != s || r.destination != d {
        return Err(CliError::user(
            "migration_receipt_conflict",
            "receipt paths do not match the requested exact roots",
        ));
    }
    Ok(())
}
fn remove_receipt(path: &Path) -> Result<(), CliError> {
    std::fs::remove_file(path).map_err(|e| io("remove uncommitted migration receipt", path, e))?;
    sync_parent(path)
}

fn write_receipt(path: &Path, r: &Receipt) -> Result<(), CliError> {
    let bytes = serde_json::to_vec_pretty(r)
        .map_err(|e| CliError::system("receipt_serialize_failed", e.to_string()))?;
    let tmp = path.with_extension(format!("json.tmp.{}", octl_core::new_op_id()));
    let mut o = OpenOptions::new();
    o.create_new(true).write(true);
    octl_core::nofollow(&mut o);
    let mut f = o
        .open(&tmp)
        .map_err(|e| io("create receipt temp file", &tmp, e))?;
    f.write_all(&bytes)
        .and_then(|()| f.sync_all())
        .map_err(|e| io("write migration receipt", &tmp, e))?;
    std::fs::rename(&tmp, path).map_err(|e| io("publish migration receipt", path, e))?;
    sync_parent(path)
}
fn io(action: &str, path: &Path, e: std::io::Error) -> CliError {
    CliError::system(
        "migration_io_error",
        format!("{action} {}: {e}", path.display()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn different_devices_are_a_typed_refusal() {
        let error = require_same_filesystem(1, 2).unwrap_err();
        assert_eq!(error.code, "cross_device_migration_unsupported");
    }

    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
    #[test]
    fn no_replace_rename_never_replaces_an_existing_directory() {
        let temp = tempfile::TempDir::new().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&destination).unwrap();
        let error = rename_noreplace(&source, &destination).unwrap_err();
        assert_eq!(error.code, "migration_destination_exists");
        assert!(source.is_dir());
        assert!(destination.is_dir());
    }
}
