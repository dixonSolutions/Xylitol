//! The Xylitol command line.
//!
//! Everything the GUI can do is reachable here, which is what makes the APKPure
//! side testable on a machine with no display.

mod select;

use std::io::{IsTerminal, Write};
use std::path::PathBuf;

use anyhow::Context;
use clap::{Args, Parser, Subcommand, ValueEnum};
use xylitol_core::apkpure::{CancelToken, Client, FileKind, Progress, Variant};
use xylitol_core::library::Library;
use xylitol_core::{human_size, paths, shim as core_shim};

#[derive(Parser)]
#[command(
    name = "xylitol-cli",
    version,
    about = "Find, download and install Android packages from APKPure"
)]
struct Cli {
    /// Print machine-readable JSON instead of a table.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Search APKPure by app name or package name.
    Search {
        query: String,
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    /// List every published release of an app.
    Versions { package: String },
    /// List every downloadable file for an app: the choices for `download`.
    Variants {
        package: String,
        /// Restrict to one release, by version name (e.g. `3.7.1`).
        #[arg(long)]
        version: Option<String>,
    },
    /// Download one file.
    Download(DownloadArgs),
    /// Show what a local APK or XAPK contains.
    Inspect { file: PathBuf },
    /// Report whether Xylitol's shim can run an app's native code, and what is
    /// missing if it cannot. With no file, report what the shim itself can do.
    Shim {
        file: Option<PathBuf>,
        /// List every symbol with no implementation.
        #[arg(long)]
        unimplemented: bool,
    },
    /// Work with downloaded packages.
    #[command(subcommand)]
    Library(LibraryCommand),
}

#[derive(Args)]
struct DownloadArgs {
    package: String,
    /// Release to download, by version name. Defaults to the newest.
    #[arg(long)]
    version: Option<String>,
    /// Exact build to download, by version code.
    #[arg(long)]
    version_code: Option<i64>,
    /// Only consider builds for this ABI (e.g. `arm64-v8a`, `universal`).
    #[arg(long)]
    arch: Option<String>,
    /// Only consider files of this kind.
    #[arg(long)]
    kind: Option<Kind>,
    /// Choose by position in the `variants` listing, starting at 1.
    #[arg(long)]
    pick: Option<usize>,
    /// Take the first match without asking, even when several remain.
    #[arg(long, short = 'y')]
    yes: bool,
    /// Where to save. Defaults to Xylitol's package directory.
    #[arg(long)]
    dir: Option<PathBuf>,
    /// List what would be downloaded and stop.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum Kind {
    Apk,
    Xapk,
}

impl From<Kind> for FileKind {
    fn from(k: Kind) -> FileKind {
        match k {
            Kind::Apk => FileKind::Apk,
            Kind::Xapk => FileKind::Xapk,
        }
    }
}

#[derive(Subcommand)]
enum LibraryCommand {
    /// List downloaded packages.
    List,
    /// Add an APK or XAPK that is already on disk.
    Add { file: PathBuf },
    /// Forget a package. Pass `--delete` to remove the file too.
    Remove {
        key: String,
        #[arg(long)]
        delete: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("XYLITOL_LOG")
                .unwrap_or_else(|_| "warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::Search { query, limit } => search(&query, limit, cli.json).await,
        Command::Versions { package } => versions(&package, cli.json).await,
        Command::Variants { package, version } => {
            list_variants(&package, version.as_deref(), cli.json).await
        }
        Command::Download(args) => download(args, cli.json).await,
        Command::Inspect { file } => inspect(&file, cli.json),
        Command::Shim {
            file,
            unimplemented,
        } => match file {
            Some(file) => shim(&file, unimplemented, cli.json),
            None => shim_capability(cli.json),
        },
        Command::Library(cmd) => library(cmd, cli.json),
    }
}

async fn search(query: &str, limit: usize, json: bool) -> anyhow::Result<()> {
    let hits = Client::new().search(query, limit).await?;
    if json {
        return print_json(&hits);
    }
    if hits.is_empty() {
        println!("No apps matched {query:?}.");
        return Ok(());
    }
    for hit in &hits {
        println!("{}", hit.title);
        println!("  package  {}", hit.package);
        if let Some(v) = &hit.latest_version {
            let code = hit
                .latest_version_code
                .map(|c| format!(" ({c})"))
                .unwrap_or_default();
            println!("  latest   {v}{code}");
        }
        if let Some(size) = hit.latest_size {
            println!("  size     {}", human_size(size));
        }
        if let Some(installs) = &hit.installs {
            println!("  installs {installs}");
        }
        println!();
    }
    Ok(())
}

async fn versions(package: &str, json: bool) -> anyhow::Result<()> {
    let releases = Client::new().releases(package).await?;
    if json {
        return print_json(&releases);
    }
    println!("{} release(s) of {package}\n", releases.len());
    for r in &releases {
        let code = r
            .version_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| "?".into());
        let size = r.size.map(human_size).unwrap_or_else(|| "?".into());
        let variants = if r.has_variants {
            "  [multiple files]"
        } else {
            ""
        };
        println!(
            "  {:<20} {:<12} {:<10} {:>10}  {}{}",
            r.version_name,
            code,
            r.kinds_display(),
            size,
            r.published.as_deref().unwrap_or(""),
            variants
        );
    }
    Ok(())
}

async fn list_variants(package: &str, version: Option<&str>, json: bool) -> anyhow::Result<()> {
    let variants = Client::new().variants(package, version).await?;
    if json {
        return print_json(&variants);
    }
    if variants.is_empty() {
        println!("APKPure lists no downloadable file for {package}.");
        return Ok(());
    }
    println!("{} downloadable file(s) for {package}\n", variants.len());
    print!("{}", select::render_table(&variants));
    println!("\nDownload one with:  xylitol-cli download {package} --pick <n>");
    Ok(())
}

async fn download(args: DownloadArgs, json: bool) -> anyhow::Result<()> {
    let client = Client::new();
    let all = client
        .variants(&args.package, args.version.as_deref())
        .await?;
    anyhow::ensure!(
        !all.is_empty(),
        "APKPure lists no downloadable file for {}",
        args.package
    );

    let chosen = select::choose(
        &all,
        select::Filters {
            version_code: args.version_code,
            arch: args.arch.as_deref(),
            kind: args.kind.map(Into::into),
            pick: args.pick,
            assume_yes: args.yes,
            // --json is a machine-readable mode: never stop to ask on it, even
            // from a terminal.
            interactive: !json && std::io::stdin().is_terminal(),
        },
    )?;

    if args.dry_run {
        println!("Would download:\n");
        print!("{}", select::render_table(std::slice::from_ref(chosen)));
        println!("\nfrom {}", chosen.download_url);
        return Ok(());
    }

    let dir = args.dir.unwrap_or_else(paths::download_dir);
    eprintln!(
        "Downloading {} {} [{}] to {}",
        chosen.package,
        chosen.version_display(),
        chosen.descriptor(),
        dir.display()
    );

    let cancel = CancelToken::new();
    // Ctrl-C leaves the .part file in place so the next run resumes.
    let ctrl_c = cancel.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("\nInterrupted — the partial file is kept, rerun to resume.");
            ctrl_c.cancel();
        }
    });

    let show_progress = std::io::stderr().is_terminal() && !json;
    let mut last_line = String::new();
    let result = client
        .download(chosen, &dir, &cancel, |p: Progress| {
            if !show_progress {
                return;
            }
            let line = match (p.fraction(), p.total) {
                (Some(f), Some(total)) => format!(
                    "  {:>5.1}%  {} / {}",
                    f * 100.0,
                    human_size(p.downloaded),
                    human_size(total)
                ),
                _ => format!("  {}", human_size(p.downloaded)),
            };
            if line != last_line {
                eprint!("\r{line}    ");
                let _ = std::io::stderr().flush();
                last_line = line;
            }
        })
        .await;
    if show_progress {
        eprintln!();
    }
    let done = match result {
        Ok(done) => done,
        Err(xylitol_core::apkpure::Error::Cancelled) => {
            // Already reported by the Ctrl-C handler; stopping on request is
            // not an error worth a non-zero exit and a stack of context.
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };

    let mut lib = Library::open()?;
    let entry = lib
        .add(&done.path, Some(chosen.clone()), done.verified)
        .with_context(|| format!("reading {}", done.path.display()))?;

    if json {
        return print_json(&serde_json::json!({
            "path": done.path,
            "bytes": done.bytes,
            "sha1": done.sha1,
            "checksum_verified": done.verified,
            "library_key": entry.key(),
            "info": entry.info,
        }));
    }

    println!("Saved {}", done.path.display());
    println!("  {} · sha1 {}", human_size(done.bytes), done.sha1);
    println!(
        "  checksum {}",
        if done.verified {
            "verified against APKPure"
        } else {
            "not published by APKPure, so not verified"
        }
    );
    println!("  library key {}", entry.key());
    Ok(())
}

