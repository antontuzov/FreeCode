//! Fetching and unpacking the pinned fork engine — the only code in the app that writes to
//! `engines/`.
//!
//! The archive comes down through the same resume + sha256 discipline as the weights
//! ([`freecode_models::download`]), is unpacked into a sibling staging directory, and is renamed into
//! place only once the server binary has been found inside it. That ordering is the point:
//!
//!   * a killed transfer leaves a `.part` under `.parts/`, which nothing can mistake for an install;
//!   * a killed unpack leaves `.staging/`, so `EngineBinary::installed` — which looks for
//!     `llama-server` under `engines/<tag>/<key>/` — can never report a half-written tree as ready;
//!   * a substituted release asset fails the sha256 recorded in `engines/lock.toml` and never
//!     reaches the loader at all.
//!
//! Nothing here chooses *which* engine to install: [`EngineBinary`] already refused SYCL, already
//! preferred the recommended CUDA build, and already rejected a binary outside `engines/`. The
//! installer takes that resolution as given.

use std::path::{Path, PathBuf};

use serde::Serialize;

use freecode_models::download::{self, DownloadPlan};
use freecode_models::lock::{EngineLock, EngineTarget};

use crate::engine::{engines_root, find_server, EngineBinary};
use crate::LlamaError;

/// Where the downloaded archive waits while it is unpacked. Dot-prefixed, and outside the
/// `<key>` directory the resolver searches, so it cannot be mistaken for an installed engine.
pub const PARTS_DIR: &str = ".parts";
/// Unpacking lands here first; the directory is renamed into place in one step.
pub const STAGING_DIR: &str = ".staging";

/// `engines/<tag>`, the directory holding one subdirectory per installed target.
pub fn tag_dir(root: &Path, lock: &EngineLock) -> PathBuf {
    engines_root(root).join(&lock.pinned_tag)
}

/// The plan for this target's release asset.
///
/// `token_env` is deliberately `None`: a GitHub release asset is public, and an HF read-only token
/// must never be sent to a host the catalog did not name.
pub fn plan_for(lock: &EngineLock, root: &Path, target: &EngineTarget) -> DownloadPlan {
    DownloadPlan {
        url: lock.download_url(target),
        dest: tag_dir(root, lock).join(PARTS_DIR).join(&target.asset),
        expected_size_bytes: target.size_bytes,
        expected_sha256: target.sha256.clone(),
        token_env: None,
    }
}

/// Create the two scratch directories an install writes into, under `engines/<tag>/`.
///
/// Both are dot-prefixed siblings of the `<key>` directories the resolver scans, so a run that dies
/// halfway leaves nothing an install could be counted from.
pub fn prepare_scratch(root: &Path, lock: &EngineLock) -> Result<PathBuf, LlamaError> {
    let tag = tag_dir(root, lock);
    let parts = tag.join(PARTS_DIR);
    let staging = tag.join(STAGING_DIR);
    std::fs::create_dir_all(&parts).or_err(&parts)?;
    std::fs::create_dir_all(&staging).or_err(&staging)?;
    Ok(parts)
}

/// What the UI is told while an install runs. Progress is reported in bytes rather than a
/// percentage, because the total is the lock's `size_bytes` and the user can check both.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "phase")]
pub enum Phase {
    #[serde(rename_all = "camelCase")]
    Downloading {
        transferred_bytes: u64,
        total_bytes: u64,
    },
    Unpacking,
    Done {
        installed: Installed,
    },
}

/// An engine that is on disk and where the resolver expects it. Not proof that it *runs* — that is
/// the banner check, which needs a subprocess and so belongs to the supervisor.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
    pub tag: String,
    pub target_key: String,
    pub asset: String,
    pub sha256: String,
    pub dir: PathBuf,
    pub server_path: PathBuf,
}

