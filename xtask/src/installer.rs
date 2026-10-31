//! `cargo xtask installer`: this platform's installer packages, made from
//! `cargo xtask bundle`'s output and the factory content.
//!
//! The payload is staged under `target/installer/root` as the files will sit
//! once installed, then packed:
//!
//! - macOS: one `.pkg` (`pkgbuild`, `productbuild`), which installs the CLAP,
//!   VST3 and Audio Unit plug-ins system-wide, the standalone app in
//!   `/Applications` and an uninstall script beside the factory content;
//! - Windows: one `.msi` (WiX), which Windows removes itself;
//! - Linux: a `.deb` and an `.rpm`.
//!
//! It also writes `target/installer/manifest.txt`, every path the installer
//! puts down and the uninstaller must take away, which `installer/test-*`
//! check after installing and again after uninstalling.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The plug-in crates the installers carry: the instrument now, the effect
/// plug-ins as they arrive.
const PLUGINS: &[&str] = &["reagent"];

/// Where the factory presets and wavetables are made.
const FACTORY: &str = "crates/content/factory";

/// Tells Windows Installer these packages are one product across versions.
const UPGRADE_CODE: &str = "6f1c2d9e-3b4a-4c58-9e07-5a8d1f2b7c43";

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Os {
    Mac,
    Windows,
    Linux,
}

/// Where each kind of file goes, relative to the staged root. On Windows the
/// first part names a standard folder: `CommonFiles` or `ProgramFiles`.
struct Places {
    clap: &'static str,
    vst3: &'static str,
    component: &'static str,
    app: &'static str,
    /// Reagent's own folder, which holds the factory content in `factory`.
    data: &'static str,
}

fn places(os: Os) -> Places {
    match os {
        Os::Mac => Places {
            clap: "Library/Audio/Plug-Ins/CLAP",
            vst3: "Library/Audio/Plug-Ins/VST3",
            component: "Library/Audio/Plug-Ins/Components",
            app: "Applications",
            data: "Library/Application Support/Loombridge/Reagent",
        },
        Os::Windows => Places {
            clap: "CommonFiles/CLAP",
            vst3: "CommonFiles/VST3",
            component: "",
            app: "ProgramFiles/Loombridge/Reagent",
            data: "ProgramFiles/Loombridge/Reagent",
        },
        Os::Linux => Places {
            clap: "usr/lib/clap",
            vst3: "usr/lib/vst3",
            component: "",
            app: "usr/bin",
            data: "usr/share/loombridge/reagent",
        },
    }
}

/// What `cargo xtask bundle` writes for `plugin` on `os`, and the staged
/// folder each file goes to.
fn payload(os: Os, plugin: &str, bundle: &Path) -> Vec<(PathBuf, &'static str)> {
    let p = places(os);
    let app = match os {
        Os::Mac => format!("{plugin}.app"),
        Os::Windows => format!("{plugin}.exe"),
        Os::Linux => plugin.to_owned(),
    };
    let mut items = vec![
        (bundle.join(format!("{plugin}.clap")), p.clap),
        (bundle.join(format!("{plugin}.vst3")), p.vst3),
        (bundle.join(app), p.app),
    ];
    if os == Os::Mac {
        items.push((bundle.join(format!("{plugin}.component")), p.component));
    }
    items
}

