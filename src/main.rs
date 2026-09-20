use std::{error::Error, path::PathBuf, process::ExitCode};

use clap::Parser;
use fuser::{Config, MountOption, SessionACL};
use kube::Client;
use tokio::runtime::Runtime;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(version, about)]
struct Args {
    /// Directory where the Kubernetes filesystem will be mounted.
    #[arg(value_name = "MOUNT_POINT")]
    mount_point: PathBuf,
}

type AppResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

fn main() -> ExitCode {
    let args = Args::parse();

    if let Err(err) = init_tracing() {
        eprintln!("failed to initialize tracing: {err}");
        return ExitCode::FAILURE;
    }

    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!(
                mount_point = %args.mount_point.display(),
                error = %err,
                "application failed",
            );

            ExitCode::FAILURE
        }
    }
}

fn run(args: &Args) -> AppResult<()> {
    let runtime = Runtime::new()?;

    let mut config = Config::default();
    config.mount_options.push(MountOption::RO);
    config.acl = SessionACL::RootAndOwner;

    let client = runtime.block_on(Client::try_default())?;
    let filesystem = kubefs::new_from_kube_client(client, runtime.handle().clone());

    let session = fuser::spawn_mount(filesystem, &args.mount_point, &config)?;

    tracing::info!(
        mount_point = %args.mount_point.display(),
        "filesystem mounted",
    );

    runtime.block_on(tokio::signal::ctrl_c())?;

    tracing::info!(
        mount_point = %args.mount_point.display(),
        "shutdown signal received",
    );

    session.umount_and_join()?;

    tracing::info!(
        mount_point = %args.mount_point.display(),
        "filesystem unmounted",
    );

    Ok(())
}

fn init_tracing() -> AppResult<()> {
    let default_directive = "kubefs=info".parse()?;

    let filter = EnvFilter::builder()
        .with_default_directive(default_directive)
        .from_env()?;

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init()?;

    Ok(())
}