/// Download the pinned asset and unpack it. `client` is passed in so the caller owns the
/// connection pool and the user-agent.
pub async fn install(
    lock: &EngineLock,
    root: &Path,
    engine: &EngineBinary,
    client: &reqwest::Client,
    on_phase: &(dyn Fn(Phase) + Send + Sync),
) -> Result<Installed, LlamaError> {
    EngineBinary::assert_tag_allowed(lock)?;
    let target = engine.target.clone();
    let plan = plan_for(lock, root, &target);

    // The first install on a machine finds `engines/` holding nothing but `lock.toml`, and
    // `download` creates its destination only once the transfer is done — the part file it streams
    // into needs its directory first.
    prepare_scratch(root, lock)?;

    let status = download::download(client, &plan, |p| {
        on_phase(Phase::Downloading {
            transferred_bytes: p.transferred_bytes,
            total_bytes: p.total_bytes,
        })
    })
    .await
    .map_err(LlamaError::Download)?;
    // `download` renames the part file only after the streaming sha256 matched, so a plan whose
    // destination exists is already verified. Anything else is a bug in that contract, not a
    // judgement call for us to make here.
    if !status.is_ok() {
        return Err(LlamaError::ArchiveUnverified {
            path: plan.dest,
            detail: status.describe(),
        });
    }

    on_phase(Phase::Unpacking);
    let root = root.to_path_buf();
    let lock = lock.clone();
    let archive = plan.dest.clone();
    let asset = target.asset.clone();
    // Unpacking a 340 MB CUDA bundle is CPU and IO work; it must not block the command runtime's
    // other tasks.
    let installed = tokio::task::spawn_blocking(move || unpack(&lock, &root, &target, &archive))
        .await
        .map_err(|source| LlamaError::InstallInterrupted {
            asset,
            reason: source.to_string(),
        })??;
    on_phase(Phase::Done {
        installed: installed.clone(),
    });
    Ok(installed)
}

/// Unpack `archive` into `engines/<tag>/<key>`, in place and atomically.
///
/// Blocking; call it through [`tokio::task::spawn_blocking`].
pub fn unpack(
    lock: &EngineLock,
    root: &Path,
    target: &EngineTarget,
    archive: &Path,
) -> Result<Installed, LlamaError> {
    let tag = tag_dir(root, lock);
    let staging = tag.join(STAGING_DIR).join(&target.key);
    let final_dir = lock.install_dir(&engines_root(root), target);

    // Scratch from an earlier attempt is not state worth preserving.
    remove_tree(&staging)?;
    std::fs::create_dir_all(&staging).or_err(&staging)?;

    if target.asset.ends_with(".tar.gz") {
        unpack_tar_gz(archive, &staging, &target.asset)?;
    } else if target.asset.ends_with(".zip") {
        unpack_zip(archive, &staging, &target.asset)?;
    } else {
        return Err(LlamaError::UnknownArchiveFormat {
            asset: target.asset.clone(),
        });
    }

    let (content, server) = content_root(&staging, lock.server_binary(), &target.asset)?;
    let relative = server
        .strip_prefix(&content)
        .unwrap_or_else(|_| Path::new(lock.server_binary()))
        .to_path_buf();
    for name in lock.binaries.keys() {
        if let Some(bin) = find_server(&content, name) {
            make_executable(&bin, &target.asset)?;
        }
    }

    swap_into_place(&final_dir, &content, &tag, &target.key)?;
    // The archive has served its purpose; keeping 340 MB of it per target would be rude to the disk
    // the weights also need.
    let _ = std::fs::remove_file(archive);
    let _ = std::fs::remove_dir_all(tag.join(STAGING_DIR));

    Ok(Installed {
        tag: lock.pinned_tag.clone(),
        target_key: target.key.clone(),
        asset: target.asset.clone(),
        sha256: target.sha256.clone(),
        dir: final_dir.clone(),
        server_path: final_dir.join(relative),
    })
}

