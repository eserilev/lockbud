//! The general rustc plugin framework.
//! Inspired by <https://github.com/facebookexperimental/MIRAI/blob/9cf3067309d591894e2d0cd9b1ee6e18d0fdd26c/checker/src/main.rs>
#![feature(rustc_private)]
#![feature(box_patterns)]

extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_session;


mod analysis;
mod callbacks;
mod detector;
mod interest;
mod options;

use log::debug;
use options::Options;
use rustc_session::config::ErrorOutputType;
use rustc_session::early_error;

fn main() {
    // Initialize loggers.
    if std::env::var("RUSTC_LOG").is_ok() {
        rustc_driver::init_rustc_env_logger();
    }
    if std::env::var("LOCKBUD_LOG").is_ok() {
        let e = env_logger::Env::new()
            .filter("LOCKBUD_LOG")
            .write_style("LOCKBUD_LOG_STYLE");
        env_logger::init_from_env(e);
    }
    // Get any options specified via the LOCKBUD_FLAGS environment variable
    let options = Options::parse_from_str(&std::env::var("LOCKBUD_FLAGS").unwrap_or_default())
        .unwrap_or_default();
    debug!("LOCKBUD options from environment: {:?}", options);
    let mut args = std::env::args_os()
        .enumerate()
        .map(|(i, arg)| {
            arg.into_string().unwrap_or_else(|arg| {
                early_error(
                    ErrorOutputType::default(),
                    &format!("Argument {} is not valid Unicode: {:?}", i, arg),
                )
            })
        })
        .collect::<Vec<_>>();
    assert!(!args.is_empty());

    // Setting RUSTC_WRAPPER causes Cargo to pass 'rustc' as the first argument.
    // We're invoking the compiler programmatically, so we remove it if present.
    if args.len() > 1 && std::path::Path::new(&args[1]).file_stem() == Some("rustc".as_ref()) {
        args.remove(1);
    }

    let mut rustc_command_line_arguments: Vec<String> = args[1..].into();
    rustc_driver::install_ice_hook();
    let result = rustc_driver::catch_fatal_errors(|| {
        // Add back the binary name
        rustc_command_line_arguments.insert(0, args[0].clone());

        let print: String = "--print=".into();
        if rustc_command_line_arguments
            .iter()
            .any(|arg| arg.starts_with(&print))
        {
            // If a --print option is given on the command line we wont get called to analyze
            // anything. We also don't want to the caller to know that LOCKBUD adds configuration
            // parameters to the command line, lest the caller be cargo and it panics because
            // the output from --print=cfg is not what it expects.
        } else {
            let sysroot: String = "--sysroot".into();
            if !rustc_command_line_arguments
                .iter()
                .any(|arg| arg.starts_with(&sysroot))
            {
                // Tell compiler where to find the std library and so on.
                // The compiler relies on the standard rustc driver to tell it, so we have to do likewise.
                rustc_command_line_arguments.push(sysroot);
                rustc_command_line_arguments.push(find_sysroot());
            }

            let always_encode_mir: String = "always-encode-mir".into();
            if !rustc_command_line_arguments
                .iter()
                .any(|arg| arg.ends_with(&always_encode_mir))
            {
                // Tell compiler to emit MIR into crate for every function with a body.
                rustc_command_line_arguments.push("-Z".into());
                rustc_command_line_arguments.push(always_encode_mir);
            }
        }

        let mut callbacks = callbacks::LockBudCallbacks::new(options);
        debug!(
            "rustc_command_line_arguments {:?}",
            rustc_command_line_arguments
        );
        let compiler =
            rustc_driver::RunCompiler::new(&rustc_command_line_arguments, &mut callbacks);
        compiler.run()
    })
    .and_then(|result| result);
    let exit_code = match result {
        Ok(_) => rustc_driver::EXIT_SUCCESS,
        Err(_) => rustc_driver::EXIT_FAILURE,
    };
    std::process::exit(exit_code);
}

fn find_sysroot() -> String {
    let home = option_env!("RUSTUP_HOME");
    let toolchain = option_env!("RUSTUP_TOOLCHAIN");
    match (home, toolchain) {
        (Some(home), Some(toolchain)) => format!("{}/toolchains/{}", home, toolchain),
        _ => option_env!("RUST_SYSROOT")
            .expect(
                "Could not find sysroot. Specify the RUST_SYSROOT environment variable, \
                 or use rustup to set the compiler to use for LOCKBUD",
            )
            .to_owned(),
    }
}
