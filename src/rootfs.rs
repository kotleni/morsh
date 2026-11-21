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

/// Resolve Gentoo "latest" to a concrete (timestamp, Spec).
/// The autobuilds `current-stage3-*` dirs keep only the newest build and the
/// timestamp is baked into the filename, so a pinned version rots weekly.
/// `latest-stage3-*.txt` (PGP-signed) carries the current tarball name:
/// `stage3-amd64-openrc-20261115T234600Z.tar.xz 277024856`.
pub fn resolve_gentoo_latest() -> Result<(String, crate::distro::Spec), String> {
    let (rel, variant) =
        crate::distro::gentoo_variant().ok_or("unsupported host arch".to_string())?;
    let base = format!(
        "https://distfiles.gentoo.org/releases/{rel}/autobuilds/current-stage3-{variant}"
    );
    let dir = std::env::temp_dir().join("dshell-gentoo-latest");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let txt = fetch_text(&format!("{base}/latest-stage3-{variant}.txt"), &dir)?;
    let prefix = format!("stage3-{variant}-");
    for line in txt.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with('#')
            || line.starts_with("-----")
            || line.starts_with("Hash:")
        {
            continue;
        }
        let file = line.split_whitespace().next().unwrap_or("");
        if file.starts_with(&prefix) && file.ends_with(".tar.xz") {
            let version = file[prefix.len()..file.len() - ".tar.xz".len()].to_string();
            if version.is_empty() || version.contains('/') {
                continue;
            }
            let spec = crate::distro::gentoo(&version).ok_or("bad gentoo version".to_string())?;
            return Ok((version, spec));
        }
    }
    Err("bad gentoo latest file".to_string())
}

fn verify(tb: &Path, txt: &str) -> Result<(), String> {
    let base = tb.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let is_hash = |h: &str| h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit());
    let mut want: Option<String> = None;
    let mut fallback: Option<String> = None;
    for raw in txt.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // BSD style: SHA256 (file) = hash (used by Void Linux)
        if let (Some(op), Some(cp), Some(eq)) = (line.find('('), line.find(')'), line.find('=')) {
            if op < cp && cp < eq {
                let f = line[op + 1..cp].trim();
                let h = line[eq + 1..]
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim();
                let fname = Path::new(f).file_name().and_then(|s| s.to_str()).unwrap_or(f);
                if (fname == base || f.trim_start_matches('*') == base) && is_hash(h) {
                    want = Some(h.to_string());
                    break;
                }
                continue;
            }
        }
        // GNU style: hash [+*]file (Alpine, Arch, Ubuntu)
        let mut p = line.split_whitespace();
        match (p.next(), p.next()) {
            (Some(h), Some(_)) if is_hash(h) => {
                let f = line[h.len()..].trim().trim_start_matches('*').trim();
                let fname = Path::new(f).file_name().and_then(|s| s.to_str()).unwrap_or(f);
                if fname == base || f == base {
                    want = Some(h.to_string());
                    break;
                }
            }
            (Some(h), None) => {
                if fallback.is_none() && is_hash(h.trim()) {
                    fallback = Some(h.trim().to_string());
                }
            }
            _ => {}
        }
    }
    let want = want.or(fallback).ok_or("bad sha256 file".to_string())?;
    let data = std::fs::read(tb).map_err(|e| e.to_string())?;
    let sum = sha2::Sha256::digest(&data);
    let got: String = sum.iter().map(|b| format!("{b:02x}")).collect();
    if got.eq_ignore_ascii_case(&want) {
        Ok(())
    } else {
        Err("sha256 mismatch".to_string())
    }
}

fn reader(tb: &Path) -> Result<Box<dyn std::io::Read>, String> {
    let f = std::fs::File::open(tb).map_err(|e| e.to_string())?;
    let name = tb.file_name().and_then(|s| s.to_str()).unwrap_or("");
    if name.ends_with(".xz") {
        Ok(Box::new(xz2::read::XzDecoder::new(f)))
    } else if name.ends_with(".zst") {
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

fn run_patches(distro: &str, root: &Path) -> Result<(), String> {
    for (name, body) in crate::distro::patches(distro) {
        eprintln!("patch {name}");
        let st = Command::new("sh")
            .arg("-c")
            .arg(body)
            .arg(name)
            .arg(root)
            .status()
            .map_err(|e| format!("patch {name}: {e}"))?;
        if !st.success() {
            return Err(format!("patch {name} failed: {st}"));
        }
    }
    Ok(())
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
    run_patches(distro, &root)?;
    std::fs::write(&ok, "1").map_err(|e| e.to_string())?;
    Ok(root)
}
