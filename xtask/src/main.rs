// Copyright 2021  The RXV64 Authors
// All rights reserved
//
// Use of this source code is governed by an MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT.

use std::{
    env,
    path::{Path, PathBuf},
    process::{self, Command},
};

type DynError = Box<dyn std::error::Error>;
type Result<T> = std::result::Result<T, DynError>;

#[derive(Clone, Copy)]
enum Build {
    Debug,
    Release,
}

impl Build {
    fn dir(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Release => "release",
        }
    }

    fn add_build_arg(self, cmd: &mut Command) {
        if let Self::Release = self {
            cmd.arg("--release");
        }
    }
}

fn main() {
    let matches = clap::Command::new("xtask")
        .version("0.1.0")
        .author("The RXV64 Authors")
        .about("Build support for the RXV64 system")
        .subcommand(
            clap::Command::new("build")
                .about("Builds RXV64, syslib and ulib")
                .args(&[
                    clap::arg!(--release "Build release version").conflicts_with("debug"),
                    clap::arg!(--debug "Build debug version (default)").conflicts_with("release"),
                ]),
        )
        .subcommand(
            clap::Command::new("expand")
                .about("Expands RXV64 macros")
                .args(&[
                    clap::arg!(--release "Build release version").conflicts_with("debug"),
                    clap::arg!(--debug "Build debug version (default)").conflicts_with("release"),
                ]),
        )
        .subcommand(
            clap::Command::new("kasm")
                .about("Emits RXV64 assembler")
                .args(&[
                    clap::arg!(--release "Build release version").conflicts_with("debug"),
                    clap::arg!(--debug "Build debug version (default)").conflicts_with("release"),
                ]),
        )
        .subcommand(
            clap::Command::new("dist")
                .about("Builds a multibootable RXV64 image")
                .args(&[
                    clap::arg!(--release "Build a release version").conflicts_with("debug"),
                    clap::arg!(--debug "Build a debug version").conflicts_with("release"),
                ]),
        )
        .subcommand(clap::Command::new("test").about("Runs unit tests").args(&[
            clap::arg!(--release "Build a release version").conflicts_with("debug"),
            clap::arg!(--debug "Build a debug version").conflicts_with("release"),
        ]))
        .subcommand(clap::Command::new("clippy").about("Runs clippy").args(&[
            clap::arg!(--release "Build a release version").conflicts_with("debug"),
            clap::arg!(--debug "Build a debug version").conflicts_with("release"),
        ]))
        .subcommand(clap::Command::new("run").about("Run RXV64 under QEMU"))
        .subcommand(clap::Command::new("accelrun").about("Run RXV64 under QEMU"))
        .subcommand(clap::Command::new("clean").about("Cargo clean"))
        .get_matches();
    if let Err(e) = match matches.subcommand() {
        Some(("build", m)) => build(build_type(m)),
        Some(("expand", m)) => expand(build_type(m)),
        Some(("kasm", m)) => kasm(build_type(m)),
        Some(("dist", m)) => dist(build_type(m)),
        Some(("test", m)) => test(build_type(m)),
        Some(("clippy", m)) => clippy(build_type(m)),
        Some(("run", _m)) => run(),
        Some(("accelrun", _m)) => accelrun(),
        Some(("clean", _)) => clean(),
        _ => Err("bad subcommand".into()),
    } {
        eprintln!("{}", e);
        process::exit(1);
    }
}

fn build_type(matches: &clap::ArgMatches) -> Build {
    if matches.get_flag("release") {
        return Build::Release;
    }
    Build::Debug
}

fn env_or(var: &str, default: &str) -> String {
    let default = default.to_string();
    env::var(var).unwrap_or(default)
}

fn cargo() -> String {
    env_or("CARGO", "cargo")
}
fn objcopy() -> String {
    let llvm_objcopy = {
        let toolchain = env_or("RUSTUP_TOOLCHAIN", "nightly-x86_64-unknown-none");
        let pos = toolchain.find('-').map(|p| p + 1).unwrap_or(0);
        let host = toolchain[pos..].to_string();
        let home = env_or("RUSTUP_HOME", "");
        let mut path = PathBuf::from(home);
        path.push("toolchains");
        path.push(toolchain);
        path.push("lib");
        path.push("rustlib");
        path.push(host);
        path.push("bin");
        path.push("llvm-objcopy");
        if path.exists() {
            path.into_os_string().into_string().unwrap()
        } else {
            "llvm-objcopy".into()
        }
    };
    env_or("OBJCOPY", &llvm_objcopy)
}
fn qemu_system_x86_64() -> String {
    env_or("QEMU", "qemu-system-x86_64")
}
fn ktarget() -> String {
    env_or("TARGET", "x86_64-unknown-none-elf")
}
fn utarget() -> String {
    env_or("UTARGET", "x86_64-unknown-rxv64-elf")
}

