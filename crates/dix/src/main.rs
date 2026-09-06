use std::{
  env,
  fmt::{
    self,
    Write as _,
  },
  fs,
  io::{
    self,
    IsTerminal as _,
  },
  path::{
    Path,
    PathBuf,
  },
};

use clap::Parser as _;
#[cfg(feature = "json")] use dix::json;
use eyre::eyre;
use yansi::Paint as _;

struct WriteFmt<W: io::Write>(W);

impl<W: io::Write> fmt::Write for WriteFmt<W> {
  fn write_str(&mut self, string: &str) -> fmt::Result {
    self.0.write_all(string.as_bytes()).map_err(|_| fmt::Error)
  }
}

#[derive(clap::Parser, Debug)]
#[command(
  version,
  about,
  args_conflicts_with_subcommands = true,
  subcommand_negates_reqs = true
)]
struct Cli {
  #[arg(required = true)]
  old_path: Option<PathBuf>,
  #[arg(required = true)]
  new_path: Option<PathBuf>,

  #[command(subcommand)]
  command: Option<Command>,

  #[command(flatten)]
  verbose: clap_verbosity_flag::Verbosity,

  /// Controls when to use color.
  #[arg(
      long,
      default_value_t = clap::ColorChoice::Auto,
      value_name = "WHEN",
      global = true,
  )]
  color: clap::ColorChoice,

  /// Fall back to a backend chain that skips `SQLite` immutable mode.
  ///
  /// This is relevant if the output of dix is to be used for more
  /// critical applications and not just as human-readable overview.
  ///
  /// The default backend falls back to opening Nix's `SQLite` database with
  /// `?immutable=1` if the normal connection fails. That is faster than Nix
  /// commands, but can be inaccurate if the database is being written to at
  /// the same time.
  #[arg(long, default_value_t = false, global = true)]
  force_correctness: bool,

  /// Select the output format to use.
  #[arg(long, value_enum, default_value_t = OutputFormat::Human, global = true)]
  output: OutputFormat,
}

/// Snapshot operations; ordinary positional comparisons remain supported.
#[derive(clap::Subcommand, Debug)]
enum Command {
  /// Export a versioned snapshot of a built output (requires the json feature).
  Snapshot {
    path: PathBuf,
    /// Atomically write to this file instead of stdout.
    #[arg(long)]
    file: Option<PathBuf>,
  },
  /// Compare snapshot files without Nix or the original store paths.
  DiffSnapshots {
    old_snapshot: PathBuf,
    new_snapshot: PathBuf,
  },
}

/// Determines the output format to be used by dix.
#[derive(Debug, Clone, Copy, clap::ValueEnum, Eq, PartialEq)]
enum OutputFormat {
  /// Output in the default dix format highlighting version changes.
  Human,
  /// Display the output as JSON for machine parsing (requires `json` feature).
  Json,
}

fn main() -> eyre::Result<()> {
  let Cli {
    old_path,
    new_path,
    verbose,
    color,
    force_correctness,
    output,
    command,
  } = Cli::parse();

  yansi::whenever(match color {
    clap::ColorChoice::Auto => yansi::Condition::from(should_style),
    clap::ColorChoice::Always => yansi::Condition::ALWAYS,
    clap::ColorChoice::Never => yansi::Condition::NEVER,
  });

  tracing_subscriber::fmt()
    .with_env_filter(
      tracing_subscriber::EnvFilter::builder()
        .with_default_directive(match verbose.log_level_filter() {
          clap_verbosity_flag::log::LevelFilter::Off
          | clap_verbosity_flag::log::LevelFilter::Error => {
            tracing::Level::ERROR.into()
          },
          clap_verbosity_flag::log::LevelFilter::Warn => {
            tracing::Level::WARN.into()
          },
          clap_verbosity_flag::log::LevelFilter::Info => {
            tracing::Level::INFO.into()
          },
          clap_verbosity_flag::log::LevelFilter::Debug => {
            tracing::Level::DEBUG.into()
          },
          clap_verbosity_flag::log::LevelFilter::Trace => {
            tracing::Level::TRACE.into()
          },
        })
        .from_env_lossy(),
    )
    .with_writer(io::stderr)
    .with_ansi(should_style())
    .with_target(false)
    .without_time()
    .init();

  if let Some(command) = command {
    return run_snapshot_command(command, output);
  }
  let old_path = old_path.ok_or_else(|| eyre!("old path is required"))?;
  let new_path = new_path.ok_or_else(|| eyre!("new path is required"))?;
  for path in [&old_path, &new_path] {
    if !path.exists() {
      return Err(eyre!("profile path does not exist: {}", path.display()));
    }
  }

  if force_correctness {
    tracing::warn!(
      "Falling back to slower but more robust backends (force_correctness is \
       set)."
    );
  }
  match output {
    OutputFormat::Human => {
      display_diff(&old_path, &new_path, force_correctness)?;
    },
    #[cfg(feature = "json")]
    OutputFormat::Json => {
      json::display_diff(&old_path, &new_path, force_correctness)?;
    },
    #[cfg(not(feature = "json"))]
    OutputFormat::Json => {
      return Err(eyre!(
        "The 'json' feature is required to use '--output json'."
      ));
    },
  }

  Ok(())
}

