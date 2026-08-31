//! OpenAPI → IR compiler. This crate is the D5 language gate.

mod category;
mod compile;
mod destructive;
mod error;
mod http;
mod loader;
mod lower;
mod names;
mod normalize;
mod parse;
mod refs;
mod safety;
mod style;

pub use compile::{compile, compile_loaded, compile_path, compile_with, CompileOptions};
pub use error::CompileError;
pub use loader::{load, load_bytes, load_file, LoadedSpec, OpenApiFamily, SpecFormat, SpecSource};
pub use names::{candidate_name, normalize as normalize_name, uniquify, NameSource};
pub use safety::{
    check_host, is_blocked_ip, parse_https_url, resolve_and_check, SafetyError, SafetyOpts,
};

pub use mcp_gateway_ir::{CompileBundle, IR_VERSION};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_compiles() {}
}