pub fn installer(root: &Path) -> Result<(), String> {
    let os = if cfg!(target_os = "macos") {
        Os::Mac
    } else if cfg!(windows) {
        Os::Windows
    } else {
        Os::Linux
    };
    let exe = std::env::current_exe().map_err(|e| format!("couldn't find xtask: {e}"))?;
    let mut bundle_args = vec!["bundle"];
    bundle_args.extend(PLUGINS);
    run(Command::new(exe).args(bundle_args).current_dir(root))?;

    let out = target_dir(root)?.join("installer");
    let stage = out.join("root");
    let dist = out.join("dist");
    if stage.exists() {
        fs::remove_dir_all(&stage).map_err(|e| format!("{}: {e}", stage.display()))?;
    }
    fs::create_dir_all(&dist).map_err(|e| format!("{}: {e}", dist.display()))?;

    let bundle = out.parent().ok_or("target has a parent")?.join("bundle");
    let mut tops = BTreeSet::new();
    for plugin in PLUGINS {
        for (source, to) in payload(os, plugin, &bundle) {
            tops.insert(copy_into(&source, &stage.join(to))?);
        }
    }
    let data = stage.join(places(os).data);
    tops.insert(data.clone());
    copy_into(&root.join(FACTORY), &data)?;

    let manifest = manifest(os, &stage, &tops)?;
    write(&out.join("manifest.txt"), &manifest)?;

    match os {
        Os::Mac => mac(&stage, &data, &dist),
        Os::Windows => windows(&stage, &out, &dist),
        Os::Linux => linux(&stage, &out, &dist),
    }
}

/// Copies the file or folder `source` into the folder `into` under its own
/// name, and returns where it went.
fn copy_into(source: &Path, into: &Path) -> Result<PathBuf, String> {
    let name = source.file_name().ok_or("a source has a name")?;
    let to = into.join(name);
    let failed = |e: std::io::Error| format!("{}: {e}", source.display());
    if source.is_dir() {
        fs::create_dir_all(&to).map_err(failed)?;
        for entry in fs::read_dir(source).map_err(failed)? {
            copy_into(&entry.map_err(failed)?.path(), &to)?;
        }
    } else {
        fs::create_dir_all(into).map_err(failed)?;
        fs::copy(source, &to).map_err(failed)?;
    }
    Ok(to)
}

/// Every installed path, as the test scripts find it: each top-level item
/// and everything in it, but none of the shared folders above them.
fn manifest(os: Os, stage: &Path, tops: &BTreeSet<PathBuf>) -> Result<String, String> {
    let mut all = BTreeSet::new();
    for top in tops {
        walk(top, &mut all)?;
    }
    let mut text = String::new();
    for path in all {
        let relative = path.strip_prefix(stage).map_err(|e| e.to_string())?;
        let installed = installed_path(os, relative, &|name| std::env::var(name).ok())?;
        writeln!(text, "{}", installed.display()).map_err(|e| e.to_string())?;
    }
    Ok(text)
}

fn walk(path: &Path, into: &mut BTreeSet<PathBuf>) -> Result<(), String> {
    into.insert(path.to_owned());
    if path.is_dir() {
        for entry in fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))? {
            walk(&entry.map_err(|e| e.to_string())?.path(), into)?;
        }
    }
    Ok(())
}

