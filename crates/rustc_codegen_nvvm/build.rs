use std::{
    env,
    ffi::{OsStr, OsString},
    fmt::Display,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

fn main() {
    rustc_llvm_build();

    // this is set by cuda_builder, but in case somebody is using the codegen
    // manually, default to 520 (which is what nvvm defaults to).
    if option_env!("CUDA_ARCH").is_none() {
        println!("cargo:rustc-env=CUDA_ARCH=520")
    }
}

fn fail(s: &str) -> ! {
    println!("\n\n{}\n\n", s);
    std::process::exit(1);
}

#[track_caller]
pub fn output(cmd: &mut Command) -> String {
    let output = match cmd.stderr(Stdio::inherit()).output() {
        Ok(status) => status,
        Err(e) => fail(&format!(
            "failed to execute command: {:?}\nerror: {}",
            cmd, e
        )),
    };
    if !output.status.success() {
        panic!(
            "command did not execute successfully: {:?}\n\
             expected success, got: {}",
            cmd, output.status
        );
    }
    String::from_utf8(output.stdout).unwrap()
}

fn detect_llvm_link() -> (&'static str, &'static str) {
    // Force the link mode we want, preferring static by default, but
    // possibly overridden by `configure --enable-llvm-link-shared`.
    if tracked_env_var_os("LLVM_LINK_SHARED").is_some() {
        ("dylib", "--link-shared")
    } else {
        ("static", "--link-static")
    }
}

pub fn tracked_env_var_os<K: AsRef<OsStr> + Display>(key: K) -> Option<OsString> {
    println!("cargo:rerun-if-env-changed={}", key);
    env::var_os(key)
}

fn rustc_llvm_build() {
    let target = env::var("TARGET").expect("TARGET was not set");
    let llvm_config = tracked_env_var_os("LLVM_CONFIG")
        .map(|x| Some(PathBuf::from(x)))
        .unwrap_or_else(|| {
            if let Some(dir) = tracked_env_var_os("CARGO_TARGET_DIR").map(PathBuf::from) {
                let to_test = dir
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join(&target)
                    .join("llvm/bin/llvm-config");
                if Command::new(&to_test).output().is_ok() {
                    return Some(to_test);
                }
            }
            None
        });

    if let Some(llvm_config) = &llvm_config {
        println!("cargo:rerun-if-changed={}", llvm_config.display());
    }
    let llvm_config = llvm_config.unwrap_or_else(|| PathBuf::from("llvm-config"));

    let optional_components = &[
        "x86",
        "arm",
        "aarch64",
        "amdgpu",
        "avr",
        "mips",
        "powerpc",
        "systemz",
        "jsbackend",
        "webassembly",
        "msp430",
        "sparc",
        "nvptx",
        "hexagon",
        "riscv",
        "bpf",
    ];

    let required_components = &[
        "ipo",
        "bitreader",
        "bitwriter",
        "linker",
        "asmparser",
        "lto",
        "coverage",
        "instrumentation",
    ];

    let components = output(Command::new(&llvm_config).arg("--components"));
    let mut components = components.split_whitespace().collect::<Vec<_>>();
    components.retain(|c| optional_components.contains(c) || required_components.contains(c));

    for component in required_components {
        if !components.contains(component) {
            panic!("require llvm component {} but wasn't found", component);
        }
    }

    for component in components.iter() {
        println!("cargo:rustc-cfg=llvm_component=\"{}\"", component);
    }

    // Link in our own LLVM shims, compiled with the same flags as LLVM
    let mut cmd = Command::new(&llvm_config);
    cmd.arg("--cxxflags");
    let cxxflags = output(&mut cmd);
    let mut cfg = cc::Build::new();
    cfg.warnings(false);
    for flag in cxxflags.split_whitespace() {
        if flag.starts_with("-flto") {
            continue;
        }

        cfg.flag(flag);
    }

    for component in &components {
        let mut flag = String::from("LLVM_COMPONENT_");
        flag.push_str(&component.to_uppercase());
        cfg.define(&flag, None);
    }

    if tracked_env_var_os("LLVM_RUSTLLVM").is_some() {
        cfg.define("LLVM_RUSTLLVM", None);
    }

    build_helper::rerun_if_changed(Path::new("rustc_llvm_wrapper"));
    cfg.file("rustc_llvm_wrapper/RustWrapper.cpp")
        .file("rustc_llvm_wrapper/PassWrapper.cpp")
        .include("rustc_llvm_wrapper/rustllvm.h")
        .cpp(true)
        .cpp_link_stdlib(None) // we handle this below
        .compile("llvm-wrapper");

    let (llvm_kind, llvm_link_arg) = detect_llvm_link();

    // Link in all LLVM libraries, if we're using the "wrong" llvm-config then
    // we don't pick up system libs because unfortunately they're for the host
    // of llvm-config, not the target that we're attempting to link.
    let mut cmd = Command::new(&llvm_config);
    cmd.arg(llvm_link_arg).arg("--libs");

    if target.contains("windows-gnu") {
        println!("cargo:rustc-link-lib=shell32");
        println!("cargo:rustc-link-lib=uuid");
    } else if target.contains("netbsd") || target.contains("haiku") {
        println!("cargo:rustc-link-lib=z");
    }
    cmd.args(&components);

    for lib in output(&mut cmd).split_whitespace() {
        let name = if let Some(stripped) = lib.strip_prefix("-l") {
            stripped
        } else if let Some(stripped) = lib.strip_prefix('-') {
            stripped
        } else if Path::new(lib).exists() {
            // On MSVC llvm-config will print the full name to libraries, but
            // we're only interested in the name part
            let name = Path::new(lib).file_name().unwrap().to_str().unwrap();
            name.trim_end_matches(".lib")
        } else if lib.ends_with(".lib") {
            // Some MSVC libraries just come up with `.lib` tacked on, so chop
            // that off
            lib.trim_end_matches(".lib")
        } else {
            continue;
        };

        // Don't need or want this library, but LLVM's CMake build system
        // doesn't provide a way to disable it, so filter it here even though we
        // may or may not have built it. We don't reference anything from this
        // library and it otherwise may just pull in extra dependencies on
        // libedit which we don't want
        if name == "LLVMLineEditor" {
            continue;
        }

        let kind = if name.starts_with("LLVM") {
            llvm_kind
        } else {
            "dylib"
        };
        println!("cargo:rustc-link-lib={}={}", kind, name);
    }

    // Link in the system libraries that LLVM depends on
    #[cfg(not(target_os = "windows"))]
    link_llvm_system_libs(&llvm_config);

    // LLVM ldflags
    //
    // If we're a cross-compile of LLVM then unfortunately we can't trust these
    // ldflags (largely where all the LLVM libs are located). Currently just
    // hack around this by replacing the host triple with the target and pray
    // that those -L directories are the same!
    let mut cmd = Command::new(&llvm_config);
    cmd.arg(llvm_link_arg).arg("--ldflags");
    for lib in output(&mut cmd).split_whitespace() {
        if let Some(stripped) = lib.strip_prefix("-LIBPATH:") {
            println!("cargo:rustc-link-search=native={}", stripped);
        } else if let Some(stripped) = lib.strip_prefix("-l") {
            println!("cargo:rustc-link-lib={}", stripped);
        } else if let Some(stripped) = lib.strip_prefix("-L") {
            println!("cargo:rustc-link-search=native={}", stripped);
        }
    }

    // Some LLVM linker flags (-L and -l) may be needed even when linking
    // rustc_llvm, for example when using static libc++, we may need to
    // manually specify the library search path and -ldl -lpthread as link
    // dependencies.
    let llvm_linker_flags = tracked_env_var_os("LLVM_LINKER_FLAGS");
    if let Some(s) = llvm_linker_flags {
        for lib in s.into_string().unwrap().split_whitespace() {
            if let Some(stripped) = lib.strip_prefix("-l") {
                println!("cargo:rustc-link-lib={}", stripped);
            } else if let Some(stripped) = lib.strip_prefix("-L") {
                println!("cargo:rustc-link-search=native={}", stripped);
            }
        }
    }

    let llvm_static_stdcpp = tracked_env_var_os("LLVM_STATIC_STDCPP");
    let llvm_use_libcxx = tracked_env_var_os("LLVM_USE_LIBCXX");

    let stdcppname = if target.contains("openbsd") {
        if target.contains("sparc64") {
            "estdc++"
        } else {
            "c++"
        }
    } else if target.contains("freebsd") || target.contains("darwin") {
        "c++"
    } else if target.contains("netbsd") && llvm_static_stdcpp.is_some() {
        // NetBSD uses a separate library when relocation is required
        "stdc++_pic"
    } else if llvm_use_libcxx.is_some() {
        "c++"
    } else {
        "stdc++"
    };

    // RISC-V requires libatomic for sub-word atomic operations
    if target.starts_with("riscv") {
        println!("cargo:rustc-link-lib=atomic");
    }

    // C++ runtime library
    if !target.contains("msvc") {
        if let Some(s) = llvm_static_stdcpp {
            assert!(!cxxflags.contains("stdlib=libc++"));
            let path = PathBuf::from(s);
            println!(
                "cargo:rustc-link-search=native={}",
                path.parent().unwrap().display()
            );
            if target.contains("windows") {
                println!("cargo:rustc-link-lib=static-nobundle={}", stdcppname);
            } else {
                println!("cargo:rustc-link-lib=static={}", stdcppname);
            }
        } else if cxxflags.contains("stdlib=libc++") {
            println!("cargo:rustc-link-lib=c++");
        } else {
            println!("cargo:rustc-link-lib={}", stdcppname);
        }
    }

    // Libstdc++ depends on pthread which Rust doesn't link on MinGW
    // since nothing else requires it.
    if target.contains("windows-gnu") {
        println!("cargo:rustc-link-lib=static-nobundle=pthread");
    }
}

#[cfg(not(target_os = "windows"))]
fn link_llvm_system_libs(llvm_config: &Path) {
    let mut cmd = Command::new(&llvm_config);
    cmd.arg("--system-libs");

    for lib in output(&mut cmd).split_whitespace() {
        let name = if let Some(stripped) = lib.strip_prefix("-l") {
            stripped
        } else {
            continue;
        };

        println!("cargo:rustc-link-lib=dylib={}", name);
    }
}