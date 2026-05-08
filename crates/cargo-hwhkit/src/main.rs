use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

mod dev;

#[derive(Parser, Debug)]
#[command(author, version, about = "HwhKit project scaffolding & maintenance")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Init {
        name: String,
        #[arg(long, default_value = "minimal-api")]
        template: TemplateKind,
    },
    /// Manage SQL migrations (currently just scaffolds files; the running
    /// service applies them via `integrations.sql.postgres.migrations`).
    Migrate {
        #[command(subcommand)]
        action: MigrateAction,
    },
    /// Manage local development infrastructure (Postgres, Redis, NATS, etc).
    /// Reads `hwhkit.toml` (or falls back to defaults) and shells out to
    /// `docker compose` against an auto-generated compose file.
    Dev {
        #[command(subcommand)]
        action: DevAction,
    },
}

#[derive(Subcommand, Debug)]
enum DevAction {
    /// Bring up dependency containers and tail their logs.
    ///
    /// With `--detach`, returns immediately after `docker compose up -d`
    /// completes; this command does **not** follow logs in detached
    /// mode. Use `docker compose logs -f` (or run again without
    /// `--detach`) to tail.
    Up {
        #[arg(long, default_value = "hwhkit.toml")]
        config: PathBuf,
        #[arg(long)]
        detach: bool,
    },
    /// Stop and remove dependency containers.
    Down {
        #[arg(long, default_value = "hwhkit.toml")]
        config: PathBuf,
    },
    /// Show docker-compose status for the dev stack.
    Status {
        #[arg(long, default_value = "hwhkit.toml")]
        config: PathBuf,
    },
    /// Generate `docker-compose.yml` to disk without starting anything.
    Generate {
        #[arg(long, default_value = "hwhkit.toml")]
        config: PathBuf,
        #[arg(long, default_value = "docker-compose.hwhkit.yml")]
        out: PathBuf,
    },
}

#[derive(Subcommand, Debug)]
enum MigrateAction {
    /// Create a new timestamped migration file under the given directory.
    Create {
        name: String,
        #[arg(long, default_value = "./migrations")]
        dir: PathBuf,
    },
    /// List migration files in the directory.
    List {
        #[arg(long, default_value = "./migrations")]
        dir: PathBuf,
    },
    /// Print a hint on how to run migrations from the live service.
    Run {
        #[arg(long, default_value = "./migrations")]
        dir: PathBuf,
    },
    /// Print a hint on how to revert (sqlx-style down migrations).
    Revert {
        #[arg(long, default_value = "./migrations")]
        dir: PathBuf,
    },
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum TemplateKind {
    /// Minimal HTTP API skeleton with all integrations enabled.
    ///
    /// In 0.6 we removed the `api-grpc` and `realtime-event` templates
    /// because the `transport-grpc` and `transport-ws` feature flags they
    /// referenced no longer exist. Pick `minimal-api` and add the bits
    /// you need by hand — the templates were deleted rather than left
    /// pointing at non-existent flags.
    MinimalApi,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { name, template } => init_project(&name, template),
        Commands::Migrate { action } => run_migrate(action),
        Commands::Dev { action } => run_dev(action),
    }
}

fn run_dev(action: DevAction) -> Result<()> {
    match action {
        DevAction::Up { config, detach } => {
            let stack = dev::DevStack::load(&config)?;
            let compose = dev::write_compose(&stack)?;
            let mut cmd = Command::new("docker");
            cmd.arg("compose").arg("-f").arg(&compose).arg("up");
            if detach {
                cmd.arg("-d");
            }
            run_streaming(cmd)?;
            Ok(())
        }
        DevAction::Down { config } => {
            let stack = dev::DevStack::load(&config)?;
            let compose = dev::write_compose(&stack)?;
            let mut cmd = Command::new("docker");
            cmd.arg("compose")
                .arg("-f")
                .arg(&compose)
                .arg("down")
                .arg("--remove-orphans");
            run_streaming(cmd)?;
            Ok(())
        }
        DevAction::Status { config } => {
            let stack = dev::DevStack::load(&config)?;
            let compose = dev::write_compose(&stack)?;
            let mut cmd = Command::new("docker");
            cmd.arg("compose").arg("-f").arg(&compose).arg("ps");
            run_streaming(cmd)?;
            Ok(())
        }
        DevAction::Generate { config, out } => {
            let stack = dev::DevStack::load(&config)?;
            let body = dev::render_compose(&stack);
            fs::write(&out, body).with_context(|| format!("failed to write {}", out.display()))?;
            println!("wrote {}", out.display());
            Ok(())
        }
    }
}

