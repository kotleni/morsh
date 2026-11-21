mod distro;
mod rootfs;
mod run;

struct Cmdline {
    show_help: bool,
    distro: Option<String>,
    distro_version: Option<String>,
    refresh: bool,
    display: bool,
    rootfs_cache: Option<String>,
    new_root: Option<String>,
    guest_argv: Vec<String>,
}

impl Default for Cmdline {
    fn default() -> Self {
        Self {
            show_help: false,
            distro: None,
            distro_version: None,
            refresh: false,
            display: false,
            rootfs_cache: None,
            new_root: None,
            guest_argv: Vec::new(),
        }
    }
}

fn print_help(prog: &str) {
    eprintln!("Usage: {prog} [OPTIONS] NEWROOT COMMAND [COMMAND_ARGS...]");
    eprintln!("       {prog} [OPTIONS] --distro NAME[:VERSION] [COMMAND [COMMAND_ARGS...]]");
    eprintln!("OPTIONS: ");
    eprintln!("        -h    --help                Show help");
    eprintln!("              --distro=NAME[:VER]     Use cached distro rootfs (alpine, arch, ubuntu, void, gentoo) instead of NEWROOT");
    eprintln!(
        "              --distro-release=VER    Distro version (alpine {}, arch {}, ubuntu {}, void {}, gentoo {} [latest resolves weekly timestamp])",
        distro::ALPINE_VERSION,
        distro::ARCH_VERSION,
        distro::UBUNTU_VERSION,
        distro::VOID_VERSION,
        distro::GENTOO_VERSION,
    );
    eprintln!("              --refresh               Re-download the distro rootfs");
    eprintln!(
        "              --display               Share host display (X11/Wayland) with the container"
    );
    eprintln!("              --rootfs-cache=DIR      Rootfs cache directory");
    eprintln!();
    eprintln!("NEWROOT:      the root directory of the container");
    eprintln!("COMMAND:      the command you want to execute in the container");
    eprintln!("COMMAND_ARGS: arguments of COMMAND");
}

fn split_long(arg: &str) -> (&str, Option<&str>) {
    match arg.find('=') {
        Some(eq) => (&arg[..eq], Some(&arg[eq + 1..])),
        None => (arg, None),
    }
}

fn parse_cmdline(args: &[String], cmdline: &mut Cmdline) -> bool {
    *cmdline = Cmdline::default();

    let mut i = 1;
    while i < args.len() {
        let arg = args[i].as_str();
        if !arg.starts_with('-') || arg == "-" {
            break;
        }
        if arg == "--" {
            i += 1;
            break;
        }
        if arg == "-h" {
            cmdline.show_help = true;
            return true;
        }
        if !arg.starts_with("--") {
            eprintln!("Unknown option {arg}");
            return false;
        }
        let (name, inline_val) = split_long(arg);
        if name == "--help" {
            cmdline.show_help = true;
            return true;
        }
        if name == "--refresh" {
            cmdline.refresh = true;
            i += 1;
            continue;
        }
        if name == "--display" {
            cmdline.display = true;
            i += 1;
            continue;
        }
        let val: &str = match inline_val {
            Some(v) => v,
            None => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Option {name} requires an argument");
                    return false;
                }
                args[i].as_str()
            }
        };
        match name {
            "--distro" => {
                let (n, v) = match val.split_once(':') {
                    Some((n, v)) => (n.to_string(), Some(v.to_string())),
                    None => (val.to_string(), None),
                };
                if n != "alpine"
                    && n != "arch"
                    && n != "ubuntu"
                    && n != "void"
                    && n != "gentoo"
                {
                    eprintln!("Unknown distro {n}");
                    return false;
                }
                cmdline.distro = Some(n);
                if let Some(v) = v {
                    cmdline.distro_version = Some(v);
                }
            }
            "--distro-release" => cmdline.distro_version = Some(val.to_string()),
            "--rootfs-cache" => cmdline.rootfs_cache = Some(val.to_string()),
            _ => {
                eprintln!("Unknown option {name}");
                return false;
            }
        }
        i += 1;
    }

    let rest = &args[i..];
    if cmdline.distro.is_some() {
        cmdline.guest_argv = rest.to_vec();
        return true;
    }
    if rest.len() >= 2 {
        cmdline.new_root = Some(rest[0].clone());
        cmdline.guest_argv = rest[1..].to_vec();
        return true;
    }
    if rest.is_empty() {
        eprintln!("Missing COMMAND argument");
        eprintln!("Missing NEWROOT argument");
    } else {
        eprintln!("Missing COMMAND argument");
    }
    false
}