fn inspect(file: &PathBuf, json: bool) -> anyhow::Result<()> {
    let info =
        xylitol_core::apk::inspect(file).with_context(|| format!("reading {}", file.display()))?;
    if json {
        return print_json(&info);
    }
    println!("{}", info.display_name());
    println!("  package    {}", info.package);
    println!("  version    {}", info.version_display());
    println!("  kind       {:?}", info.kind);
    if let Some(split) = &info.split {
        println!("  split      {split}");
    }
    if let (Some(min), Some(target)) = (info.min_sdk, info.target_sdk) {
        println!("  sdk        min {min}, target {target}");
    }
    if !info.abis.is_empty() {
        println!("  abis       {}", info.abis.join(", "));
    }
    if !info.contained_apks.is_empty() {
        println!("  contains   {} apk(s)", info.contained_apks.len());
    }
    println!("  size       {}", human_size(info.file_size));
    println!("  sha256     {}", info.sha256);
    if !info.launchable_activities.is_empty() {
        println!("  launches   {}", info.launchable_activities.join(", "));
    }
    println!("  permissions ({})", info.permissions.len());
    for p in &info.permissions {
        println!("    {p}");
    }
    Ok(())
}

fn shim_capability(json: bool) -> anyhow::Result<()> {
    let capability = core_shim::capability();
    if json {
        return print_json(&capability);
    }
    println!("Xylitol runs an app's native code in this process. There is no Android");
    println!("runtime, emulator or container involved, and no instruction translation:");
    println!();
    println!(
        "  loadable code   {}",
        capability
            .host_abi
            .as_deref()
            .unwrap_or("none — this CPU is not supported")
    );
    println!(
        "  platform libs   {} known by name",
        capability.known_libraries
    );
    println!(
        "  implemented     {} symbols",
        capability.implemented_symbols
    );
    println!();
    println!("Point it at a package to see whether that app can run:");
    println!("  xylitol-cli shim <file.apk>");
    Ok(())
}