fn build(profile: Build) -> Result<()> {
    kbuild(profile)?;
    ubuild(profile)?;
    Ok(())
}

fn kbuild(profile: Build) -> Result<()> {
    let mut cmd = Command::new(cargo());
    cmd.current_dir(workspace());
    cmd.arg("build");
    #[rustfmt::skip]
    cmd.arg("--workspace");
    cmd.arg("--exclude").arg("xtask");
    cmd.arg("--exclude").arg("ulib");
    cmd.arg("-Z").arg("build-std=core");
    cmd.arg("-Z").arg("json-target-spec");
    cmd.arg("--target").arg(format!("lib/{}.json", ktarget()));
    profile.add_build_arg(&mut cmd);
    let status = cmd.status()?;
    if !status.success() {
        return Err("build kernel failed".into());
    }
    Ok(())
}

fn ubuild(profile: Build) -> Result<()> {
    let mut cmd = Command::new(cargo());
    cmd.current_dir(workspace());
    cmd.arg("build");
    cmd.arg("--workspace");
    cmd.arg("--exclude").arg("xtask");
    cmd.arg("--exclude").arg("kernel");
    cmd.arg("-Z").arg("build-std=core");
    cmd.arg("-Z").arg("json-target-spec");
    cmd.arg("--target").arg(format!("lib/{}.json", utarget()));
    profile.add_build_arg(&mut cmd);
    let status = cmd.status()?;
    if !status.success() {
        return Err("build kernel failed".into());
    }
    Ok(())
}

fn expand(profile: Build) -> Result<()> {
    let mut subdir = workspace();
    subdir.push("kernel");
    let mut cmd = Command::new(cargo());
    cmd.current_dir(subdir);
    cmd.arg("rustc");
    cmd.arg("-Z").arg("build-std=core");
    cmd.arg("-Z").arg("json-target-spec");
    cmd.arg("--target")
        .arg(format!("../lib/{}.json", ktarget()));
    cmd.arg("--").arg("--pretty=expanded");
    profile.add_build_arg(&mut cmd);
    let status = cmd.status()?;
    if !status.success() {
        return Err("build kernel failed".into());
    }
    Ok(())
}

fn kasm(profile: Build) -> Result<()> {
    let mut cmd = Command::new(cargo());
    cmd.current_dir(workspace());
    cmd.arg("build");
    cmd.arg("--workspace");
    cmd.arg("--exclude").arg("xtask");
    cmd.arg("--exclude").arg("ulib");
    cmd.arg("--exclude").arg("syslib");
    cmd.arg("-Z").arg("build-std=core");
    cmd.arg("-Z").arg("json-target-spec");
    cmd.arg("--target").arg(format!("lib/{}.json", utarget()));
    cmd.arg("--").arg("--emit").arg("asm");
    profile.add_build_arg(&mut cmd);
    let status = cmd.status()?;
    if !status.success() {
        return Err("build kernel failed".into());
    }
    Ok(())
}

fn dist(profile: Build) -> Result<()> {
    build(profile)?;
    let status = Command::new(objcopy())
        .arg("--input-target=elf64-x86-64")
        .arg("--output-target=elf32-i386")
        .arg(format!("target/{}/{}/kernel", ktarget(), profile.dir()))
        .arg(format!(
            "target/{}/{}/rxv64.elf32",
            ktarget(),
            profile.dir()
        ))
        .current_dir(workspace())
        .status()?;
    if !status.success() {
        return Err("objcopy failed".into());
    }
    Ok(())
}

fn test(profile: Build) -> Result<()> {
    let mut cmd = Command::new(cargo());
    cmd.current_dir(workspace());
    cmd.arg("test");
    profile.add_build_arg(&mut cmd);
    let status = cmd.status()?;
    if !status.success() {
        return Err("test failed".into());
    }
    Ok(())
}

fn clippy(profile: Build) -> Result<()> {
    let mut cmd = Command::new(cargo());
    cmd.current_dir(workspace());
    cmd.arg("clippy");
    #[rustfmt::skip]
    cmd.arg("--workspace");
    cmd.arg("--exclude").arg("xtask");
    profile.add_build_arg(&mut cmd);
    let status = cmd.status()?;
    if !status.success() {
        return Err("build kernel failed".into());
    }
    Ok(())
}

