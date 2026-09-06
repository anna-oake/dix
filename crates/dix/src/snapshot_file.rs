//! Portable, versioned snapshots. Reading and comparing these never accesses
//! Nix.
use std::{
  collections::BTreeSet,
  fs,
  io::{
    Read,
    Write,
  },
  path::{
    Path,
    PathBuf,
  },
};

use eyre::{
  Result,
  WrapErr as _,
  ensure,
};
use serde::{
  Deserialize,
  Serialize,
};
use size::Size;

use crate::{
  StorePath,
  StoreSnapshot,
  query_store_snapshot,
  store::StorePathInfo,
};

/// Version 1 stores byte sizes and full store paths, independent of dix
/// releases.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotFile {
  pub schema_version: u32,
  pub root:           String,
  pub closure:        Vec<SnapshotPath>,
  pub selected:       Vec<String>,
}

/// One unique member of a runtime closure.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotPath {
  pub path:     String,
  pub nar_size: i64,
}

impl SnapshotFile {
  /// Capture a built output using the correctness-preserving backend chain.
  ///
  /// # Errors
  /// Fails if the output or its metadata cannot be read.
  pub fn capture(root: &Path) -> Result<Self> {
    let root = if root.parent() == Some(Path::new("/nix/store")) {
      root.to_path_buf()
    } else {
      fs::canonicalize(root).wrap_err("cannot resolve snapshot root")?
    };
    let snapshot = query_store_snapshot(&root, true)?;
    let mut closure = snapshot
      .closure
      .iter()
      .map(|info| {
        SnapshotPath {
          path:     info.path().to_string_lossy().into_owned(),
          nar_size: info.nar_size().bytes(),
        }
      })
      .collect::<Vec<_>>();
    closure.sort_by(|a, b| a.path.cmp(&b.path));
    let mut selected = snapshot
      .selected
      .iter()
      .map(|p| p.to_string_lossy().into_owned())
      .collect::<Vec<_>>();
    selected.sort();
    selected.dedup();
    let file = Self {
      schema_version: 1,
      root: root.to_string_lossy().into_owned(),
      closure,
      selected,
    };
    file.to_snapshot()?;
    Ok(file)
  }

  /// Read and validate metadata without resolving any store paths.
  ///
  /// # Errors
  /// Rejects invalid JSON, unsupported versions, or inconsistent snapshots.
  pub fn read(reader: impl Read) -> Result<Self> {
    let file: Self =
      serde_json::from_reader(reader).wrap_err("invalid snapshot JSON")?;
    file.to_snapshot()?;
    Ok(file)
  }

  /// Write compact JSON with a trailing newline.
  ///
  /// # Errors
  /// Fails on invalid metadata or a write error.
  pub fn write(&self, mut writer: impl Write) -> Result<()> {
    self.to_snapshot()?;
    serde_json::to_writer(&mut writer, self)?;
    writer.write_all(b"\n")?;
    Ok(())
  }

  /// Replace a file atomically, using a temporary file in the same directory.
  /// Parent directories must already exist.
  ///
  /// # Errors
  /// Fails if serialization, syncing, or renaming fails.
  pub fn write_atomic(&self, path: &Path) -> Result<()> {
    let parent = path
      .parent()
      .filter(|p| !p.as_os_str().is_empty())
      .unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    self.write(tmp.as_file_mut())?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).wrap_err("cannot publish snapshot")?;
    Ok(())
  }

  /// Convert to the existing pure comparison API.
  ///
  /// # Errors
  /// Rejects malformed paths, duplicate entries, invalid sizes, and missing
  /// roots.
  pub fn to_snapshot(&self) -> Result<StoreSnapshot> {
    ensure!(
      self.schema_version == 1,
      "unsupported snapshot schema version: {}",
      self.schema_version
    );
    validate_path(&self.root)?;
    let mut paths = BTreeSet::new();
    let mut total = 0_i64;
    let closure = self
      .closure
      .iter()
      .map(|entry| {
        let path = validate_path(&entry.path)?;
        ensure!(
          paths.insert(entry.path.as_str()),
          "duplicate closure path: {}",
          entry.path
        );
        ensure!(entry.nar_size >= 0, "negative NAR size: {}", entry.path);
        total = total
          .checked_add(entry.nar_size)
          .ok_or_else(|| eyre::eyre!("closure size exceeds i64"))?;
        Ok(StorePathInfo::new(path, Size::from_bytes(entry.nar_size)))
      })
      .collect::<Result<Vec<_>>>()?;
    ensure!(
      paths.contains(self.root.as_str()),
      "snapshot root is missing from closure"
    );
    let mut seen = BTreeSet::new();
    let selected = self
      .selected
      .iter()
      .map(|path| {
        ensure!(
          paths.contains(path.as_str()),
          "selected path is missing from closure: {path}"
        );
        ensure!(seen.insert(path), "duplicate selected path: {path}");
        validate_path(path)
      })
      .collect::<Result<Vec<_>>>()?;
    Ok(StoreSnapshot { closure, selected })
  }
}

fn validate_path(value: &str) -> Result<StorePath> {
  let path = PathBuf::from(value);
  ensure!(
    path.parent() == Some(Path::new("/nix/store")),
    "not a top-level store path: {value}"
  );
  let name = value
    .strip_prefix("/nix/store/")
    .ok_or_else(|| eyre::eyre!("invalid store path: {value}"))?;
  let (hash, name) = name
    .split_once('-')
    .ok_or_else(|| eyre::eyre!("invalid store path: {value}"))?;
  ensure!(
    hash.len() == 32
      && hash
        .bytes()
        .all(|b| b"0123456789abcdfghijklmnpqrsvwxyz".contains(&b))
      && !name.is_empty()
      && name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"+-._?=".contains(&b)),
    "invalid store path: {value}"
  );
  StorePath::try_from(path)
}
