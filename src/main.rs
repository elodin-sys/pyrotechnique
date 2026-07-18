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
mod effects;
mod flight;
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
    /// Scene file, relative to the assets folder.
    #[arg(long, default_value = "scenes/falcon9.scene.ron")]
    pub scene: String,
    /// Scenario to activate on startup (defaults to the scene's first).
    #[arg(long)]
    pub scenario: Option<String>,
}

#[derive(clap::Args, Debug, Clone)]
pub struct CaptureArgs {
    /// Scene file, relative to the assets folder.
    #[arg(long, default_value = "scenes/falcon9.scene.ron")]
    pub scene: String,
    /// Scenario to capture (camera + capture time come from the scene file).
    #[arg(long)]
    pub scenario: String,
    /// Override the scenario's capture time (seconds of simulated flight).
    #[arg(long)]
    pub time: Option<f32>,
    /// Output PNG path.
    #[arg(long, default_value = "shots/capture.png")]
    pub out: std::path::PathBuf,
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
    /// Directory to write .effect files into.
    #[arg(long, default_value = "assets/effects")]
    pub out_dir: std::path::PathBuf,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        None => app::run_edit(EditArgs {
            scene: "scenes/falcon9.scene.ron".to_string(),
            scenario: None,
        }),
        Some(Command::Edit(args)) => app::run_edit(args),
        Some(Command::Capture(args)) => app::run_capture(args),
        Some(Command::GenEffects(args)) => effects::builders::generate(&args),
    }
}