fn push_file_bind(binds: &mut Vec<run::ExtraBind>, path: &str) {
    // Note: exists(), not is_file() — sockets (dbus, wayland) are valid bind sources.
    if std::path::Path::new(path).exists() {
        binds.push(run::ExtraBind {
            src: path.to_string(),
            dst: path.to_string(),
            dir: false,
        });
    }
}

fn push_dir_bind(binds: &mut Vec<run::ExtraBind>, path: &str) {
    if std::path::Path::new(path).is_dir() {
        binds.push(run::ExtraBind {
            src: path.to_string(),
            dst: path.to_string(),
            dir: true,
        });
    }
}

fn collect_display(env: &mut Vec<(String, String)>) -> Result<Vec<run::ExtraBind>, String> {
    let mut binds = Vec::new();
    let mut found = false;
    if let Ok(d) = std::env::var("DISPLAY") {
        if !d.is_empty() {
            found = true;
            env.push(("DISPLAY".to_string(), d));
            if std::path::Path::new("/tmp/.X11-unix").is_dir() {
                binds.push(run::ExtraBind {
                    src: "/tmp/.X11-unix".to_string(),
                    dst: "/tmp/.X11-unix".to_string(),
                    dir: true,
                });
            } else {
                eprintln!("warning: /tmp/.X11-unix not found, X11 may fall back to TCP");
            }
            let cookie = std::env::var("XAUTHORITY")
                .ok()
                .filter(|p| std::path::Path::new(p).is_file())
                .or_else(|| {
                    std::env::var("HOME")
                        .ok()
                        .map(|h| format!("{h}/.Xauthority"))
                        .filter(|p| std::path::Path::new(p).is_file())
                });
            if let Some(c) = cookie {
                env.push(("XAUTHORITY".to_string(), "/tmp/.dshell-xauth".to_string()));
                binds.push(run::ExtraBind {
                    src: c,
                    dst: "/tmp/.dshell-xauth".to_string(),
                    dir: false,
                });
            }
        }
    }
    if let Ok(w) = std::env::var("WAYLAND_DISPLAY") {
        if !w.is_empty() {
            let rdir = std::env::var("XDG_RUNTIME_DIR")
                .unwrap_or_else(|_| format!("/run/user/{}", unsafe { libc::getuid() }));
            let (host, disp) = if w.starts_with('/') {
                (w.clone(), w.clone())
            } else {
                (format!("{rdir}/{w}"), w.clone())
            };
            if !std::path::Path::new(&host).exists() {
                return Err(format!("wayland socket {host} not found"));
            }
            found = true;
            env.push(("WAYLAND_DISPLAY".to_string(), disp));
            env.push(("XDG_RUNTIME_DIR".to_string(), rdir));
            binds.push(run::ExtraBind {
                src: host.clone(),
                dst: host,
                dir: false,
            });
        }
    }
    if !found {
        return Err("no display found (set DISPLAY or WAYLAND_DISPLAY)".to_string());
    }
    // D-Bus session bus (KDE/Qt apps segfault or misbehave without it).
    // Only forward unix:path sockets; TCP/host addresses are left as-is.
    if let Ok(bus) = std::env::var("DBUS_SESSION_BUS_ADDRESS") {
        if !bus.is_empty() {
            env.push(("DBUS_SESSION_BUS_ADDRESS".to_string(), bus.clone()));
            for part in bus.split(';') {
                if let Some(p) = part.strip_prefix("unix:path=") {
                    let sock = p.split(',').next().unwrap_or("");
                    if !sock.is_empty() && std::path::Path::new(sock).exists() {
                        binds.push(run::ExtraBind {
                            src: sock.to_string(),
                            dst: sock.to_string(),
                            dir: false,
                        });
                    }
                }
            }
        }
    }
    // machine-id is required by dbus/glib (kitty explicitly errors without it).
    push_file_bind(&mut binds, "/etc/machine-id");
    // System bus for hosted apps that talk to logind/UPower/etc.
    push_file_bind(&mut binds, "/run/dbus/system_bus_socket");
    // Fonts: Alpine minirootfs ships fontconfig but no fonts, so
    // xterm/kitty/konsole all fail (FcFontMatch, missing fixed font).
    push_dir_bind(&mut binds, "/usr/share/fonts");
    // GPU for Qt (konsole segfaults on llvmpipe-less fallback in some setups).
    push_dir_bind(&mut binds, "/dev/dri");
    // Let Qt pick the right platform (wayland vs xcb) like on the host.
    for key in [
        "XDG_CURRENT_DESKTOP",
        "XDG_SESSION_TYPE",
        "DESKTOP_SESSION",
        "KDE_SESSION_VERSION",
    ] {
        if let Ok(v) = std::env::var(key) {
            if !v.is_empty() {
                env.push((key.to_string(), v));
            }
        }
    }
    Ok(binds)
}

