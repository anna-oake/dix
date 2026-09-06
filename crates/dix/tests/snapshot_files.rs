#![cfg(feature = "json")]
use std::{
  fs,
  process::Command,
};

use dix::{
  diff_store_snapshots,
  json::JsonReport,
  snapshot_file::SnapshotFile,
};
use serde_json::{
  Value,
  json,
};

fn fixture(version: &str, size: i64) -> Value {
  let path =
    format!("/nix/store/00000000000000000000000000000000-hello-{version}");
  json!({"schema_version":1,"root":path,"closure":[{"path":path,"nar_size":size}],"selected":[path]})
}

#[test]
fn roundtrip_preserves_report_and_atomic_replacement() {
  let old =
    SnapshotFile::read(fixture("1.0", 100).to_string().as_bytes()).unwrap();
  let new =
    SnapshotFile::read(fixture("2.0", 200).to_string().as_bytes()).unwrap();
  let dir = tempfile::tempdir().unwrap();
  let path = dir.path().join("snapshot.json");
  old.write_atomic(&path).unwrap();
  new.write_atomic(&path).unwrap();
  let restored = SnapshotFile::read(fs::File::open(path).unwrap()).unwrap();
  let expected = diff_store_snapshots(
    &old.to_snapshot().unwrap(),
    &new.to_snapshot().unwrap(),
  );
  let actual = diff_store_snapshots(
    &old.to_snapshot().unwrap(),
    &restored.to_snapshot().unwrap(),
  );
  assert_eq!(
    serde_json::to_value(JsonReport::from(&expected)).unwrap(),
    serde_json::to_value(JsonReport::from(&actual)).unwrap()
  );
}

#[test]
fn rejects_corrupt_or_incompatible_snapshots() {
  for mutation in 0..7 {
    let mut value = fixture("1.0", 100);
    match mutation {
      0 => value["schema_version"] = json!(2),
      1 => value["closure"][0]["nar_size"] = json!(-1),
      2 => {
        let entry = value["closure"][0].clone();
        value["closure"].as_array_mut().unwrap().push(entry);
      },
      3 => {
        value["root"] =
          json!("/nix/store/00000000000000000000000000000000-missing");
      },
      4 => value["closure"][0]["path"] = json!("/nix/store/../etc/passwd"),
      5 => {
        value["selected"] =
          json!(["/nix/store/00000000000000000000000000000000-missing"]);
      },
      _ => {
        value["closure"][0]["nar_size"] = json!(i64::MAX);
        value["closure"].as_array_mut().unwrap().push(json!({"path":"/nix/store/11111111111111111111111111111111-other", "nar_size":1}));
      },
    }
    assert!(
      SnapshotFile::read(value.to_string().as_bytes()).is_err(),
      "mutation {mutation}"
    );
  }
}

#[test]
fn cli_compares_missing_store_paths_without_nix() {
  let dir = tempfile::tempdir().unwrap();
  let old = dir.path().join("old.json");
  let new = dir.path().join("new.json");
  fs::write(&old, fixture("1.0", 100).to_string()).unwrap();
  fs::write(&new, fixture("2.0", 200).to_string()).unwrap();
  let output = Command::new(env!("CARGO_BIN_EXE_dix"))
    .env("PATH", dir.path())
    .args([
      "diff-snapshots",
      old.to_str().unwrap(),
      new.to_str().unwrap(),
      "--output",
      "json",
      "-v",
    ])
    .output()
    .unwrap();
  assert!(
    output.status.success(),
    "{}",
    String::from_utf8_lossy(&output.stderr)
  );
  let report: Value = serde_json::from_slice(&output.stdout).unwrap();
  assert_eq!(report["size_old"], 100);
  assert_eq!(report["size_new"], 200);
  assert_eq!(report["paths"]["added"], 1);
  assert_eq!(report["paths"]["removed"], 1);
  assert_eq!(report["diffs"][0]["status"], "Upgraded");
  assert_eq!(report["diffs"][0]["selection"], "Selected");
  let output = Command::new(env!("CARGO_BIN_EXE_dix"))
    .env("PATH", dir.path())
    .args([
      "diff-snapshots",
      old.to_str().unwrap(),
      new.to_str().unwrap(),
      "--color",
      "never",
    ])
    .output()
    .unwrap();
  assert!(output.status.success());
  let text = String::from_utf8(output.stdout).unwrap();
  assert!(text.contains("hello"));
  assert!(text.contains("<<< /nix/store/"));
}
