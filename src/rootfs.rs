use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::Digest;

pub fn base_dir() -> PathBuf {
    if let Ok(x) = std::env::var("XDG_CACHE_HOME") {
        if !x.is_empty() {
            return PathBuf::from(x).join("dshell");
        }
    }
    if let Ok(h) = std::env::var("HOME") {
        return PathBuf::from(h).join(".cache").join("dshell");
    }
    PathBuf::from("/tmp/dshell-cache")
}

fn fetch(url: &str, dest: &Path) -> Result<(), String> {
    let d = dest.to_string_lossy().into_owned();
    let attempts: [(&str, Vec<&str>); 2] =
        [("curl", vec!["-fSL", "-o", &d, url]), ("wget", vec!["-O", &d, url])];
    for (bin, args) in attempts {
        match Command::new(bin).args(&args).status() {
            Ok(s) if s.success() => return Ok(()),
            Ok(s) => return Err(format!("{bin} failed: {s}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("{bin}: {e}")),
        }
    }
    Err("need curl or wget".to_string())
}

fn fetch_text(url: &str, dir: &Path) -> Result<String, String> {
    let tmp = dir.join("sha256.tmp");
    fetch(url, &tmp)?;
    std::fs::read_to_string(&tmp).map_err(|e| e.to_string())
}

fn verify(tb: &Path, txt: &str) -> Result<(), String> {
    let base = tb.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let mut want = None;
    for line in txt.lines() {
        let mut p = line.split_whitespace();
        match (p.next(), p.next()) {
            (Some(h), Some(f)) if f.trim_start_matches('*') == base => {
                want = Some(h);
                break;
            }
            (Some(h), None) => {
                if want.is_none() {
                    want = Some(h);
                }
            }
            _ => {}
        }
    }
    let want = want.ok_or("bad sha256 file".to_string())?;
    let data = std::fs::read(tb).map_err(|e| e.to_string())?;
    let sum = sha2::Sha256::digest(&data);
    let got: String = sum.iter().map(|b| format!("{b:02x}")).collect();
    if got.eq_ignore_ascii_case(want) {
        Ok(())
    } else {
        Err("sha256 mismatch".to_string())
    }
}

fn reader(tb: &Path) -> Result<Box<dyn std::io::Read>, String> {
    let f = std::fs::File::open(tb).map_err(|e| e.to_string())?;
    let name = tb.file_name().and_then(|s| s.to_str()).unwrap_or("");
    if name.ends_with(".zst") {
        Ok(Box::new(
            ruzstd::decoding::StreamingDecoder::new(f).map_err(|e| e.to_string())?,
        ))
    } else {
        Ok(Box::new(flate2::read::GzDecoder::new(f)))
    }
}

fn unpack(tb: &Path, dest: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let mut ar = tar::Archive::new(reader(tb)?);
    for e in ar.entries().map_err(|e| e.to_string())? {
        let mut e = e.map_err(|e| e.to_string())?;
        let rel = e.path().map_err(|e| e.to_string())?.into_owned();
        if rel == Path::new(".") {
            continue;
        }
        if e.header().entry_type().is_dir() {
            std::fs::create_dir_all(dest.join(&rel)).map_err(|e| e.to_string())?;
        } else {
            e.unpack_in(dest).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn apply_modes(tb: &Path, dest: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut ar = tar::Archive::new(reader(tb)?);
    let mut dirs = Vec::new();
    for e in ar.entries().map_err(|e| e.to_string())? {
        let e = e.map_err(|e| e.to_string())?;
        let t = e.header().entry_type();
        if !(t.is_file() || t.is_dir()) {
            continue;
        }
        let mode = e.header().mode().map_err(|e| e.to_string())?;
        let p = dest.join(e.path().map_err(|e| e.to_string())?);
        if t.is_dir() {
            dirs.push((p, mode | 0o700));
        } else {
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode))
                .map_err(|e| e.to_string())?;
        }
    }
    dirs.sort_by(|a, b| b.0.cmp(&a.0));
    for (p, mode) in dirs {
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn writable(p: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let st = std::fs::symlink_metadata(p).map_err(|e| e.to_string())?;
    if st.file_type().is_symlink() {
        return Ok(());
    }
    if st.is_dir() {
        for e in std::fs::read_dir(p).map_err(|e| e.to_string())? {
            writable(&e.map_err(|e| e.to_string())?.path())?;
        }
    }
    let mut m = std::fs::metadata(p).map_err(|e| e.to_string())?.permissions();
    m.set_mode(m.mode() | 0o200);
    std::fs::set_permissions(p, m).map_err(|e| e.to_string())
}

fn remove_all(p: &Path) -> Result<(), String> {
    if std::fs::remove_dir_all(p).is_err() {
        writable(p)?;
        std::fs::remove_dir_all(p).map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn extract(tb: &Path, dest: &Path) -> Result<(), String> {
    unpack(tb, dest)?;
    apply_modes(tb, dest)
}

pub fn ensure(
    distro: &str,
    version: &str,
    spec: &crate::distro::Spec,
    cache: Option<&str>,
    refresh: bool,
) -> Result<PathBuf, String> {
    if version.is_empty() || version.starts_with('.') || version.contains('/') {
        return Err("bad version".to_string());
    }
    let arch = crate::distro::host_arch().ok_or("unsupported host arch".to_string())?;
    let dir = match cache {
        Some(c) => PathBuf::from(c),
        None => base_dir(),
    }
    .join("rootfs")
    .join(distro)
    .join(version)
    .join(arch);
    let root = dir.join("rootfs");
    let ok = dir.join(".ok");
    if !refresh && ok.exists() && root.join("bin").exists() {
        eprintln!("using cached rootfs {}", root.display());
        return Ok(root);
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&ok);
    let tb = dir.join(&spec.file);
    eprintln!("downloading {}", spec.tarball_url);
    fetch(&spec.tarball_url, &tb)?;
    let txt = fetch_text(&spec.sha_url, &dir)?;
    eprintln!("verifying {}", tb.display());
    verify(&tb, &txt)?;
    let tmp = dir.join("rootfs.tmp");
    if tmp.exists() {
        remove_all(&tmp)?;
    }
    if root.exists() {
        remove_all(&root)?;
    }
    eprintln!("unpacking {} to {}", tb.display(), root.display());
    extract(&tb, &tmp)?;
    let src = match &spec.subdir {
        Some(s) => tmp.join(s),
        None => tmp,
    };
    std::fs::rename(&src, &root).map_err(|e| e.to_string())?;
    let rc = root.join("etc").join("resolv.conf");
    if !rc.exists() {
        std::fs::write(&rc, "nameserver 8.8.8.8\nnameserver 1.1.1.1\n")
            .map_err(|e| e.to_string())?;
    }
    if distro == "arch" {
        let ml = root.join("etc").join("pacman.d").join("mirrorlist");
        std::fs::write(&ml, "Server = https://geo.mirror.pkgbuild.com/$repo/os/$arch\n")
            .map_err(|e| e.to_string())?;
        let pc = root.join("etc").join("pacman.conf");
        let txt = std::fs::read_to_string(&pc).map_err(|e| e.to_string())?;
        let mut patched = String::new();
        for line in txt.lines() {
            let t = line.trim_start();
            if t.starts_with("SigLevel") && !t.starts_with("LocalFileSigLevel") {
                patched.push_str("SigLevel = Never\n");
            } else {
                patched.push_str(line);
                patched.push('\n');
            }
        }
        let patched = patched
            .replace("#DisableSandboxFilesystem", "DisableSandboxFilesystem")
            .replace("#DisableSandboxSyscalls", "DisableSandboxSyscalls");
        std::fs::write(&pc, patched).map_err(|e| e.to_string())?;
    }
    std::fs::write(&ok, "1").map_err(|e| e.to_string())?;
    Ok(root)
}