/// The unpacked directory that actually holds the engine.
///
/// The two archive formats the releases use are laid out differently, which matters because
/// [`crate::engine`] looks for the server directly under `engines/<tag>/<key>/`. Read from the
/// pinned release on 2026-10-13:
///   * `…-bin-macos-arm64.tar.gz` wraps everything in one `llama-prism-<tag>/` directory;
///   * `…-bin-win-cpu-arm64.zip` is flat, with `llama-server.exe` beside the DLLs.
///
/// So a single wrapping directory is unwrapped, and anything else is refused rather than guessed at
/// — an engine we cannot point a launcher at is worse than no engine. Returns the engine root
/// together with the server binary found inside it.
fn content_root(
    staging: &Path,
    server_binary: &str,
    asset: &str,
) -> Result<(PathBuf, PathBuf), LlamaError> {
    let missing = || LlamaError::ServerBinaryMissing {
        asset: asset.to_string(),
        dir: staging.to_path_buf(),
        binary: server_binary.to_string(),
    };
    if let Some(server) = find_server(staging, server_binary) {
        return Ok((staging.to_path_buf(), server));
    }
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut loose_files = 0usize;
    for entry in std::fs::read_dir(staging).map_err(|source| LlamaError::WriteEngine {
        path: staging.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| LlamaError::WriteEngine {
            path: staging.to_path_buf(),
            source,
        })?;
        if entry
            .file_type()
            .map_err(|source| LlamaError::WriteEngine {
                path: entry.path(),
                source,
            })?
            .is_dir()
        {
            dirs.push(entry.path());
        } else {
            loose_files += 1;
        }
    }
    let wrapper = dirs.first().filter(|_| dirs.len() == 1 && loose_files == 0);
    match wrapper.and_then(|w| find_server(w, server_binary).map(|s| (w.clone(), s))) {
        Some(found) => Ok(found),
        None => Err(missing()),
    }
}

/// Entry paths are sanitised one at a time with `Entry::unpack_in`, which refuses to write outside
/// the destination. It reports a skipped entry as `Ok(false)` rather than an error; for an engine
/// archive a skipped entry means a missing file that would only surface at launch, so it is turned
/// into a refusal here.
fn unpack_tar_gz(archive: &Path, into: &Path, asset: &str) -> Result<(), LlamaError> {
    let file = std::fs::File::open(archive).map_err(|source| LlamaError::ReadArchive {
        asset: asset.to_string(),
        source,
    })?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(decoder);
    let entries = tar.entries().map_err(|source| LlamaError::UnpackTar {
        asset: asset.to_string(),
        source,
    })?;
    for entry in entries {
        let mut entry = entry.map_err(|source| LlamaError::UnpackTar {
            asset: asset.to_string(),
            source,
        })?;
        let name = entry
            .path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "<unreadable entry>".to_string());
        let escaped = match entry.unpack_in(into) {
            Ok(true) => false,
            Ok(false) => true,
            Err(source) => {
                return Err(LlamaError::UnpackTar {
                    asset: asset.to_string(),
                    source,
                })
            }
        };
        if escaped {
            return Err(LlamaError::UnpackTar {
                asset: asset.to_string(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("{name:?} escapes the engine directory"),
                ),
            });
        }
    }
    Ok(())
}

