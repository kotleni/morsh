use std::ffi::CString;
use std::path::Path;

fn do_mount(
    src: Option<&str>,
    tgt: &str,
    fstype: Option<&str>,
    flags: libc::c_ulong,
    data: Option<&str>,
) -> Result<(), String> {
    let s = src
        .map(CString::new)
        .transpose()
        .map_err(|_| "bad mount source".to_string())?;
    let t = CString::new(tgt).map_err(|_| "bad mount target".to_string())?;
    let f = fstype
        .map(CString::new)
        .transpose()
        .map_err(|_| "bad mount fstype".to_string())?;
    let d = data
        .map(CString::new)
        .transpose()
        .map_err(|_| "bad mount data".to_string())?;
    let r = unsafe {
        libc::mount(
            s.as_ref().map(|x| x.as_ptr()).unwrap_or(std::ptr::null()),
            t.as_ptr(),
            f.as_ref().map(|x| x.as_ptr()).unwrap_or(std::ptr::null()),
            flags,
            d.as_ref().map(|x| x.as_ptr()).unwrap_or(std::ptr::null()) as *const libc::c_void,
        )
    };
    if r < 0 {
        return Err(format!(
            "mount {tgt}: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

fn bind(src: &str, tgt: &str) -> Result<(), String> {
    do_mount(Some(src), tgt, None, (libc::MS_BIND | libc::MS_REC) as _, None)
}

pub struct ExtraBind {
    pub src: String,
    pub dst: String,
    pub dir: bool,
}

fn tmpfs(tgt: &str) -> Result<(), String> {
    do_mount(
        Some("tmpfs"),
        tgt,
        Some("tmpfs"),
        (libc::MS_NOSUID | libc::MS_NODEV) as _,
        None,
    )
}

fn status_code(st: i32) -> i32 {
    if libc::WIFEXITED(st) {
        libc::WEXITSTATUS(st)
    } else if libc::WIFSIGNALED(st) {
        128 + libc::WTERMSIG(st)
    } else {
        -1
    }
}

fn wait_pid(pid: libc::pid_t) -> i32 {
    let mut st = 0;
    loop {
        let r = unsafe { libc::waitpid(pid, &mut st, 0) };
        if r >= 0 {
            return status_code(st);
        }
        if std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
            eprintln!("waitpid: {}", std::io::Error::last_os_error());
            return -1;
        }
    }
}

fn grandchild(
    root: &str,
    argv: &[CString],
    env: &[(String, String)],
    workdir: &str,
    extra: &[ExtraBind],
) -> ! {
    let fail = |msg: String| -> ! {
        eprintln!("{msg}");
        unsafe { libc::_exit(1) };
    };
    if let Err(e) = do_mount(None, "/", None, (libc::MS_REC | libc::MS_PRIVATE) as _, None) {
        fail(e);
    }
    if let Err(e) = bind(root, root) {
        fail(e);
    }
    let _ = std::fs::remove_dir(Path::new(root).join("oldroot"));
    if std::fs::create_dir(Path::new(root).join("oldroot")).is_err() {
        fail("mkdir oldroot".to_string());
    }
    let nr = CString::new(root).unwrap();
    let put_old = format!("{root}/oldroot");
    let po = CString::new(put_old).unwrap();
    let r = unsafe { libc::syscall(libc::SYS_pivot_root, nr.as_ptr(), po.as_ptr()) };
    if r < 0 {
        fail(format!("pivot_root: {}", std::io::Error::last_os_error()));
    }
    if unsafe { libc::chdir(CString::new("/").unwrap().as_ptr()) } < 0 {
        fail(format!("chdir: {}", std::io::Error::last_os_error()));
    }
    let step = |r: Result<(), String>| {
        if let Err(e) = r {
            fail(e);
        }
    };
    step(do_mount(
        Some("proc"),
        "/proc",
        Some("proc"),
        (libc::MS_NOSUID | libc::MS_NOEXEC | libc::MS_NODEV) as _,
        None,
    ));
    if do_mount(
        Some("sysfs"),
        "/sys",
        Some("sysfs"),
        (libc::MS_RDONLY | libc::MS_NOSUID | libc::MS_NOEXEC | libc::MS_NODEV) as _,
        None,
    )
    .is_err()
    {
        step(bind("/oldroot/sys", "/sys"));
        let _ = do_mount(
            None,
            "/sys",
            None,
            (libc::MS_BIND | libc::MS_REMOUNT | libc::MS_RDONLY) as _,
            None,
        );
    }
    step(tmpfs("/dev"));
    let _ = std::fs::create_dir_all("/dev/pts");
    for d in ["null", "zero", "full", "random", "urandom", "tty"] {
        let host = format!("/oldroot/dev/{d}");
        let tgt = format!("/dev/{d}");
        if !Path::new(&host).exists() {
            continue;
        }
        if std::fs::File::create(&tgt).is_err() {
            fail(format!("create {tgt}"));
        }
        step(do_mount(Some(&host), &tgt, None, libc::MS_BIND as _, None));
    }
    step(do_mount(
        Some("devpts"),
        "/dev/pts",
        Some("devpts"),
        (libc::MS_NOSUID | libc::MS_NOEXEC) as _,
        Some("newinstance,ptmxmode=0666,mode=0620"),
    ));
    let _ = std::os::unix::fs::symlink("pts/ptmx", "/dev/ptmx");
    let _ = std::fs::create_dir_all("/run/shm");
    step(tmpfs("/run"));
    let _ = std::fs::create_dir_all("/run/shm");
    let _ = std::os::unix::fs::symlink("../run/shm", "/dev/shm");
    let _ = std::os::unix::fs::symlink("/proc/self/fd/0", "/dev/stdin");
    let _ = std::os::unix::fs::symlink("/proc/self/fd/1", "/dev/stdout");
    let _ = std::os::unix::fs::symlink("/proc/self/fd/2", "/dev/stderr");
    if Path::new("/tmp").exists() {
        step(tmpfs("/tmp"));
    }
    if Path::new("/var/tmp").exists() {
        step(tmpfs("/var/tmp"));
    }
    for b in extra {
        let host = format!("/oldroot{}", b.src);
        if b.dir {
            let _ = std::fs::create_dir_all(&b.dst);
            step(bind(&host, &b.dst));
        } else {
            if let Some(p) = Path::new(&b.dst).parent() {
                let _ = std::fs::create_dir_all(p);
            }
            if std::fs::File::create(&b.dst).is_err() {
                fail(format!("create {}", b.dst));
            }
            step(do_mount(
                Some(&host),
                &b.dst,
                None,
                libc::MS_BIND as _,
                None,
            ));
        }
    }
    if unsafe { libc::umount2(CString::new("/oldroot").unwrap().as_ptr(), libc::MNT_DETACH) } < 0 {
        fail(format!("umount oldroot: {}", std::io::Error::last_os_error()));
    }
    let _ = std::fs::remove_dir("/oldroot");
    let h = CString::new("dshell").unwrap();
    unsafe {
        libc::sethostname(h.as_ptr(), 6);
    }
    if unsafe { libc::chdir(CString::new(workdir).unwrap().as_ptr()) } < 0 {
        fail(format!("chdir {workdir}: {}", std::io::Error::last_os_error()));
    }
    let mut argp: Vec<*const libc::c_char> = argv.iter().map(|a| a.as_ptr()).collect();
    argp.push(std::ptr::null());
    let cenv: Vec<CString> = env
        .iter()
        .map(|(k, v)| CString::new(format!("{k}={v}")).unwrap())
        .collect();
    let mut envp: Vec<*const libc::c_char> = cenv.iter().map(|e| e.as_ptr()).collect();
    envp.push(std::ptr::null());
    let path = env
        .iter()
        .find(|(k, _)| k == "PATH")
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string());
    let prog = &argv[0];
    let mut tried = false;
    let mut denied = false;
    if prog.to_string_lossy().contains('/') {
        unsafe {
            libc::execve(prog.as_ptr(), argp.as_ptr(), envp.as_ptr());
        }
    } else {
        for d in path.split(':') {
            if d.is_empty() {
                continue;
            }
            if let Ok(c) = CString::new(format!("{d}/{}", prog.to_string_lossy())) {
                unsafe {
                    libc::execve(c.as_ptr(), argp.as_ptr(), envp.as_ptr());
                }
                tried = true;
                if std::io::Error::last_os_error().raw_os_error() == Some(libc::EACCES) {
                    denied = true;
                }
            }
        }
    }
    unsafe {
        let e = if !tried {
            std::io::Error::from_raw_os_error(libc::ENOENT)
        } else if denied {
            std::io::Error::from_raw_os_error(libc::EACCES)
        } else {
            std::io::Error::last_os_error()
        };
        let prog = argv[0].to_string_lossy();
        eprintln!("Couldn't execute '{prog}': {e}");
        libc::_exit(127);
    }
}

pub fn enter(
    root: &Path,
    argv: &[String],
    env: &[(String, String)],
    workdir: &str,
    extra: &[ExtraBind],
) -> i32 {
    if argv.is_empty() {
        eprintln!("Missing COMMAND argument");
        return -1;
    }
    if argv.iter().any(|a| a.bytes().any(|b| b == 0)) {
        eprintln!("bad argument");
        return -1;
    }
    let root = match root.canonicalize() {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(e) => {
            eprintln!("{e}");
            return -1;
        }
    };
    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };
    let c_argv: Vec<CString> = argv.iter().map(|a| CString::new(a.as_str()).unwrap()).collect();
    let c_env: Vec<(String, String)> = env.to_vec();

    let mut ready = [0; 2];
    let mut go = [0; 2];
    if unsafe { libc::pipe(ready.as_mut_ptr()) } < 0 || unsafe { libc::pipe(go.as_mut_ptr()) } < 0
    {
        eprintln!("pipe: {}", std::io::Error::last_os_error());
        return -1;
    }
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        eprintln!("fork: {}", std::io::Error::last_os_error());
        return -1;
    }
    if pid == 0 {
        unsafe {
            libc::close(ready[0]);
            libc::close(go[1]);
        }
        if unsafe { libc::unshare(libc::CLONE_NEWUSER) } < 0 {
            eprintln!("unshare(user): {}", std::io::Error::last_os_error());
            unsafe { libc::_exit(1) };
        }
        let _ = std::fs::write("/proc/self/setgroups", "deny");
        if unsafe { libc::write(ready[1], c"1".as_ptr() as *const libc::c_void, 1) } != 1 {
            unsafe { libc::_exit(1) };
        }
        unsafe {
            libc::close(ready[1]);
        }
        let mut b = [0u8; 1];
        if unsafe { libc::read(go[0], b.as_mut_ptr() as *mut libc::c_void, 1) } != 1 {
            unsafe { libc::_exit(1) };
        }
        unsafe {
            libc::close(go[0]);
        }
        if unsafe { libc::unshare(libc::CLONE_NEWNS | libc::CLONE_NEWUTS | libc::CLONE_NEWPID) } < 0
        {
            eprintln!("unshare(ns): {}", std::io::Error::last_os_error());
            unsafe { libc::_exit(1) };
        }
        let pid2 = unsafe { libc::fork() };
        if pid2 < 0 {
            unsafe { libc::_exit(1) };
        }
        if pid2 == 0 {
            grandchild(&root, &c_argv, &c_env, workdir, extra);
        }
        unsafe { libc::_exit(wait_pid(pid2)) };
    }
    unsafe {
        libc::close(ready[1]);
        libc::close(go[0]);
    }
    let mut b = [0u8; 1];
    let n = unsafe { libc::read(ready[0], b.as_mut_ptr() as *mut libc::c_void, 1) };
    unsafe {
        libc::close(ready[0]);
    }
    if n != 1 {
        return wait_pid(pid);
    }
    fn username_for(uid: u32) -> Option<String> {
        unsafe {
            let pw = libc::getpwuid(uid);
            if pw.is_null() || (*pw).pw_name.is_null() {
                return None;
            }
            std::ffi::CStr::from_ptr((*pw).pw_name)
                .to_str()
                .ok()
                .map(|s| s.to_string())
        }
    }

    fn sub_range(path: &str, uid: u32, gid: u32, username: Option<&str>) -> Option<(u32, u32)> {
        let txt = std::fs::read_to_string(path).ok()?;
        let want_uid = uid.to_string();
        let want_gid = gid.to_string();
        let mut fallback = None;
        for line in txt.lines() {
            let mut p = line.split(':');
            let (name, start, count) = match (p.next(), p.next(), p.next()) {
                (Some(n), Some(s), Some(c)) => (n.trim(), s.trim(), c.trim()),
                _ => continue,
            };
            let (start, count): (u32, u32) = match (start.parse(), count.parse()) {
                (Ok(s), Ok(c)) => (s, c),
                _ => continue,
            };
            if let Some(u) = username {
                if name == u {
                    return Some((start, count));
                }
            }
            if name == want_uid || name == want_gid {
                return Some((start, count));
            }
            if fallback.is_none() {
                fallback = Some((start, count));
            }
        }
        // Single-user machine: only one entry, use it even if the key
        // style (user name vs numeric id) didn't match.
        fallback
    }

    let username = username_for(uid).or_else(|| std::env::var("USER").ok());
    let uid_sub = sub_range("/etc/subuid", uid, gid, username.as_deref());
    let gid_sub = sub_range("/etc/subgid", uid, gid, username.as_deref());
    let map = |helper: &str, id: u32, sub: Option<(u32, u32)>| -> bool {
        let mut args = vec![pid.to_string(), "0".to_string(), id.to_string(), "1".to_string()];
        if let Some((start, count)) = sub {
            args.push("1".to_string());
            args.push(start.to_string());
            args.push(count.to_string());
        }
        std::process::Command::new(helper)
            .args(&args)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };
    if map("newuidmap", uid, uid_sub) && map("newgidmap", gid, gid_sub) {
        unsafe {
            libc::write(go[1], c"1".as_ptr() as *const libc::c_void, 1);
            libc::close(go[1]);
        }
    } else {
        eprintln!("uid/gid mapping failed");
        unsafe {
            libc::close(go[1]);
        }
    }
    wait_pid(pid)
}
