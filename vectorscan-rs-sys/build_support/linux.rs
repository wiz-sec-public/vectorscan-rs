//! Give CMake stable paths despite Cargo changing OUT_DIR between builds.
//! The mount is private to a child process; Cargo keeps using the real paths.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use anyhow::{Context, Result};
use nix::{
    errno::Errno,
    libc,
    mount::{mount, MsFlags},
    sched::{unshare, CloneFlags},
    sys::wait::{waitpid, WaitStatus},
    unistd::{fork, getgid, getuid, ForkResult},
};

fn setup_environment(out_dir: &Path, target_dir: &Path) -> Result<()> {
    let uid = getuid();
    let gid = getgid();

    unshare(CloneFlags::CLONE_NEWUSER | CloneFlags::CLONE_NEWNS)
        .context("unshare user and mount namespaces")?;
    fs::write("/proc/self/setgroups", b"deny").context("deny setgroups")?;
    fs::write("/proc/self/uid_map", format!("{uid} {uid} 1")).context("map uid")?;
    fs::write("/proc/self/gid_map", format!("{gid} {gid} 1")).context("map gid")?;

    mount(
        None::<&str>,
        "/",
        None::<&str>,
        MsFlags::MS_PRIVATE | MsFlags::MS_REC,
        None::<&str>,
    )
    .context("make mount propagation private")?;
    mount(
        Some(out_dir),
        target_dir,
        None::<&str>,
        MsFlags::MS_BIND,
        None::<&str>,
    )
    .context("bind mount build directory")?;

    Ok(())
}

/// Run the build under a stable mount, falling back if namespace setup fails.
/// This must be called before the build script starts any threads.
pub fn run_contained(out_dir: &Path, build: impl FnOnce(&Path)) {
    // Do not duplicate buffered Cargo directives across fork, or lose the
    // child's final output when it exits without running destructors.
    io::stdout().flush().expect("flush build output before fork");
    // SAFETY: The build script is single-threaded when entering this function.
    match unsafe { fork().expect("fork build process") } {
        ForkResult::Parent { child } => loop {
            match waitpid(child, None) {
                Ok(WaitStatus::Exited(_, 0)) => return,
                Ok(WaitStatus::Exited(_, 1)) => {
                    eprintln!("Falling back to standard build");
                    build(out_dir);
                    return;
                }
                Err(Errno::EINTR) => continue,
                Ok(status) => panic!("Contained Vectorscan build failed: {status:?}"),
                Err(error) => panic!("Failed to wait for Vectorscan build: {error}"),
            }
        },
        ForkResult::Child => {
            // /var exists on Linux and leaves the source checkout, compiler,
            // temporary files and the user's compiler cache accessible.
            let target_dir = Path::new("/var");
            if let Err(error) = setup_environment(out_dir, target_dir) {
                eprintln!("Failed to set up deterministic build directory: {error:#}");
                // The parent retries in its unchanged namespaces.
                unsafe { libc::_exit(1) };
            }

            build(target_dir);
            io::stdout().flush().expect("flush contained build output");
            unsafe { libc::_exit(0) };
        }
    }
}