fn unpack_zip(archive: &Path, into: &Path, asset: &str) -> Result<(), LlamaError> {
    let file = std::fs::File::open(archive).map_err(|source| LlamaError::ReadArchive {
        asset: asset.to_string(),
        source,
    })?;
    let mut zip = zip::ZipArchive::new(file).map_err(|source| LlamaError::UnpackZip {
        asset: asset.to_string(),
        detail: source.to_string(),
    })?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|source| LlamaError::UnpackZip {
            asset: asset.to_string(),
            detail: source.to_string(),
        })?;
        // `enclosed_name` is the sanitiser: it returns None for absolute paths and for entries that
        // would escape the destination.
        let Some(name) = entry.enclosed_name() else {
            return Err(LlamaError::UnpackZip {
                asset: asset.to_string(),
                detail: format!("{:?} escapes the engine directory", entry.name()),
            });
        };
        let out = into.join(&name);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).or_err(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).or_err(parent)?;
        }
        let mut dest = std::fs::File::create(&out).map_err(|source| LlamaError::WriteEngine {
            path: out.clone(),
            source,
        })?;
        std::io::copy(&mut entry, &mut dest)
            .map_err(|source| LlamaError::WriteEngine { path: out, source })?;
    }
    Ok(())
}

/// Move the unpacked tree into place, keeping the previous install reachable until the new one is
/// settled so an interruption cannot leave the target directory missing.
fn swap_into_place(
    final_dir: &Path,
    staging: &Path,
    tag: &Path,
    key: &str,
) -> Result<(), LlamaError> {
    let old = tag.join(format!("{key}.old"));
    let _ = std::fs::remove_dir_all(&old);
    if final_dir.exists() {
        std::fs::rename(final_dir, &old).map_err(|source| LlamaError::SwapEngine {
            path: final_dir.to_path_buf(),
            source,
        })?;
    }
    if let Err(source) = std::fs::rename(staging, final_dir) {
        if old.exists() {
            let _ = std::fs::rename(&old, final_dir);
        }
        return Err(LlamaError::SwapEngine {
            path: final_dir.to_path_buf(),
            source,
        });
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}

#[cfg(unix)]
fn make_executable(path: &Path, asset: &str) -> Result<(), LlamaError> {
    use std::os::unix::fs::PermissionsExt;
    // A tar entry can lose its mode when the archive is produced by a non-POSIX tool, and the
    // resulting failure ("cannot execute binary file") appears at launch, far from its cause.
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).map_err(|source| {
        LlamaError::WriteEngine {
            path: path.to_path_buf(),
            source,
        }
    })?;
    let _ = asset;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path, _asset: &str) -> Result<(), LlamaError> {
    Ok(())
}

/// `std::fs` errors carry no path; every one of these becomes a message that names the file,
/// because "No such file or directory" from a 300 MB install is not actionable.
trait OrErr {
    fn or_err(self, path: &Path) -> Result<(), LlamaError>;
}
impl<T> OrErr for std::io::Result<T> {
    fn or_err(self, path: &Path) -> Result<(), LlamaError> {
        self.map(|_| ()).map_err(|source| LlamaError::WriteEngine {
            path: path.to_path_buf(),
            source,
        })
    }
}