fn run() -> i32 {
    let args: Vec<String> = std::env::args().collect();
    let prog = args
        .first()
        .cloned()
        .unwrap_or_else(|| "dshell".to_string());
    let mut cmdline = Cmdline::default();

    if !parse_cmdline(&args, &mut cmdline) {
        println!();
        print_help(&prog);
        return -1;
    }
    if cmdline.show_help {
        print_help(&prog);
        return 0;
    }

    let is_distro = cmdline.distro.is_some();
    if is_distro {
        let name = cmdline.distro.clone().unwrap();
        let mut version = cmdline
            .distro_version
            .clone()
            .unwrap_or_else(|| distro::default_version(&name).unwrap().to_string());
        // Gentoo stage3 filenames embed a weekly timestamp, so "latest"
        // is resolved to a concrete timestamp first (also used for the cache dir).
        let mut latest: Option<distro::Spec> = None;
        if name == "gentoo" && version == "latest" {
            match rootfs::resolve_gentoo_latest() {
                Ok((ts, spec)) => {
                    version = ts;
                    latest = Some(spec);
                }
                Err(e) => {
                    eprintln!("{e}");
                    return -1;
                }
            }
        }
        let spec = match latest {
            Some(s) => s,
            None => match distro::spec(&name, &version) {
                Some(s) => s,
                None => {
                    eprintln!("Bad distro version {version}");
                    return -1;
                }
            },
        };
        let root = match rootfs::ensure(
            &name,
            &version,
            &spec,
            cmdline.rootfs_cache.as_deref(),
            cmdline.refresh,
        ) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("{e}");
                return -1;
            }
        };
        cmdline.new_root = Some(root.to_string_lossy().to_string());
        if cmdline.guest_argv.is_empty() {
            cmdline.guest_argv = vec![spec.shell.to_string()];
        }
    }

    let root = cmdline.new_root.clone().unwrap_or_default();
    let (mut env, workdir) = if is_distro {
        let term = std::env::var("TERM").unwrap_or_else(|_| "xterm-256color".to_string());
        (
            vec![
                (
                    "PATH".to_string(),
                    "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string(),
                ),
                ("HOME".to_string(), "/root".to_string()),
                ("TERM".to_string(), term),
            ],
            "/root",
        )
    } else {
        (vec![("TEST".to_string(), "works".to_string())], "/")
    };
    let extra = if cmdline.display {
        match collect_display(&mut env) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("{e}");
                return -1;
            }
        }
    } else {
        Vec::new()
    };
    run::enter(
        std::path::Path::new(&root),
        &cmdline.guest_argv,
        &env,
        workdir,
        &extra,
    )
}

fn main() {
    let code = run();
    std::process::exit(code);
}