fn shim(file: &PathBuf, list_unimplemented: bool, json: bool) -> anyhow::Result<()> {
    let report = xylitol_shim::report::analyse(file)
        .with_context(|| format!("analysing {}", file.display()))?;
    if json {
        return print_json(&report);
    }

    println!("{} {}", report.package, report.version);
    println!(
        "  host abi   {}",
        report
            .host_abi
            .as_deref()
            .unwrap_or("unsupported architecture")
    );
    println!("  app abis   {}", join_or(&report.available_abis, "none"));
    if !report.android_libraries.is_empty() {
        println!("  links      {}", report.android_libraries.join(", "));
    }

    if !report.objects.is_empty() {
        println!("\n  native objects loadable here:");
        for object in &report.objects {
            let entries: Vec<&str> = object
                .object
                .entry_points
                .iter()
                .map(|e| e.symbol())
                .collect();
            println!(
                "    {:<44} {:>4} shim {:>4} host {:>4} stub",
                object.object.path, object.counts.shim, object.counts.host, object.counts.stub
            );
            if !entries.is_empty() {
                println!("      entry points: {}", entries.join(", "));
            }
            if object.object.jni_natives > 0 {
                println!("      {} Java_* natives", object.object.jni_natives);
            }
        }
        let t = report.totals;
        println!(
            "\n  symbols    {} total — {} shim, {} host, {} stub",
            t.total(),
            t.shim,
            t.host,
            t.stub
        );
    }

    println!("\n  verdict    {}", report.verdict.headline());
    if report.verdict.is_runnable() {
        println!("             (nothing has been executed; this is a reading of the files)");
    }

    if list_unimplemented && !report.unimplemented.is_empty() {
        println!("\n  unimplemented ({}):", report.unimplemented.len());
        for symbol in &report.unimplemented {
            println!("    {symbol}");
        }
    } else if !report.unimplemented.is_empty() {
        println!(
            "             pass --unimplemented to list the {} missing symbol(s)",
            report.unimplemented.len()
        );
    }
    Ok(())
}

fn join_or(values: &[String], empty: &str) -> String {
    if values.is_empty() {
        empty.to_string()
    } else {
        values.join(", ")
    }
}

fn library(cmd: LibraryCommand, json: bool) -> anyhow::Result<()> {
    let mut lib = Library::open()?;
    match cmd {
        LibraryCommand::List => {
            let entries = lib.entries();
            if json {
                return print_json(&entries);
            }
            if entries.is_empty() {
                println!("Nothing downloaded yet.");
                return Ok(());
            }
            for e in entries {
                println!("{}", e.key());
                println!("  {} {}", e.info.display_name(), e.info.version_display());
                println!("  {}", e.path.display());
                println!("  {} · added {}", human_size(e.info.file_size), e.added);
                println!();
            }
        }
        LibraryCommand::Add { file } => {
            let entry = lib.add(&file, None, false)?;
            println!("Added {} as {}", entry.info.display_name(), entry.key());
        }
        LibraryCommand::Remove { key, delete } => {
            if lib.remove(&key, delete)? {
                println!("Removed {key}");
            } else {
                anyhow::bail!("no library entry with key {key}");
            }
        }
    }
    Ok(())
}

fn print_json<T: serde::Serialize>(value: &T) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

trait VariantExt {
    fn version_display(&self) -> String;
}

impl VariantExt for Variant {
    fn version_display(&self) -> String {
        format!("{} ({})", self.version_name, self.version_code)
    }
}