fn run_streaming(cmd: Command) -> Result<()> {
    // Run the child process in a tokio runtime so we can race a
    // ctrl_c() future against `wait`. On SIGINT we forward to the
    // child's process group (Unix) or send a CTRL_BREAK_EVENT
    // equivalent (Windows) — without this, hitting Ctrl+C against
    // `cargo hwhkit dev up` left the docker compose stack running. (N12.)
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime for dev command")?;
    rt.block_on(run_streaming_async(cmd))
}

async fn run_streaming_async(cmd: Command) -> Result<()> {
    let mut tokio_cmd = tokio::process::Command::from(cmd);
    #[cfg(unix)]
    {
        // Place the child in its own process group so a single signal
        // forwards to every grandchild docker spawns (compose, etc.).
        tokio_cmd.process_group(0);
    }
    let mut child = tokio_cmd
        .spawn()
        .with_context(|| "failed to spawn docker; is the docker CLI on PATH?".to_string())?;
    let pid = child.id();

    let status = tokio::select! {
        status = child.wait() => status.context("waiting on docker child failed")?,
        _ = tokio::signal::ctrl_c() => {
            forward_interrupt(pid);
            // Give the child a moment to drain, then enforce.
            child.wait().await.context("waiting on docker child failed")?
        }
    };

    if !status.success() {
        anyhow::bail!("docker command failed with status: {status}");
    }
    Ok(())
}

#[cfg(unix)]
fn forward_interrupt(pid: Option<u32>) {
    if let Some(pid) = pid {
        // Negative pid → kill the entire process group. We placed the
        // child in its own group via process_group(0) above, so this
        // also reaches docker compose's children.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGINT);
        }
    }
}

#[cfg(windows)]
fn forward_interrupt(_pid: Option<u32>) {
    // Windows lacks a clean equivalent of "send SIGINT to a process
    // group". We fall back to letting the child see the console
    // ctrl-c that the OS already delivered to it; tokio's spawn does
    // not detach the console handle by default. If users hit Ctrl+C
    // and the docker child does not exit, they can `docker compose
    // down` manually — surface that hint so the operator knows what
    // to do without spelunking the source.
    eprintln!(
        "\nInterrupted. If containers are still running, use `cargo hwhkit dev down` to stop them."
    );
}

fn run_migrate(action: MigrateAction) -> Result<()> {
    match action {
        MigrateAction::Create { name, dir } => {
            fs::create_dir_all(&dir)
                .with_context(|| format!("create migrations dir {}", dir.display()))?;
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| anyhow!("clock error: {e}"))?
                .as_secs();
            let safe = name.replace(|c: char| !c.is_alphanumeric() && c != '_', "_");
            let up = dir.join(format!("{ts}_{safe}.sql"));
            let down = dir.join(format!("{ts}_{safe}.down.sql"));
            fs::write(&up, "-- write your forward migration here\n")?;
            fs::write(&down, "-- write your revert migration here\n")?;
            println!("created {} and {}", up.display(), down.display());
            Ok(())
        }
        MigrateAction::List { dir } => {
            if !dir.exists() {
                println!("(no migrations directory at {})", dir.display());
                return Ok(());
            }
            let mut entries: Vec<_> = fs::read_dir(&dir)?
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.file_name()
                        .to_str()
                        .map(|n| n.ends_with(".sql"))
                        .unwrap_or(false)
                })
                .collect();
            entries.sort_by_key(|e| e.file_name());
            for e in entries {
                println!("{}", e.file_name().to_string_lossy());
            }
            Ok(())
        }
        MigrateAction::Run { dir } => {
            println!(
                "configure your service with `[integrations.sql.postgres.migrations]\\nrun_on_start = true\\npath = \"{}\"` to apply migrations on bootstrap.",
                dir.display()
            );
            Ok(())
        }
        MigrateAction::Revert { dir } => {
            println!(
                "down migrations live next to up migrations in {} (e.g. `*.down.sql`). Apply them via your DB client; sqlx::Migrator does not auto-revert.",
                dir.display()
            );
            Ok(())
        }
    }
}