#[cfg(feature = "json")]
fn run_snapshot_command(
  command: Command,
  output: OutputFormat,
) -> eyre::Result<()> {
  use dix::snapshot_file::SnapshotFile;
  match command {
    Command::Snapshot { path, file } => {
      // Snapshot data must remain dependable after the source is collected.
      let snapshot = SnapshotFile::capture(&path)?;
      if let Some(file) = file {
        snapshot.write_atomic(&file)?;
      } else {
        snapshot.write(io::stdout().lock())?;
      }
    },
    Command::DiffSnapshots {
      old_snapshot,
      new_snapshot,
    } => {
      let old = SnapshotFile::read(fs::File::open(old_snapshot)?)?;
      let new = SnapshotFile::read(fs::File::open(new_snapshot)?)?;
      let report =
        dix::diff_store_snapshots(&old.to_snapshot()?, &new.to_snapshot()?);
      match output {
        OutputFormat::Human => {
          let mut out = WriteFmt(io::stdout().lock());
          writeln!(out, "<<< {}", old.root)?;
          writeln!(out, ">>> {}", new.root)?;
          dix::write_diff_report(&mut out, &report)?;
        },
        OutputFormat::Json => {
          json::generate_diff(&mut io::stdout().lock(), &report)?;
        },
      }
    },
  }
  Ok(())
}

#[cfg(not(feature = "json"))]
fn run_snapshot_command(
  _command: Command,
  _output: OutputFormat,
) -> eyre::Result<()> {
  Err(eyre!("snapshot commands require the 'json' feature"))
}

fn display_diff(
  old_path: &Path,
  new_path: &Path,
  force_correctness: bool,
) -> eyre::Result<()> {
  let mut out = WriteFmt(io::stdout());

  tracing::info!("starting diff computation");

  writeln!(
    out,
    "{arrows} {old}",
    arrows = "<<<".bold(),
    old = fs::canonicalize(old_path)
      .unwrap_or_else(|_| old_path.to_path_buf())
      .display(),
  )?;
  writeln!(
    out,
    "{arrows} {new}",
    arrows = ">>>".bold(),
    new = fs::canonicalize(new_path)
      .unwrap_or_else(|_| new_path.to_path_buf())
      .display(),
  )?;

  let report = dix::query_diff_report(old_path, new_path, force_correctness)?;
  dix::write_diff_report(&mut out, &report)?;

  tracing::info!("diff computation complete");

  Ok(())
}

// https://bixense.com/clicolors/
fn should_style() -> bool {
  // If NO_COLOR is set and is not empty, don't style.
  if let Some(value) = env::var_os("NO_COLOR")
    && !value.is_empty()
  {
    return false;
  }

  // If CLICOLOR is set and is 0, don't style.
  if let Some(value) = env::var_os("CLICOLOR")
    && value == "0"
  {
    return false;
  }

  // If CLICOLOR_FORCE is set and not 0, always style.
  if let Some(value) = env::var_os("CLICOLOR_FORCE")
    && value != "0"
  {
    return true;
  }

  // Style if it is a terminal.
  io::stdout().is_terminal()
}