// qemu-system-x86_64 -cpu qemu64,pdpe1gb,xsaveopt,fsgsbase,x2apic -smp 8 -m 8192 -curses -kernel root/rxv64.elf
// qemu-system-x86_64 -cpu qemu64,pdpe1gb,xsaveopt,fsgsbase,apic -smp 8 -m 8192 -curses -kernel root/rxv64.elf
// qemu-system-x86_64 -cpu host,pdpe1gb,xsaveopt,fsgsbase,apic -accel hvf -smp 8 -m 2048 -nographic -kernel root/rxv64.elf
// qemu-system-x86_64 -cpu host,pdpe1gb,xsaveopt,fsgsbase,apic,msr -accel kvm -smp 8 -m 8192 -curses -kernel root/rxv64.elf
// qemu-system-x86_64 -cpu qemu64,pdpe1gb,xsaveopt,fsgsbase,x2apic -smp 8 -m 8192 -curses -kernel root/rxv64.elf
// qemu-system-x86_64 -cpu qemu64,pdpe1gb,xsaveopt,fsgsbase,apic,msr -smp 8 -m 8192 -curses -kernel root/rxv64.elf
// qemu-system-x86_64 -cpu qemu64,pdpe1gb,xsaveopt,fsgsbase,apic,msr -smp 8 -m 1152 -curses -kernel root/rxv64.elf
// qemu-system-x86_64 -cpu qemu64,pdpe1gb,xsaveopt,fsgsbase,apic,msr -smp 2 -m 129 -curses -kernel root/rxv64.elf
// qemu-system-x86_64 -cpu qemu64,pdpe1gb,xsaveopt,fsgsbase,apic,msr -smp 2 -m 129 -curses -kernel root/rxv64.elf
// qemu-system-x86_64 -cpu qemu64,pdpe1gb,xsaveopt,fsgsbase,apic,msr -smp 8 -m 8192 -kernel root/rxv64.elf
// qemu-system-x86_64 -cpu host,pdpe1gb,xsaveopt,fsgsbase,apic,msr -accel kvm -smp 8 -m 8192 -curses -kernel root/rxv64.elf
// qemu-system-x86_64 -cpu qemu64,pdpe1gb,xsaveopt,fsgsbase,apic -smp 8 -m 8192 -nographic -kernel root/rxv64.elf
// qemu-system-x86_64 -cpu qemu64,pdpe1gb,xsaveopt,fsgsbase,apic,msr -smp 8 -m 8192 -nographic -kernel root/rxv64.elf
// qemu-system-x86_64 -cpu host,pdpe1gb,xsaveopt,fsgsbase,apic,msr -accel kvm -smp 8 -m 8192 -curses "$@" -kernel root/rxv64.elf
fn run() -> Result<()> {
    let profile = Build::Release;
    dist(profile)?;
    let status = Command::new(qemu_system_x86_64())
        //.arg("-nographic")
        //.arg("-curses")
        .arg("-s")
        .arg("-M")
        .arg("q35")
        .arg("-cpu")
        .arg("qemu64,pdpe1gb,xsaveopt,fsgsbase,apic,msr")
        .arg("-smp")
        .arg("8")
        .arg("-m")
        .arg("8192")
        .arg("-device")
        .arg("ahci,id=ahci0")
        .arg("-drive")
        .arg("id=sdahci0,file=sdahci0.img,if=none,format=raw")
        .arg("-device")
        .arg("ide-hd,drive=sdahci0,bus=ahci0.0")
        .arg("-kernel")
        .arg(format!(
            "target/{}/{}/rxv64.elf32",
            ktarget(),
            profile.dir()
        ))
        .current_dir(workspace())
        .status()?;
    if !status.success() {
        return Err("qemu failed".into());
    }
    Ok(())
}

fn accelrun() -> Result<()> {
    let profile = Build::Release;
    dist(profile)?;
    let status = Command::new(qemu_system_x86_64())
        //.arg("-nographic")
        .arg("-display")
        .arg("curses")
        .arg("-accel")
        .arg("kvm")
        .arg("-M")
        .arg("q35")
        .arg("-cpu")
        .arg("host,pdpe1gb,xsaveopt,fsgsbase,apic,msr")
        .arg("-smp")
        .arg("8")
        .arg("-m")
        .arg("8192")
        .arg("-device")
        .arg("ahci,id=ahci0")
        .arg("-drive")
        .arg("id=sdahci0,file=sdahci0.img,if=none,format=raw")
        .arg("-device")
        .arg("ide-hd,drive=sdahci0,bus=ahci0.0")
        .arg("-kernel")
        .arg(format!(
            "target/{}/{}/rxv64.elf32",
            ktarget(),
            profile.dir()
        ))
        .current_dir(workspace())
        .status()?;
    if !status.success() {
        return Err("qemu failed".into());
    }
    Ok(())
}

fn clean() -> Result<()> {
    let status = Command::new(cargo())
        .current_dir(workspace())
        .arg("clean")
        .status()?;
    if !status.success() {
        return Err("clean failed".into());
    }
    Ok(())
}

fn workspace() -> PathBuf {
    Path::new(&env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(1)
        .unwrap()
        .to_path_buf()
}