fn init_project(name: &str, template: TemplateKind) -> Result<()> {
    let root = PathBuf::from(name);
    let package_name = package_name_from_root(&root)?;
    create_common_layout(&root)?;

    match template {
        TemplateKind::MinimalApi => render_minimal_api(&root, &package_name)?,
    }

    write_file(
        &root.join("README.md"),
        &format!(
            "# {package_name}\n\nGenerated by `cargo hwhkit init --template {}`.\n",
            template_name(template)
        ),
    )?;

    println!("initialized project: {}", root.display());
    println!("template: {}", template_name(template));
    Ok(())
}

fn create_common_layout(root: &Path) -> Result<()> {
    fs::create_dir_all(root.join("src/routes"))?;
    fs::create_dir_all(root.join("src/domain"))?;
    fs::create_dir_all(root.join("config"))?;
    fs::create_dir_all(root.join("tests"))?;
    fs::create_dir_all(root.join(".github/workflows"))?;

    write_file(
        &root.join(".env.example"),
        "HWHKIT__SERVER__PORT=3000\nHWHKIT__OBSERVABILITY__LOGGING__LEVEL=info\n",
    )?;
    write_file(
        &root.join("Dockerfile"),
        "FROM rust:1.84 as builder\nWORKDIR /app\nCOPY . .\nRUN cargo build --release\n\nFROM debian:bookworm-slim\nCOPY --from=builder /app/target/release/APP_NAME /usr/local/bin/app\nCMD [\"/usr/local/bin/app\"]\n",
    )?;
    write_file(
        &root.join("justfile"),
        "run:\n    cargo run\n\ntest:\n    cargo test\n\nfmt:\n    cargo fmt --all\n",
    )?;
    write_file(
        &root.join(".github/workflows/ci.yml"),
        "name: ci\non: [push, pull_request]\njobs:\n  test:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v4\n      - uses: dtolnay/rust-toolchain@stable\n      - run: cargo test --all\n",
    )?;
    write_file(
        &root.join("tests/smoke.rs"),
        "#[test]\nfn smoke() {\n    assert!(true);\n}\n",
    )?;

    Ok(())
}

fn render_minimal_api(root: &Path, name: &str) -> Result<()> {
    // Generated services pull in `postgres` + `redis` by default — these
    // are by far the most common picks and keep the smoke template
    // useful out of the box. Other integrations are one Cargo.toml line
    // away. We deliberately avoid `config-v2` (now the default; not a
    // separate flag in 0.6+) and the deleted `transport-grpc` /
    // `transport-ws` flags.
    write_file(
        &root.join("Cargo.toml"),
        &cargo_toml(name, &["postgres", "redis"]),
    )?;
    write_main(root)?;
    write_standard_app(root)?;
    write_file(&root.join("src/routes/mod.rs"), "pub mod health;\n")?;
    write_file(
        &root.join("src/routes/health.rs"),
        "pub async fn health() -> &'static str {\n    \"ok\"\n}\n",
    )?;
    write_file(&root.join("src/domain/mod.rs"), "")?;
    write_config_files(root, "dev")?;
    patch_docker_app_name(root, name)?;
    Ok(())
}