/// Where the staged `relative` path lands on `os`: under `/`, except on
/// Windows where its first part is `CommonFiles` or `ProgramFiles`, found
/// from the environment.
fn installed_path(
    os: Os,
    relative: &Path,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<PathBuf, String> {
    if os != Os::Windows {
        return Ok(Path::new("/").join(relative));
    }
    let mut parts = relative.components();
    let variable = match parts.next().and_then(|c| c.as_os_str().to_str()) {
        Some("CommonFiles") => "CommonProgramW6432",
        Some("ProgramFiles") => "ProgramW6432",
        other => return Err(format!("{other:?} isn't a Windows install folder")),
    };
    let base = env(variable).ok_or_else(|| format!("{variable} isn't set"))?;
    Ok(PathBuf::from(base).join(parts.as_path()))
}

fn mac(stage: &Path, data: &Path, dist: &Path) -> Result<(), String> {
    let script = data.join("uninstall.sh");
    write(&script, UNINSTALL_MAC)?;
    set_executable(&script)?;
    // Left alone, the installer would update any copy of the app it finds
    // elsewhere, such as the one `cargo xtask bundle` leaves in `target`, and
    // skip a bundle at the same version.
    let plist = dist.join("component.plist");
    run(Command::new("pkgbuild")
        .arg("--analyze")
        .arg("--root")
        .arg(stage)
        .arg(&plist))?;
    let analysis = fs::read_to_string(&plist).map_err(|e| e.to_string())?;
    write(&plist, &pin_bundles(&analysis))?;
    let component = dist.join("component.pkg");
    run(Command::new("pkgbuild")
        .args(["--root"])
        .arg(stage)
        .arg("--component-plist")
        .arg(&plist)
        .args(["--identifier", MAC_ID, "--version", VERSION])
        .args(["--install-location", "/", "--ownership", "recommended"])
        .arg(&component))?;
    let pkg = dist.join(format!(
        "Reagent-{VERSION}-macos-{}.pkg",
        std::env::consts::ARCH
    ));
    run(Command::new("productbuild")
        .arg("--package")
        .arg(&component)
        .arg(&pkg))?;
    for scratch in [&component, &plist] {
        fs::remove_file(scratch).map_err(|e| e.to_string())?;
    }
    eprintln!("{}", pkg.display());
    Ok(())
}

/// `pkgbuild --analyze`'s component list with every bundle pinned to where
/// the package puts it, and installed even when the same version is there.
fn pin_bundles(analysis: &str) -> String {
    ["BundleIsRelocatable", "BundleIsVersionChecked"]
        .iter()
        .fold(analysis.to_owned(), |text, key| {
            text.replace(
                &format!("<key>{key}</key>\n\t\t<true/>"),
                &format!("<key>{key}</key>\n\t\t<false/>"),
            )
        })
}

const MAC_ID: &str = "com.loombridge.reagent";

/// Removes what the package installed, as its file list names it: files
/// first, then each folder it made that's now empty. It never deletes a
/// folder with anything else in it.
const UNINSTALL_MAC: &str = "#!/bin/bash
# Removes Reagent. Run it with sudo.
set -eu
pkg=com.loombridge.reagent
if [ \"$(id -u)\" != 0 ]; then echo \"run this with sudo\" >&2; exit 1; fi
prefix=$(pkgutil --pkg-info \"$pkg\" | sed -n 's/^location: //p')
files=$(pkgutil --only-files --files \"$pkg\")
dirs=$(pkgutil --only-dirs --files \"$pkg\" | sort -r)
while IFS= read -r f; do rm -f \"/$prefix/$f\"; done <<< \"$files\"
while IFS= read -r d; do rmdir \"/$prefix/$d\" 2>/dev/null || true; done <<< \"$dirs\"
pkgutil --forget \"$pkg\" > /dev/null
";

fn windows(stage: &Path, out: &Path, dist: &Path) -> Result<(), String> {
    let wxs = out.join("reagent.wxs");
    write(&wxs, &wix_source(stage)?)?;
    let msi = dist.join(format!("Reagent-{VERSION}-windows-x64.msi"));
    run(Command::new("wix")
        .args(["build", "-arch", "x64", "-o"])
        .arg(&msi)
        .arg(&wxs))?;
    eprintln!("{}", msi.display());
    Ok(())
}

/// The WiX source for everything staged: a component for each file, in the
/// folders the stage holds.
fn wix_source(stage: &Path) -> Result<String, String> {
    let mut next = 0;
    let mut directories = String::new();
    let mut refs = String::new();
    for (folder, standard) in [
        ("CommonFiles", "CommonFiles64Folder"),
        ("ProgramFiles", "ProgramFiles64Folder"),
    ] {
        writeln!(directories, "    <StandardDirectory Id=\"{standard}\">").unwrap();
        wix_directory(
            &stage.join(folder),
            3,
            &mut next,
            &mut directories,
            &mut refs,
        )?;
        writeln!(directories, "    </StandardDirectory>").unwrap();
    }
    Ok(format!(
        "<Wix xmlns=\"http://wixtoolset.org/schemas/v4/wxs\">
  <Package Name=\"Reagent\" Manufacturer=\"Loombridge\" Version=\"{VERSION}\" UpgradeCode=\"{UPGRADE_CODE}\" Scope=\"perMachine\">
    <MajorUpgrade DowngradeErrorMessage=\"A newer Reagent is already installed.\" />
    <MediaTemplate EmbedCab=\"yes\" />
{directories}    <Feature Id=\"Reagent\">
{refs}    </Feature>
  </Package>
</Wix>
"
    ))
}

fn wix_directory(
    path: &Path,
    depth: usize,
    next: &mut usize,
    out: &mut String,
    refs: &mut String,
) -> Result<(), String> {
    let pad = "  ".repeat(depth);
    let mut entries: Vec<_> = fs::read_dir(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    entries.sort();
    for entry in entries {
        *next += 1;
        let id = *next;
        let name = xml(&entry.file_name().unwrap_or_default().to_string_lossy());
        if entry.is_dir() {
            writeln!(out, "{pad}<Directory Id=\"d{id}\" Name=\"{name}\">").unwrap();
            wix_directory(&entry, depth + 1, next, out, refs)?;
            writeln!(out, "{pad}</Directory>").unwrap();
        } else {
            let source = xml(&entry.to_string_lossy());
            writeln!(
                out,
                "{pad}<Component Id=\"c{id}\"><File Id=\"f{id}\" Name=\"{name}\" Source=\"{source}\" /></Component>"
            )
            .unwrap();
            writeln!(refs, "      <ComponentRef Id=\"c{id}\" />").unwrap();
        }
    }
    Ok(())
}

fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('"', "&quot;")
}

fn linux(stage: &Path, out: &Path, dist: &Path) -> Result<(), String> {
    let deb_arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => return Err(format!("no Linux package for {other}")),
    };
    write(&stage.join("DEBIAN/control"), &deb_control(deb_arch))?;
    let deb = dist.join(format!("reagent_{VERSION}_{deb_arch}.deb"));
    run(Command::new("dpkg-deb")
        .args(["--build", "--root-owner-group"])
        .arg(stage)
        .arg(&deb))?;
    eprintln!("{}", deb.display());

    let spec = out.join("reagent.spec");
    write(&spec, &rpm_spec(stage, std::env::consts::ARCH))?;
    let rpm = format!("reagent-{VERSION}-1.{}.rpm", std::env::consts::ARCH);
    run(Command::new("rpmbuild")
        .arg("-bb")
        .arg("--define")
        .arg(format!("_topdir {}", out.join("rpmbuild").display()))
        .arg("--define")
        .arg(format!("_rpmdir {}", dist.display()))
        .arg("--define")
        .arg(format!("_rpmfilename {rpm}"))
        .arg(&spec))?;
    eprintln!("{}", dist.join(rpm).display());
    Ok(())
}

const SUMMARY: &str = "Reagent wavetable synthesiser";
const DESCRIPTION: &str =
    "Reagent as CLAP and VST3 plug-ins and a standalone app, with its factory content.";

/// What the plug-ins load at run time: ALSA, for the standalone app's audio.
fn deb_control(arch: &str) -> String {
    format!(
        "Package: reagent
Version: {VERSION}
Architecture: {arch}
Maintainer: Loombridge <code@loombridge.io>
Section: sound
Priority: optional
Depends: libasound2t64 | libasound2
Description: {SUMMARY}
 {DESCRIPTION}
"
    )
}

/// The spec copies the staged tree into the build root, and lists the
/// top-level items: `DEBIAN`, which the stage also holds for the `.deb`,
/// isn't one of them.
fn rpm_spec(stage: &Path, arch: &str) -> String {
    let mut files = String::new();
    for place in PLUGINS
        .iter()
        .flat_map(|p| payload(Os::Linux, p, Path::new("")))
        .map(|(source, to)| {
            Path::new("/")
                .join(to)
                .join(source.file_name().unwrap_or_default())
        })
        .chain([Path::new("/").join(places(Os::Linux).data)])
    {
        writeln!(files, "{}", place.display()).unwrap();
    }
    format!(
        "%global debug_package %{{nil}}
%global __os_install_post %{{nil}}
%global _build_id_links none
%define _unpackaged_files_terminate_build 0
Name: reagent
Version: {VERSION}
Release: 1
Summary: {SUMMARY}
License: GPL-3.0-or-later
BuildArch: {arch}
AutoReqProv: no
Requires: alsa-lib

%description
{DESCRIPTION}

%install
mkdir -p %{{buildroot}}
cp -a {stage}/. %{{buildroot}}/

%files
{files}",
        stage = stage.display()
    )
}

fn target_dir(root: &Path) -> Result<PathBuf, String> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(root)
        .output()
        .map_err(|e| format!("couldn't run cargo: {e}"))?;
    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|e| format!("cargo metadata: {e}"))?;
    metadata["target_directory"]
        .as_str()
        .map(PathBuf::from)
        .ok_or_else(|| "cargo metadata names no target directory".to_owned())
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(not(unix))]
fn set_executable(_: &Path) -> Result<(), String> {
    Ok(())
}

fn run(command: &mut Command) -> Result<(), String> {
    let status = command
        .status()
        .map_err(|e| format!("couldn't run {:?}: {e}", command.get_program()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{:?} failed ({status})", command.get_program()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stage with one CLAP, a VST3 folder and a factory folder, as
    /// Windows lays them out.
    fn windows_stage() -> tempdir::Stage {
        tempdir::Stage::with(
            "windows",
            &[
                "CommonFiles/CLAP/reagent.clap",
                "CommonFiles/VST3/reagent.vst3/Contents/x86_64-win/reagent.vst3",
                "ProgramFiles/Loombridge/Reagent/reagent.exe",
                "ProgramFiles/Loombridge/Reagent/factory/README.md",
            ],
        )
    }

    /// A scratch folder with the given files in it, left for the system to clear.
    mod tempdir {
        use std::path::PathBuf;

        pub struct Stage(pub PathBuf);

        impl Stage {
            pub fn with(name: &str, files: &[&str]) -> Self {
                let dir = std::env::temp_dir()
                    .join(format!("reagent-installer-{name}-{}", std::process::id()));
                for file in files {
                    let path = dir.join(file);
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    std::fs::write(path, file).unwrap();
                }
                Self(dir)
            }
        }
    }

    #[test]
    fn every_platform_puts_the_plugins_in_the_places_hosts_look() {
        let bundle = Path::new("bundle");
        for (os, clap, vst3) in [
            (
                Os::Mac,
                "Library/Audio/Plug-Ins/CLAP",
                "Library/Audio/Plug-Ins/VST3",
            ),
            (Os::Windows, "CommonFiles/CLAP", "CommonFiles/VST3"),
            (Os::Linux, "usr/lib/clap", "usr/lib/vst3"),
        ] {
            let items = payload(os, "reagent", bundle);
            assert!(
                items.contains(&(bundle.join("reagent.clap"), clap)),
                "{os:?}"
            );
            assert!(
                items.contains(&(bundle.join("reagent.vst3"), vst3)),
                "{os:?}"
            );
        }
    }

    #[test]
    fn only_macos_carries_the_audio_unit_and_each_platform_its_own_app() {
        let bundle = Path::new("bundle");
        let names = |os| -> Vec<String> {
            payload(os, "reagent", bundle)
                .iter()
                .map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned())
                .collect()
        };
        assert!(names(Os::Mac).contains(&"reagent.component".to_owned()));
        assert!(names(Os::Mac).contains(&"reagent.app".to_owned()));
        assert!(names(Os::Windows).contains(&"reagent.exe".to_owned()));
        assert!(names(Os::Linux).contains(&"reagent".to_owned()));
        assert!(!names(Os::Windows).contains(&"reagent.component".to_owned()));
        assert!(!names(Os::Linux).contains(&"reagent.component".to_owned()));
    }

    #[test]
    fn windows_folders_come_from_the_environment() {
        let env = |name: &str| match name {
            "CommonProgramW6432" => Some("C:\\Program Files\\Common Files".to_owned()),
            "ProgramW6432" => Some("C:\\Program Files".to_owned()),
            _ => None,
        };
        let path = |p: &str| installed_path(Os::Windows, Path::new(p), &env).unwrap();
        assert_eq!(
            path("CommonFiles/CLAP/reagent.clap"),
            Path::new("C:\\Program Files\\Common Files").join("CLAP/reagent.clap")
        );
        assert_eq!(
            path("ProgramFiles/Loombridge"),
            Path::new("C:\\Program Files").join("Loombridge")
        );
        assert!(installed_path(Os::Windows, Path::new("usr/lib"), &env).is_err());
        assert_eq!(
            installed_path(Os::Linux, Path::new("usr/lib/clap"), &env).unwrap(),
            Path::new("/usr/lib/clap")
        );
    }

    #[test]
    fn the_manifest_lists_each_item_and_what_is_in_it_but_not_the_shared_folders() {
        let stage = windows_stage();
        let tops = BTreeSet::from([
            stage.0.join("CommonFiles/CLAP/reagent.clap"),
            stage.0.join("CommonFiles/VST3/reagent.vst3"),
            stage.0.join("ProgramFiles/Loombridge/Reagent"),
        ]);
        let text = manifest(Os::Linux, &stage.0, &tops).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert!(
            lines
                .iter()
                .any(|l| l.ends_with("CommonFiles/CLAP/reagent.clap"))
        );
        assert!(
            lines
                .iter()
                .any(|l| l.ends_with("reagent.vst3/Contents/x86_64-win/reagent.vst3"))
        );
        assert!(
            lines
                .iter()
                .any(|l| l.ends_with("Reagent/factory/README.md"))
        );
        assert!(!lines.iter().any(|l| l.ends_with("CommonFiles/CLAP")));
        assert!(!lines.iter().any(|l| l.ends_with("ProgramFiles/Loombridge")));
    }

    #[test]
    fn the_wix_source_has_a_component_for_every_staged_file_in_its_folder() {
        let stage = windows_stage();
        let source = wix_source(&stage.0).unwrap();
        assert_eq!(source.matches("<File ").count(), 4);
        assert_eq!(source.matches("<ComponentRef ").count(), 4);
        assert!(source.contains("Name=\"reagent.vst3\""));
        assert!(source.contains("<StandardDirectory Id=\"CommonFiles64Folder\">"));
        assert!(source.contains(&format!("UpgradeCode=\"{UPGRADE_CODE}\"")));
        assert_eq!(xml("a&b\"<"), "a&amp;b&quot;&lt;");
    }

    #[test]
    fn the_rpm_lists_the_items_and_not_the_debian_folder() {
        let spec = rpm_spec(Path::new("/stage"), "x86_64");
        let files = spec.split("%files\n").nth(1).unwrap();
        assert_eq!(
            files.lines().collect::<Vec<_>>(),
            [
                "/usr/lib/clap/reagent.clap",
                "/usr/lib/vst3/reagent.vst3",
                "/usr/bin/reagent",
                "/usr/share/loombridge/reagent"
            ]
        );
        assert!(spec.contains("cp -a /stage/. %{buildroot}/"));
    }

    #[test]
    fn pkgbuild_is_told_not_to_relocate_or_skip_a_bundle() {
        let analysis = "<key>BundleIsRelocatable</key>\n\t\t<true/>\n\t\t<key>BundleIsVersionChecked</key>\n\t\t<true/>\n\t\t<key>BundleHasStrictIdentifier</key>\n\t\t<true/>";
        let pinned = pin_bundles(analysis);
        assert!(pinned.contains("<key>BundleIsRelocatable</key>\n\t\t<false/>"));
        assert!(pinned.contains("<key>BundleIsVersionChecked</key>\n\t\t<false/>"));
        assert!(pinned.contains("<key>BundleHasStrictIdentifier</key>\n\t\t<true/>"));
    }

    #[test]
    fn the_deb_depends_on_alsa_and_names_its_architecture() {
        let control = deb_control("amd64");
        assert!(control.contains("Architecture: amd64\n"));
        assert!(control.contains("Depends: libasound2t64 | libasound2\n"));
    }

    #[test]
    fn the_macos_uninstaller_never_deletes_a_folder_recursively() {
        assert!(!UNINSTALL_MAC.contains("rm -r"));
        assert!(UNINSTALL_MAC.contains("com.loombridge.reagent"));
    }
}
