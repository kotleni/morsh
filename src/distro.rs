pub const ALPINE_VERSION: &str = "3.21.3";
pub const ARCH_VERSION: &str = "2026.11.01";
pub const UBUNTU_VERSION: &str = "26.04";
pub const VOID_VERSION: &str = "20250202";
pub const GENTOO_VERSION: &str = "latest";

pub struct Spec {
    pub tarball_url: String,
    pub sha_url: String,
    pub file: String,
    pub shell: &'static str,
    pub subdir: Option<String>,
}

pub fn host_arch() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some("x86_64"),
        "aarch64" => Some("aarch64"),
        _ => None,
    }
}

pub fn alpine(version: &str) -> Option<Spec> {
    let arch = host_arch()?;
    let mut parts = version.split('.');
    let branch = format!("v{}.{}", parts.next()?, parts.next()?);
    let file = format!("alpine-minirootfs-{version}-{arch}.tar.gz");
    let base = format!("https://dl-cdn.alpinelinux.org/alpine/{branch}/releases/{arch}");
    Some(Spec {
        tarball_url: format!("{base}/{file}"),
        sha_url: format!("{base}/{file}.sha256"),
        file,
        shell: "/bin/ash",
        subdir: None,
    })
}

pub fn arch(version: &str) -> Option<Spec> {
    if version.is_empty() || host_arch()? != "x86_64" {
        return None;
    }
    let file = format!("archlinux-bootstrap-{version}-x86_64.tar.zst");
    let base = format!("https://geo.mirror.pkgbuild.com/iso/{version}");
    Some(Spec {
        tarball_url: format!("{base}/{file}"),
        sha_url: format!("{base}/sha256sums.txt"),
        file,
        shell: "/bin/bash",
        subdir: Some("root.x86_64".to_string()),
    })
}

pub fn ubuntu(version: &str) -> Option<Spec> {
    if version.is_empty() {
        return None;
    }

    let arch = host_arch()?;
    let patched_arch = match arch {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        _ => arch,
    };

    let file = format!("ubuntu-{version}-server-cloudimg-{patched_arch}-root.tar.xz");
    let base = "https://cloud-images.ubuntu.com/releases/resolute/release/";
    Some(Spec {
        tarball_url: format!("{base}/{file}"),
        sha_url: format!("{base}/SHA256SUMS"),
        file,
        shell: "/bin/bash",
        subdir: None,
    })
}

pub fn void_linux(version: &str) -> Option<Spec> {
    let arch = host_arch()?;
    let file = format!("void-{arch}-ROOTFS-{version}.tar.xz");
    let base = format!("https://repo-default.voidlinux.org/live/{version}/");
    Some(Spec {
        tarball_url: format!("{base}/{file}"),
        sha_url: format!("{base}/sha256sum.txt"),
        file,
        shell: "/bin/bash",
        subdir: None,
    })
}

/// (releases dir arch, stage3 variant) for this host.
pub fn gentoo_variant() -> Option<(&'static str, &'static str)> {
    match host_arch()? {
        "x86_64" => Some(("amd64", "amd64-openrc")),
        "aarch64" => Some(("arm64", "arm64-openrc")),
        _ => None,
    }
}

pub fn gentoo(version: &str) -> Option<Spec> {
    // "latest" is resolved over the network in rootfs::resolve_gentoo_latest
    if version == "latest" {
        return None;
    }
    if version.is_empty() || version.starts_with('.') || version.contains('/') {
        return None;
    }
    let (rel, variant) = gentoo_variant()?;
    let file = format!("stage3-{variant}-{version}.tar.xz");
    let base =
        format!("https://distfiles.gentoo.org/releases/{rel}/autobuilds/current-stage3-{variant}");
    Some(Spec {
        tarball_url: format!("{base}/{file}"),
        sha_url: format!("{base}/{file}.sha256"),
        file,
        shell: "/bin/bash",
        subdir: None,
    })
}

pub fn spec(name: &str, version: &str) -> Option<Spec> {
    match name {
        "alpine" => alpine(version),
        "arch" => arch(version),
        "ubuntu" => ubuntu(version),
        "void" => void_linux(version),
        "gentoo" => gentoo(version),
        _ => None,
    }
}

pub fn default_version(name: &str) -> Option<&'static str> {
    match name {
        "alpine" => Some(ALPINE_VERSION),
        "arch" => Some(ARCH_VERSION),
        "ubuntu" => Some(UBUNTU_VERSION),
        "void" => Some(VOID_VERSION),
        "gentoo" => Some(GENTOO_VERSION),
        _ => None,
    }
}

pub fn patches(name: &str) -> &'static [(&'static str, &'static str)] {
    match name {
        "arch" => &[
            ("resolv-conf.sh", include_str!("../patches/resolv-conf.sh")),
            (
                "pacman-mirrorlist.sh",
                include_str!("../patches/pacman-mirrorlist.sh"),
            ),
        ],
        "ubuntu" => &[
            ("resolv-conf.sh", include_str!("../patches/resolv-conf.sh")),
            ("apt-sandbox.sh", include_str!("../patches/apt-sandbox.sh")),
        ],
        "gentoo" => &[
            ("resolv-conf.sh", include_str!("../patches/resolv-conf.sh")),
            (
                "portage-userns.sh",
                include_str!("../patches/portage-userns.sh"),
            ),
        ],
        _ => &[("resolv-conf.sh", include_str!("../patches/resolv-conf.sh"))],
    }
}