fn write_main(root: &Path) -> Result<()> {
    write_file(
        &root.join("src/main.rs"),
        "mod app;\n\nuse hwhkit::{config::BootstrapConfig, run};\n\n#[tokio::main]\nasync fn main() {\n    let bootstrap = BootstrapConfig::default();\n\n    match run(app::App, bootstrap).await {\n        Ok(built) => {\n            println!(\"bootstrap completed\");\n            println!(\"applied sources: {:?}\", built.applied_sources());\n            println!(\"initialized integrations: {:?}\", built.initialized_integrations());\n            println!(\"degraded integrations: {:?}\", built.degraded_integrations());\n            println!(\"next: wire router/server runtime in app-specific entrypoint\");\n        }\n        Err(err) => {\n            eprintln!(\"bootstrap failed: {err}\");\n            std::process::exit(1);\n        }\n    }\n}\n",
    )
}

fn write_standard_app(root: &Path) -> Result<()> {
    write_file(
        &root.join("src/app.rs"),
        "use async_trait::async_trait;\nuse axum::{routing::get, Router};\nuse hwhkit::{\n    config::AppConfig,\n    AppContext, Application, Result,\n};\n\npub struct App;\n\n#[async_trait]\nimpl Application for App {\n    async fn build_router(&self, _ctx: AppContext, _cfg: &AppConfig) -> Result<Router> {\n        Ok(Router::new().route(\"/healthz\", get(health)))\n    }\n}\n\nasync fn health() -> &'static str {\n    \"ok\"\n}\n",
    )
}

fn write_config_files(root: &Path, env_name: &str) -> Result<()> {
    write_file(
        &root.join("config/default.toml"),
        "[server]\nhost = \"0.0.0.0\"\nport = 3000\n\n[observability]\nservice_name = \"app\"\nenvironment = \"dev\"\n\n[observability.logging]\nlevel = \"info\"\nformat = \"pretty\"\n",
    )?;
    write_file(&root.join(format!("config/{env_name}.toml")), "")?;
    write_file(&root.join("config/prod.toml"), "")?;
    Ok(())
}

fn cargo_toml(name: &str, features: &[&str]) -> String {
    let features = features
        .iter()
        .map(|f| format!("\"{f}\""))
        .collect::<Vec<_>>()
        .join(", ");

    // Pin the generated template to the *same* workspace version as the
    // CLI binary that scaffolded it. Hard-coding "0.2" here became stale
    // the moment we shipped 0.4 — `env!` keeps the two in lockstep.
    let hwhkit_version = env!("CARGO_PKG_VERSION");

    format!(
        "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nasync-trait = \"0.1\"\nhwhkit = {{ version = \"{hwhkit_version}\", features = [{features}] }}\ntokio = {{ version = \"1\", features = [\"full\"] }}\n"
    )
}

fn patch_docker_app_name(root: &Path, name: &str) -> Result<()> {
    let dockerfile_path = root.join("Dockerfile");
    let current = fs::read_to_string(&dockerfile_path)
        .with_context(|| format!("failed to read {}", dockerfile_path.display()))?;
    let patched = current.replace("APP_NAME", name);
    write_file(&dockerfile_path, &patched)
}

fn package_name_from_root(root: &Path) -> Result<String> {
    let Some(file_name) = root.file_name().and_then(|v| v.to_str()) else {
        anyhow::bail!("invalid project path: {}", root.display());
    };

    let normalized = file_name.replace('_', "-");
    if normalized.trim().is_empty() {
        anyhow::bail!("project name cannot be empty");
    }

    Ok(normalized)
}

fn write_file(path: &Path, content: &str) -> Result<()> {
    fs::write(path, content).with_context(|| format!("failed to write {}", path.display()))
}

fn template_name(template: TemplateKind) -> &'static str {
    match template {
        TemplateKind::MinimalApi => "minimal-api",
    }
}
