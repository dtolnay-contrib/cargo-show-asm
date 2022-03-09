use cargo_show_asm::*;

use std::collections::BTreeSet;

use cargo::{
    core::{
        compiler::{CompileKind, TargetInfo},
        Workspace,
    },
    ops::{compile, CleanOptions, CompileFilter, CompileOptions, Packages},
    util::interning::InternedString,
    Config,
};

/// This should be called before calling any cli method or printing any output.
pub fn reset_signal_pipe_handler() -> anyhow::Result<()> {
    #[cfg(target_family = "unix")]
    {
        use nix::sys::signal;
        unsafe {
            signal::signal(signal::Signal::SIGPIPE, signal::SigHandler::SigDfl)?;
        }
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    reset_signal_pipe_handler()?;

    let opts = opts::opts();

    let mut cfg = Config::default()?;
    cfg.configure(
        opts.verbosity as u32,
        false,
        None,
        opts.frozen,
        opts.locked,
        opts.offline,
        &None,
        &[],
        &[],
    )?;

    let ws = Workspace::new(&opts.manifest, &cfg)?;

    let package = opts::select_package(&opts, &ws);

    let rustc = cfg.load_global_rustc(Some(&ws))?;
    let target_info = TargetInfo::new(&cfg, &[CompileKind::Host], &rustc, CompileKind::Host)?;

    let mut copts = CompileOptions::new(&cfg, cargo::core::compiler::CompileMode::Build)?;

    copts.spec = Packages::Packages(vec![package.clone()]);

    if let Some(focus) = opts.focus {
        copts.filter = CompileFilter::from(focus);
    }
    copts.build_config.requested_profile = InternedString::new("release");
    copts.target_rustc_args = Some(vec![
        String::from("-C"),
        String::from("codegen-units=1"),
        String::from("--emit"),
        String::from("asm"),
        String::from("-C"),
        String::from("llvm-args=-x86-asm-syntax=intel"),
        String::from("-C"),
        String::from("debuginfo=2"),
    ]);

    copts.build_config.build_plan = opts.dry;

    let mut retrying = false;
    loop {
        let comp = compile(&ws, &copts)?;
        let output = comp.deps_output.get(&CompileKind::Host).unwrap();

        let target = opts.function.as_deref().unwrap_or(" ");

        let seen;
        let mut existing = BTreeSet::new();
        let mut asm_files = glob::glob(&format!(
            "{}/{}-*.s",
            output.display(),
            &comp.root_crate_names[0]
        ))?
        .collect::<Vec<_>>();

        match asm_files.len() {
            0 => {
                eprintln!(
                    "Compilation produced no files satisfying {}/{}-*.s, this is a bug",
                    output.display(),
                    &comp.root_crate_names[0]
                );
                std::process::exit(1);
            }
            1 => {
                let file = asm_files.remove(0)?;
                seen = asm::dump_function(
                    target,
                    &file,
                    &target_info.sysroot,
                    &opts.format,
                    &mut existing,
                )?;
            }
            _ => {
                if retrying {
                    eprintln!(
                        "Compilation produced multiple matching files: {:?}, this is a bug",
                        asm_files
                    );
                    std::process::exit(1);
                }
                let opts = CleanOptions {
                    config: &cfg,
                    spec: vec![package.clone()],
                    targets: Vec::new(),
                    profile_specified: false,
                    requested_profile: InternedString::new("release"),
                    doc: false,
                };
                cargo::ops::clean(&ws, &opts)?;
                retrying = true;
                continue;
            }
        }

        if !seen {
            println!("Try one of those");
            for x in existing.iter() {
                println!("\t{x}");
            }
        }
        break;
    }

    Ok(())
}
