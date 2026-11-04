//! Reads a DWG file into the entity model and writes it as JSON.
//!
//! `cargo run --example to_json -- <input.dwg> <output.json>`

use std::process::ExitCode;

use uncad_model::ToJsonOptions;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let (Some(input), Some(output)) = (args.next(), args.next()) else {
        eprintln!("usage: to_json <input.dwg> <output.json>");
        return ExitCode::from(2);
    };
    let bytes = match std::fs::read(&input) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{input}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let database = match undwg::read_database(&bytes) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{input}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let json = match database.to_json(ToJsonOptions { pretty: true }) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("{input}: {e:?}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = std::fs::write(&output, json) {
        eprintln!("{output}: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
