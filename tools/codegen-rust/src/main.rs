//! Code Generation tool for Rust.
//!
//! This tool takes an input file (e.g. manifest.json) with a json formatted
//! schema, and outputs a file (e.g. generated.rs) with Rust code which can
//! then be used to read/write messages in that schema.
//!
//! See [`../README.md`]

use std::{fs::write, path::PathBuf};

use clap::Parser;
use codegen_rust::{generate_file_string, parse_input_from_path};
use convert_case::{Case, Casing};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    additional_events: Vec<String>,

    #[arg(short, long)]
    input: String,

    #[arg(short, long)]
    output: Option<String>,
}

fn main() {
    let args = Args::parse();
    let input = PathBuf::from(args.input);

    let platform_library = match parse_input_from_path(input.as_path()) {
        Ok(platform_library) => platform_library,
        Err(err) => panic!("Could not parse input: {err}"),
    };
    let additional_events = args.additional_events;
    let additional_events_str = additional_events
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let generated_string = match generate_file_string(
        &platform_library,
        if additional_events.is_empty() {
            None
        } else {
            Some(additional_events_str.as_slice())
        },
    ) {
        Ok(generated_string) => generated_string,
        Err(err) => panic!("Could not generate file string: {err}"),
    };

    // store output path

    let output_path = args.output.map_or_else(
        || {
            input
                .with_file_name(platform_library.name().to_case(Case::Snake))
                .with_extension("rs")
        },
        PathBuf::from,
    );

    write(output_path, generated_string).expect("error writing generated .rs file");
}
