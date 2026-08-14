//! pyrotechnique — 3D HDR particle-effect design tool for Bevy Hanabi.
//!
//! Two ways to run:
//! - `pyrotechnique [edit]` — interactive editor (3D viewport + egui panels).
//! - `pyrotechnique capture` — deterministic headless-ish capture: step the sim
//!   to a scenario time, screenshot to PNG, optionally compose a side-by-side
//!   with a target reference image, then exit. Built for AI agent iteration.
//! - `pyrotechnique gen-effects` — regenerate the built-in `.effect` files from
//!   the Rust builders in `src/effects/builders.rs`.

mod app;
mod capture;
mod earth_env;
mod effects;
mod flight;
mod orbit;
mod project;
mod render;
mod rocket;
mod scene;
mod ui;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "pyrotechnique",
    about = "3D HDR Hanabi particle effect design tool",
    version
)]
struct Cli {
    /// Project to open when no subcommand is given
    /// (resolves to assets/scenes/<project>.scene.ron).
    #[arg(long, default_value = "falcon9")]
    project: String,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Open the interactive editor (default when no subcommand is given).
    Edit(EditArgs),
    /// Deterministically simulate a scenario and write a screenshot, then exit.
    Capture(CaptureArgs),
    /// Regenerate built-in .effect files from the Rust builders.
    GenEffects(GenEffectsArgs),
}

#[derive(clap::Args, Debug, Clone)]
pub struct EditArgs {
    /// Project to open (scene at assets/scenes/<project>.scene.ron).
    #[arg(default_value = "falcon9")]
    pub project: String,
    /// Scenario to activate on startup (defaults to the scene's first).
    #[arg(long)]
    pub scenario: Option<String>,
}

#[derive(clap::Args, Debug, Clone)]
pub struct CaptureArgs {
    /// Project to capture from (scene at assets/scenes/<project>.scene.ron).
    #[arg(long, default_value = "falcon9")]
    pub project: String,
    /// Scenario to capture (camera + capture time come from the scene file).
    #[arg(long)]
    pub scenario: String,
    /// Override the scenario's capture time (seconds of simulated flight).
    #[arg(long)]
    pub time: Option<f32>,
    /// Output PNG path. Defaults to shots/<project>/<scenario>.png.
    #[arg(long)]
    pub out: Option<std::path::PathBuf>,
    /// Also write a side-by-side composite against this reference image.
    /// Pass a path, or "auto" to use the scenario's reference from the scene file.
    #[arg(long)]
    pub compare: Option<String>,
    /// Render size WxH.
    #[arg(long, default_value = "1600x900")]
    pub size: String,
    /// PRNG seed applied to all effects for reproducible particles.
    #[arg(long, default_value_t = 42)]
    pub seed: u32,
    /// Fixed simulation step rate in Hz.
    #[arg(long, default_value_t = 60.0)]
    pub fps: f64,
}

#[derive(clap::Args, Debug, Clone)]
pub struct GenEffectsArgs {
    /// Root directory for .effect output; files land in <out-dir>/<project>/.
    #[arg(long, default_value = "assets/effects")]
    pub out_dir: std::path::PathBuf,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        None => app::run_edit(EditArgs {
            project: cli.project,
            scenario: None,
        }),
        Some(Command::Edit(args)) => app::run_edit(args),
        Some(Command::Capture(args)) => app::run_capture(args),
        Some(Command::GenEffects(args)) => effects::builders::generate(&args),
    }
}
