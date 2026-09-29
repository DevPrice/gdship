//! Plays godot, butler or gdget, chosen by its file name or `FAKE_ROLE`.
//!
//! Every run appends its role and arguments, tab-separated, to the file in `FAKE_LOG`.
//! Behavior is steered by environment variables; lists are comma-separated:
//!
//! - `FAKE_GODOT_VERSION`: what `godot --version` prints.
//! - `FAKE_GODOT_FAIL`: presets whose export exits 1.
//! - `FAKE_GODOT_SILENT`: presets whose export exits 0 without writing anything.
//! - `FAKE_GODOT_IMPORT_EXIT`: exit code of `godot --import`.
//! - `FAKE_BUTLER_FAIL`: channels whose push exits 1.
//! - `FAKE_GDGET_EXIT`: exit code of `gdget sync --check`.

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let role = std::env::var("FAKE_ROLE").unwrap_or_else(|_| {
        let exe = std::env::current_exe().unwrap();
        exe.file_stem().unwrap().to_string_lossy().into_owned()
    });
    if let Ok(log) = std::env::var("FAKE_LOG") {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .unwrap();
        writeln!(file, "{role}\t{}", args.join("\t")).unwrap();
    }
    match role.as_str() {
        "godot" => godot(&args),
        "butler" => butler(&args),
        "gdget" => code_from("FAKE_GDGET_EXIT"),
        other => {
            eprintln!("fake-tool: unknown role {other}");
            ExitCode::from(101)
        }
    }
}

fn listed(var: &str, item: &str) -> bool {
    std::env::var(var).is_ok_and(|list| list.split(',').any(|i| i == item))
}

fn code_from(var: &str) -> ExitCode {
    let code = std::env::var(var).map_or(0, |c| c.parse().unwrap());
    ExitCode::from(code)
}

fn godot(args: &[String]) -> ExitCode {
    if args.iter().any(|a| a == "--version") {
        let version = std::env::var("FAKE_GODOT_VERSION")
            .unwrap_or_else(|_| "4.7.2.stable.official.abc1234".to_owned());
        println!("{version}");
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--import") {
        println!("Godot Engine v4.7.2.stable.official - https://godotengine.org");
        eprintln!("importing on stderr");
        return code_from("FAKE_GODOT_IMPORT_EXIT");
    }
    let Some(i) = args.iter().position(|a| a == "--export-release") else {
        eprintln!("fake godot: unexpected arguments {args:?}");
        return ExitCode::from(101);
    };
    let (preset, out) = (&args[i + 1], Path::new(&args[i + 2]));
    for n in 1..=50 {
        println!("export line {n} for {preset}");
    }
    if listed("FAKE_GODOT_FAIL", preset) {
        eprintln!("ERROR: Project export for preset \"{preset}\" failed.");
        return ExitCode::FAILURE;
    }
    if listed("FAKE_GODOT_SILENT", preset) {
        eprintln!("ERROR: export failed, but exiting 0 anyway");
        return ExitCode::SUCCESS;
    }
    std::fs::write(out, format!("built {preset}")).unwrap();
    if out.file_name().is_some_and(|n| n == "index.html") {
        std::fs::write(out.with_extension("wasm"), "wasm").unwrap();
        std::fs::write(out.with_extension("pck"), "pck").unwrap();
    }
    ExitCode::SUCCESS
}

fn butler(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("push") => {
            let target = &args[2];
            let channel = target.rsplit(':').next().unwrap();
            if listed("FAKE_BUTLER_FAIL", channel) {
                eprintln!("fake butler: push to {target} failed");
                return ExitCode::FAILURE;
            }
            println!("pushed {} to {target}", args[1]);
        }
        Some("status") => println!("status of {}", args[1]),
        Some("login") => println!("logged in"),
        _ => {
            eprintln!("fake butler: unexpected arguments {args:?}");
            return ExitCode::from(101);
        }
    }
    ExitCode::SUCCESS
}
