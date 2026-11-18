mod distro;
mod rootfs;
mod run;

struct Cmdline {
    show_help: bool,
    distro: Option<String>,
    distro_version: Option<String>,
    refresh: bool,
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
    eprintln!("              --distro=NAME[:VER]     Use cached distro rootfs (alpine, arch) instead of NEWROOT");
    eprintln!("              --distro-release=VER    Distro version (alpine {}, arch {})", distro::ALPINE_VERSION, distro::ARCH_VERSION);
    eprintln!("              --refresh               Re-download the distro rootfs");
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
                if n != "alpine" && n != "arch" {
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

fn run() -> i32 {
    let args: Vec<String> = std::env::args().collect();
    let prog = args.first().cloned().unwrap_or_else(|| "dshell".to_string());
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
        let version = cmdline
            .distro_version
            .clone()
            .unwrap_or_else(|| distro::default_version(&name).unwrap().to_string());
        let spec = match distro::spec(&name, &version) {
            Some(s) => s,
            None => {
                eprintln!("Bad distro version {version}");
                return -1;
            }
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
    let (env, workdir) = if is_distro {
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
    run::enter(std::path::Path::new(&root), &cmdline.guest_argv, &env, workdir)
}

fn main() {
    let code = run();
    std::process::exit(code);
}
