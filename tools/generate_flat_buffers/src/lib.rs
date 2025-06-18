//! Convert *.fbs (`FlatBuffers`) files into generated code.
//!
//! Runs external program `flatc`.

use std::{
    borrow::Cow,
    env::var_os,
    ffi::OsStr,
    fs::{self},
    io,
    path::{Path, PathBuf},
    process::Command,
};

const FLATC_PATH: &str = "flatc";

/// This is not a complete list of the output types that `flatc` supports. It's
/// just those that we've needed so far. Feel free to add to this list.
#[derive(Default)]
enum OutputType {
    #[default]
    Rust,
    TypeScript,
}

/// Generates (transpiles) the `FlatBuffers` input to Rust code.
///
/// It is a wrapper around `generate_flat_buffers()` with default values:
/// - `out_dir` is set to the cargo `OUT_DIR` env var (i.e. in `target/...`
///   somewhere).
/// - `input_path` is set to the first file found in `src/*.fbs`.
///
/// Usage: `build_tools::FfiBuilder::new().write();`
#[derive(Default)]
pub struct GenerateFlatBuffers<'a> {
    output_type: OutputType,
    input_path: Option<Cow<'a, Path>>,
    out_dir: Option<Cow<'a, Path>>,
}

impl<'a> GenerateFlatBuffers<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the output type to TypeScript.
    ///
    /// Note: if not called, the default output type is Rust.
    pub fn type_script(mut self) -> Self {
        self.output_type = OutputType::TypeScript;
        self
    }

    /// A path to a directory for storing output.
    ///
    /// Must be a writable directory (not a file path).
    pub fn out_dir(mut self, out_dir: &'a Path) -> Self {
        self.out_dir = Some(Cow::from(out_dir));
        self
    }

    /// A path to a directory containing *.fbs or a direct path to a file.
    pub fn in_path(mut self, in_path: &'a Path) -> Self {
        //self.input_path = Some(in_path);
        self.input_path = Some(Cow::from(in_path));
        self
    }

    /// The "build" function of this builder.
    ///
    /// If this call fails, it will panic and display an error (which is not good
    /// practice in general, but is sensible in a build script).
    pub fn write(self) {
        let out_dir = self
            .out_dir
            .unwrap_or_else(|| Cow::from(PathBuf::from(var_os("OUT_DIR").unwrap())));

        let input_path = self
            .input_path
            .unwrap_or_else(|| Cow::from(PathBuf::from("./src")));

        let fbs_files = find_fbs_files(input_path).expect("Finding fbs file");

        let output_type = match self.output_type {
            OutputType::Rust => "--rust",
            OutputType::TypeScript => "--ts",
        };

        for input_path in &fbs_files {
            run_flatc(&out_dir, input_path, output_type);
        }
    }
}

/// List all the *.fbs files in `directory`.
///
/// If the input is a file path, return it as the only entry in the vector (this
/// simplifies usage, the output is always a vector).
fn find_fbs_files<P>(directory_or_file: P) -> io::Result<Vec<PathBuf>>
where
    P: Into<PathBuf>,
{
    let in_path = directory_or_file.into();
    let mut result = vec![];
    if in_path.is_dir() {
        for entry in fs::read_dir(in_path)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() && path.extension() == Some(OsStr::new("fbs")) {
                result.push(path.clone());
            }
        }
    } else {
        assert!(in_path.is_file());
        result.push(in_path.clone());
    }
    Ok(result)
}

/// Generates the events Rust code.
fn run_flatc(out_dir: &Path, fbs_path: &Path, output_type: &str) {
    let status = Command::new(FLATC_PATH)
        .arg(output_type)
        .arg("-o")
        .arg(out_dir)
        .arg(fbs_path)
        .arg("--gen-object-api")
        .arg("--gen-name-strings")
        .spawn()
        .unwrap_or_else(|e| panic!("Running {FLATC_PATH} flatbuffer compiler: {e}"))
        .wait()
        .expect("Getting exit status from flatc");
    assert!(status.success());
}
