//! Embed desktop application resources; engine data is built by kaixin-core.
use std::path::{Path, PathBuf};
use std::{env, fs};

fn version_resource_numbers(version: &str) -> [u16; 4] {
    let mut out = [0u16; 4];
    for (idx, part) in version.split('.').take(4).enumerate() {
        let digits: String = part.chars().take_while(|ch| ch.is_ascii_digit()).collect();
        out[idx] = digits.parse::<u16>().unwrap_or(0);
    }
    out
}

fn rc_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn compile_windows_app_resource(out_dir: &Path, manifest_dir: &Path, app_version: &str) {
    #[cfg(windows)]
    {
        let repo_root = manifest_dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| manifest_dir.to_path_buf());
        let icon_path = repo_root.join("assets").join("kaixin-input.ico");
        if !icon_path.is_file() {
            println!(
                "cargo:warning=Windows icon not found, skip embedding: {}",
                icon_path.display()
            );
            return;
        }

        let icon_path: PathBuf = icon_path
            .canonicalize()
            .unwrap_or_else(|_| icon_path.clone());
        let icon_escaped = rc_string(&icon_path.display().to_string());
        let version_numbers = version_resource_numbers(app_version);
        let app_name = "\u{5f00}\u{5fc3}\u{8f93}\u{5165}\u{6cd5}";
        let company_name = "\u{5f00}\u{5fc3}\u{8f93}\u{5165}\u{6cd5}";
        let rc_path = out_dir.join("kaixin_app.rc");
        let rc_content = format!(
            r#"#pragma code_page(65001)
1 ICON "{icon}"

1 VERSIONINFO
FILEVERSION {v0},{v1},{v2},{v3}
PRODUCTVERSION {v0},{v1},{v2},{v3}
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "080404B0"
        BEGIN
            VALUE "CompanyName", "{company}"
            VALUE "FileDescription", "{app}"
            VALUE "FileVersion", "{version}"
            VALUE "InternalName", "{app}"
            VALUE "OriginalFilename", "{app}"
            VALUE "ProductName", "{app}"
            VALUE "ProductVersion", "{version}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x0804, 1200
    END
END
"#,
            icon = icon_escaped,
            v0 = version_numbers[0],
            v1 = version_numbers[1],
            v2 = version_numbers[2],
            v3 = version_numbers[3],
            company = rc_string(company_name),
            app = rc_string(app_name),
            version = rc_string(app_version),
        );
        if let Err(err) = fs::write(&rc_path, rc_content) {
            println!(
                "cargo:warning=Failed to write Windows resource script: {} ({err})",
                rc_path.display()
            );
            return;
        }

        let result =
            embed_resource::compile(rc_path.to_string_lossy().as_ref(), embed_resource::NONE);
        if let Err(err) = result.manifest_optional() {
            println!("cargo:warning=Failed to compile Windows app resource: {err}");
        } else {
            println!("cargo:rerun-if-changed={}", icon_path.display());
            println!("cargo:rerun-if-changed={}", rc_path.display());
        }
    }
}

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("out dir"));
    let version_path = manifest.join("../VERSION");
    let version = fs::read_to_string(&version_path).expect("read VERSION");
    assert_eq!(
        version.trim(),
        env::var("CARGO_PKG_VERSION").expect("package version")
    );
    compile_windows_app_resource(&out, &manifest, version.trim());
    println!("cargo:rerun-if-changed={}", version_path.display());
    println!(
        "cargo:rerun-if-changed={}",
        manifest.join("../assets/kaixin-input.ico").display()
    );
}
