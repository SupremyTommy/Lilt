use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

fn find_rc(target_arch: &str) -> Option<PathBuf> {
    if Command::new("rc.exe").arg("/?").output().is_ok() {
        return Some(PathBuf::from("rc.exe"));
    }

    let kits = env::var_os("ProgramFiles(x86)")
        .map(PathBuf::from)?
        .join("Windows Kits")
        .join("10")
        .join("bin");
    let mut versions = fs::read_dir(kits)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    versions.sort_by(|left, right| right.file_name().cmp(&left.file_name()));
    let arch = if target_arch == "x86_64" {
        "x64"
    } else {
        target_arch
    };
    versions
        .iter()
        .map(|version| version.join(arch).join("rc.exe"))
        .chain(
            versions
                .iter()
                .map(|version| version.join("x86").join("rc.exe")),
        )
        .find(|path| path.is_file())
}

fn main() {
    println!("cargo:rerun-if-changed=assets/app_icon.rc");
    println!("cargo:rerun-if-changed=assets/app_icon.ico");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_else(|_| "x64".into());
    let rc = find_rc(&arch).unwrap_or_else(|| {
        panic!("Windows Resource Compiler (rc.exe) is required to embed Lilt's application icon")
    });
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set"));
    let resource = out_dir.join("lilt-app-icon.res");
    let assets = Path::new("assets");
    let result = Command::new(rc)
        .arg("/nologo")
        .arg(format!("/fo{}", resource.display()))
        .arg("app_icon.rc")
        .current_dir(assets)
        .status()
        .expect("failed to run Windows Resource Compiler");
    assert!(result.success(), "failed to compile assets/app_icon.rc");
    println!(
        "cargo:rustc-link-arg-bin=lilt-player={}",
        resource.display()
    );
}