/// Clear a scratch directory, treating its absence as success — the first install of a target has
/// nothing to clear, and that is not a failure.
fn remove_tree(path: &Path) -> Result<(), LlamaError> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(LlamaError::WriteEngine {
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use freecode_models::hardware::Backend;
    use std::io::Write;

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn lock() -> EngineLock {
        EngineLock::load_default(&repo_root()).expect("the repo's engine lock must parse")
    }

    fn target(lock: &EngineLock, os: &str, arch: &str, backend: Backend) -> EngineTarget {
        lock.target_for(os, arch, backend)
            .expect("pinned target")
            .clone()
    }

    #[test]
    fn the_plan_is_the_pinned_asset_with_the_locks_own_digest() {
        let l = lock();
        let t = target(&l, "darwin", "aarch64", Backend::Metal);
        let p = plan_for(&l, Path::new("/work"), &t);
        assert_eq!(p.url, "https://github.com/PrismML-Eng/llama.cpp/releases/download/prism-b10787-73bfe6c/llama-prism-b10787-73bfe6c-bin-macos-arm64.tar.gz");
        assert_eq!(p.expected_sha256, t.sha256);
        assert_eq!(p.expected_size_bytes, t.size_bytes);
        assert_eq!(
            p.dest,
            PathBuf::from("/work/engines/prism-b10787-73bfe6c/.parts/llama-prism-b10787-73bfe6c-bin-macos-arm64.tar.gz")
        );
        // A public release asset must never be fetched with the model token attached.
        assert_eq!(p.token_env, None);
    }

    /// A stand-in for a release tarball: gzip-compressed tar, entries written through the raw header
    /// API because `tar::Builder` refuses the `..` paths the archive-slip test has to plant. The
    /// layout mirrors the pinned `…-bin-macos-arm64.tar.gz`, which wraps everything in one
    /// `llama-prism-<tag>/` directory.
    const FAKE_BANNER: &[u8] = b"#!/bin/sh\necho 'version: 10787 (73bfe6c)'\n";
    const WRAPPED_SERVER: &str = "llama-prism-b10787-73bfe6c/llama-server";

    fn write_tar_gz(into: &Path, asset: &str, entries: &[(&str, &[u8])]) -> PathBuf {
        std::fs::create_dir_all(into).unwrap();
        let path = into.join(asset);
        let mut gz = flate2::write::GzEncoder::new(
            std::fs::File::create(&path).unwrap(),
            flate2::Compression::default(),
        );
        for (name, body) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o755);
            header.set_entry_type(tar::EntryType::Regular);
            // The name field is bytes 0..100 of the header block, and the block starts zeroed.
            header.as_mut_bytes()[..name.len()].copy_from_slice(name.as_bytes());
            header.set_cksum();
            gz.write_all(header.as_bytes()).unwrap();
            gz.write_all(body).unwrap();
            gz.write_all(&vec![0u8; (512 - body.len() % 512) % 512])
                .unwrap();
        }
        gz.write_all(&[0u8; 1024]).unwrap();
        gz.finish().unwrap();
        path
    }

    #[test]
    fn unpacking_lands_the_server_binary_where_the_resolver_looks_for_it() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let l = lock();
        let t = target(&l, "darwin", "aarch64", Backend::Metal);
        let archive = write_tar_gz(
            &tag_dir(root, &l).join(PARTS_DIR),
            &t.asset,
            &[(WRAPPED_SERVER, FAKE_BANNER)],
        );

        let installed = unpack(&l, root, &t, &archive).expect("a well-formed asset unpacks");
        // The single wrapping directory is unwrapped, so the resolver finds the server at the root
        // of the install directory.
        assert_eq!(installed.server_path, installed.dir.join("llama-server"));
        assert!(
            installed.server_path.is_file(),
            "{:?}",
            installed.server_path
        );
        assert_eq!(
            installed.dir,
            root.join("engines").join(&l.pinned_tag).join("macos-arm64")
        );
        // The resolver's own readiness probe must now agree, or the install is invisible.
        let e = EngineBinary::from_target(root, &l, &t).unwrap();
        assert!(e.installed, "{e:?}");
        assert_eq!(e.server_path, installed.server_path);
        // Neither the archive nor the staging tree survives.
        assert!(!archive.exists());
        assert!(!tag_dir(root, &l).join(STAGING_DIR).exists());
    }

    #[test]
    fn reinstalling_replaces_the_previous_tree_rather_than_merging_into_it() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let l = lock();
        let t = target(&l, "darwin", "aarch64", Backend::Metal);
        let parts = tag_dir(root, &l).join(PARTS_DIR);

        let first = write_tar_gz(&parts, &t.asset, &[(WRAPPED_SERVER, &b"old build\n"[..])]);
        unpack(&l, root, &t, &first).unwrap();
        // A file the second archive does not contain must not survive the swap.
        let leftover = l.install_dir(&engines_root(root), &t).join("stale.txt");
        std::fs::write(&leftover, b"from the old build").unwrap();

        let second = write_tar_gz(&parts, &t.asset, &[(WRAPPED_SERVER, &b"new build\n"[..])]);
        let installed = unpack(&l, root, &t, &second).unwrap();
        assert!(!leftover.exists(), "the old tree was merged, not replaced");
        assert!(installed.server_path.is_file());
        assert_eq!(
            std::fs::read_to_string(&installed.server_path).unwrap(),
            "new build\n"
        );
        assert!(
            !tag_dir(root, &l).join(format!("{}.old", t.key)).exists(),
            "the moved-aside tree must be cleaned up"
        );
    }

    #[test]
    fn an_asset_without_the_server_binary_is_refused_before_it_reaches_engines() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let l = lock();
        let t = target(&l, "darwin", "aarch64", Backend::Metal);
        let archive = write_tar_gz(
            &tag_dir(root, &l).join(PARTS_DIR),
            &t.asset,
            &[("llama-prism-b10787-73bfe6c/llama-cli", FAKE_BANNER)],
        );

        let err = unpack(&l, root, &t, &archive).expect_err("no llama-server means no install");
        assert!(
            matches!(err, LlamaError::ServerBinaryMissing { .. }),
            "{err:?}"
        );
        // Refusing must leave the target directory absent, not empty-but-present.
        assert!(!l.install_dir(&engines_root(root), &t).exists());
    }

    #[test]
    fn an_entry_escaping_the_engine_directory_fails_the_unpack() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let l = lock();
        let t = target(&l, "darwin", "aarch64", Backend::Metal);
        let path = write_tar_gz(
            &tag_dir(root, &l).join(PARTS_DIR),
            &t.asset,
            &[("../../../../etc/llama-server", b"payload")],
        );

        let err = unpack(&l, root, &t, &path).expect_err("archive slip must be refused");
        assert!(matches!(err, LlamaError::UnpackTar { .. }), "{err:?}");
        assert!(!l.install_dir(&engines_root(root), &t).exists());
        // The escape attempt wrote nothing outside the engine tree.
        assert!(!Path::new("/etc/llama-server").exists());
    }

    #[test]
    fn a_zip_asset_unpacks_to_the_same_place_as_a_tarball() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let l = lock();
        // Windows ships .zip, so the zip path is the one that carries every real install there.
        let t = target(&l, "windows", "x86_64", Backend::Cuda);
        std::fs::create_dir_all(tag_dir(root, &l).join(PARTS_DIR)).unwrap();
        let path = tag_dir(root, &l).join(PARTS_DIR).join(&t.asset);
        let mut writer = zip::ZipWriter::new(std::io::BufWriter::new(
            std::fs::File::create(&path).unwrap(),
        ));
        let opts: zip::write::FileOptions<_> = zip::write::SimpleFileOptions::default();
        // The pinned win-cpu-arm64 zip is flat: `llama-server.exe` sits beside the ggml DLLs.
        for name in ["ggml.dll", "llama-server.exe"] {
            writer.start_file(name, opts).unwrap();
            writer.write_all(b"MZ").unwrap();
        }
        writer.finish().unwrap();

        let installed = unpack(&l, root, &t, &path).expect("the zip path unpacks");
        assert_eq!(
            installed.server_path,
            installed.dir.join("llama-server.exe")
        );
        assert!(
            installed.server_path.is_file(),
            "{:?}",
            installed.server_path
        );
        assert!(EngineBinary::from_target(root, &l, &t).unwrap().installed);
    }

    #[test]
    fn an_unknown_archive_extension_is_named_rather_than_attempted() {
        let tmp = tempfile::tempdir().unwrap();
        let l = lock();
        let mut t = target(&l, "darwin", "aarch64", Backend::Metal);
        t.asset = "engine.7z".into();
        let err = unpack(&l, tmp.path(), &t, Path::new("/nope")).expect_err("cannot unpack 7z");
        assert!(
            matches!(err, LlamaError::UnknownArchiveFormat { .. }),
            "{err:?}"
        );
    }

    /// The first install on a machine has no `engines/<tag>/` at all, and `download` opens its
    /// `.part` file long before it creates the destination. Without the scratch directories the
    /// transfer dies with "No such file or directory (os error 2)" on the byte that should have
    /// been the first of the engine.
    #[test]
    fn the_scratch_directories_are_there_before_the_transfer_needs_them() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let l = lock();
        let parts = prepare_scratch(root, &l).expect("a clean engines/ tree gets its scratch");
        assert_eq!(parts, tag_dir(root, &l).join(PARTS_DIR));
        assert!(parts.is_dir());
        assert!(tag_dir(root, &l).join(STAGING_DIR).is_dir());
        // The part file the plan names is writable the moment the resume logic opens it.
        let plan = plan_for(&l, root, &target(&l, "darwin", "aarch64", Backend::Metal));
        std::fs::File::create(plan.part_path()).expect("the .part file must be creatable");

        // And unpacking into the same clean tree needs nothing more than the archive.
        let t = target(&l, "darwin", "aarch64", Backend::Metal);
        let archive = write_tar_gz(root, &t.asset, &[(WRAPPED_SERVER, FAKE_BANNER)]);
        let installed = unpack(&l, root, &t, &archive).expect("first unpack onto a clean root");
        assert!(EngineBinary::from_target(root, &l, &t).unwrap().installed);
        assert!(installed.server_path.is_file());
    }

    /// Proves the installer against the bytes GitHub actually ships rather than only against the
    /// fixtures above, and re-checks the digest recorded in `engines/lock.toml`. Deliberately
    /// `#[ignore]`d: CI never downloads, and the asset has to be fetched out of band first.
    ///
    ///   curl -sSL -o /tmp/asset.tar.gz \
    ///     https://github.com/PrismML-Eng/llama.cpp/releases/download/prism-b10787-73bfe6c/llama-prism-b10787-73bfe6c-bin-macos-arm64.tar.gz
    ///   FREECODE_ENGINE_ARCHIVE=/tmp/asset.tar.gz cargo test -p freecode-llama -- --ignored
    #[test]
    #[ignore]
    fn a_downloaded_release_asset_installs_where_the_resolver_looks() {
        let archive = PathBuf::from(std::env::var("FREECODE_ENGINE_ARCHIVE").unwrap());
        let l = lock();
        let name = archive.file_name().unwrap().to_string_lossy().to_string();
        let t = l
            .targets
            .iter()
            .find(|t| t.asset == name)
            .unwrap_or_else(|| panic!("{name} is not an asset engines/lock.toml pins"))
            .clone();

        // The asset on disk must be the asset the lock describes, or this proves nothing.
        assert_eq!(
            archive.metadata().unwrap().len(),
            t.size_bytes,
            "{name}: size differs from engines/lock.toml"
        );
        assert_eq!(
            freecode_models::gguf::sha256_file(&archive).unwrap(),
            t.sha256,
            "{name}: digest differs from engines/lock.toml"
        );

        let tmp = tempfile::tempdir().unwrap();
        let parts = tag_dir(tmp.path(), &l).join(PARTS_DIR);
        std::fs::create_dir_all(&parts).unwrap();
        let staged = parts.join(&name);
        std::fs::copy(&archive, &staged).unwrap();

        let installed = unpack(&l, tmp.path(), &t, &staged).expect("the real asset unpacks");
        assert!(
            installed.server_path.is_file(),
            "{:?}",
            installed.server_path
        );
        assert_eq!(
            installed.server_path,
            installed.dir.join(format!(
                "llama-server{}",
                if t.asset.ends_with(".zip") {
                    ".exe"
                } else {
                    ""
                }
            ))
        );
        let e = EngineBinary::from_target(tmp.path(), &l, &t).unwrap();
        assert!(e.installed, "{e:?}");
        assert_eq!(e.server_path, installed.server_path);
    }
}
