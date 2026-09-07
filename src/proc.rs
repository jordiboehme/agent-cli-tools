//! Process enumeration on macOS: every process the kernel will name, with
//! the details a matcher needs. `sysctl(KERN_PROC_ALL)` is the only source
//! that lists other users' processes without privileges, so the pid and the
//! short name come from there; the executable path and the argument vector
//! are looked up per process and are allowed to be missing, because the
//! kernel withholds an argument vector from a process it does not own.
//!
//! All of the crate's unsafe FFI lives here.

use libc::{c_char, c_int, c_uint, c_void};

/// One process, as much of it as the kernel would hand over.
#[derive(Debug, Clone)]
pub struct Process {
    pub pid: i32,
    /// The kernel's short name: the basename of the executed file, cut to
    /// 16 bytes. Always available, for every process.
    pub comm: String,
    /// The fully resolved executable path, absent when the kernel refuses
    /// it (a handful of system processes) or the process is gone.
    pub exe: Option<String>,
    /// The argument vector. `None` means unreadable, which is what a
    /// process belonging to another user looks like; `Some` of an empty
    /// vector means the process genuinely has no arguments.
    pub argv: Option<Vec<String>>,
}

/// `MAXCOMLEN` from `<sys/param.h>`: Darwin remembers 16 characters of a
/// command name, and `p_comm` holds them plus the NUL. Linux cuts at 15,
/// which is where the number in most `pidof` lore comes from.
const MAXCOMLEN: usize = 16;

/// `struct extern_proc` from `<sys/proc.h>`, the head of a `kinfo_proc`.
/// Only `p_pid` and `p_comm` are ever read; every other field is here so
/// that the offsets of those two, and the stride between table entries,
/// match what the kernel writes. The layout is identical on
/// `aarch64-apple-darwin` and `x86_64-apple-darwin`, which the assertions
/// below pin down.
#[repr(C)]
#[allow(dead_code)]
struct ExternProc {
    /// A union of a run-queue link pair or a start `timeval`, never read.
    p_un: [u8; 16],
    p_vmspace: *mut c_void,
    p_sigacts: *mut c_void,
    p_flag: c_int,
    p_stat: c_char,
    p_pid: libc::pid_t,
    p_oppid: libc::pid_t,
    p_dupfd: c_int,
    user_stack: *mut c_char,
    exit_thread: *mut c_void,
    p_debugger: c_int,
    /// `boolean_t`, which is a plain `int` on Darwin.
    sigwait: c_int,
    p_estcpu: c_uint,
    p_cpticks: c_int,
    /// `fixpt_t`.
    p_pctcpu: u32,
    p_wchan: *mut c_void,
    p_wmesg: *mut c_char,
    p_swtime: c_uint,
    p_slptime: c_uint,
    p_realtimer: libc::itimerval,
    p_rtime: libc::timeval,
    p_uticks: u64,
    p_sticks: u64,
    p_iticks: u64,
    p_traceflag: c_int,
    p_tracep: *mut c_void,
    p_siglist: c_int,
    p_textvp: *mut c_void,
    p_holdcnt: c_int,
    p_sigmask: libc::sigset_t,
    p_sigignore: libc::sigset_t,
    p_sigcatch: libc::sigset_t,
    p_priority: u8,
    p_usrpri: u8,
    p_nice: c_char,
    /// `char p_comm[MAXCOMLEN+1]` in the header; kept as bytes because
    /// that is how it is read, and `u8` and `c_char` share a layout.
    p_comm: [u8; MAXCOMLEN + 1],
    p_pgrp: *mut c_void,
    p_addr: *mut c_void,
    p_xstat: u16,
    p_acflag: u16,
    p_ru: *mut c_void,
}

/// `struct kinfo_proc` from `<sys/sysctl.h>`, one entry of the process
/// table. `kp_eproc` is opaque here: nothing in it is needed, and its inner
/// structures are far more intricate than its size, which is all that the
/// stride depends on.
#[repr(C)]
#[allow(dead_code)]
struct KinfoProc {
    kp_proc: ExternProc,
    kp_eproc: [u8; 352],
}

// The table is read by striding over raw kernel bytes, so a transcription
// slip would silently yield garbage rather than fail. These pin the three
// numbers that matter against the values measured from the system headers.
const _: () = assert!(size_of::<KinfoProc>() == 648);
const _: () = assert!(size_of::<ExternProc>() == 296);
const _: () = assert!(std::mem::offset_of!(KinfoProc, kp_proc.p_pid) == 40);
const _: () = assert!(std::mem::offset_of!(KinfoProc, kp_proc.p_comm) == 243);

/// The buffer size to fall back on when `KERN_ARGMAX` cannot be read: the
/// 1 MiB Darwin actually reports.
const ARGMAX_FALLBACK: usize = 1 << 20;

/// Every process on the machine, sorted by pid, descending.
pub fn all() -> Vec<Process> {
    let table = process_table();
    let stride = size_of::<KinfoProc>();
    let count = table.len() / stride;

    // Both scratch buffers are reused across the whole table: `argmax` is a
    // megabyte and there are hundreds of processes.
    let mut args = vec![0u8; argmax()];
    let mut path = [0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];

    let mut processes = Vec::with_capacity(count);
    for i in 0..count {
        // SAFETY: `i < count`, so the entry lies wholly inside `table`, and
        // the read is unaligned because `table` is a byte buffer while
        // `KinfoProc` wants 8-byte alignment. Every bit pattern is a valid
        // `KinfoProc`: it holds only integers, raw pointers and byte arrays.
        let entry: KinfoProc =
            unsafe { std::ptr::read_unaligned(table.as_ptr().add(i * stride).cast()) };
        let pid = entry.kp_proc.p_pid;
        processes.push(Process {
            pid,
            comm: string_until_nul(&entry.kp_proc.p_comm),
            exe: exe_of(pid, &mut path),
            argv: argv_of(pid, &mut args),
        });
    }

    processes.sort_unstable_by_key(|p| std::cmp::Reverse(p.pid));
    processes
}

/// The raw `kinfo_proc` table. Sizing and fetching are two separate calls,
/// so the table can grow in between and the fetch fail with `ENOMEM`; one
/// retry re-sizes and tries again. Any other failure yields no processes
/// rather than a panic, since a caller can do nothing about it either way.
fn process_table() -> Vec<u8> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROC, libc::KERN_PROC_ALL, 0];
    for _ in 0..2 {
        let mut size: libc::size_t = 0;
        // SAFETY: `mib` holds the 4 elements named by the length argument,
        // and a null buffer with a non-null length asks sysctl for the size
        // it would need.
        let rc = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                mib.len() as c_uint,
                std::ptr::null_mut(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if rc != 0 || size == 0 {
            return Vec::new();
        }
        let mut buffer = vec![0u8; size];
        let mut written = size;
        // SAFETY: the buffer holds `written` bytes, which is what sysctl is
        // told it may fill; on return `written` is what it actually filled.
        let rc = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                mib.len() as c_uint,
                buffer.as_mut_ptr().cast(),
                &mut written,
                std::ptr::null_mut(),
                0,
            )
        };
        if rc == 0 {
            buffer.truncate(written);
            return buffer;
        }
        if errno() != libc::ENOMEM {
            return Vec::new();
        }
    }
    Vec::new()
}

/// The kernel's limit on the size of an argument area, which is how big a
/// `KERN_PROCARGS2` answer can be.
fn argmax() -> usize {
    let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
    let mut value: c_int = 0;
    let mut size = size_of::<c_int>();
    // SAFETY: `mib` holds the 2 elements named by the length argument, and
    // the output buffer is exactly the `int` this sysctl returns.
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as c_uint,
            (&raw mut value).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc == 0 && value > 0 {
        value as usize
    } else {
        ARGMAX_FALLBACK
    }
}

/// The resolved executable path. `proc_pidpath` reports a length, but the
/// answer is NUL-terminated either way, so only success or failure is taken
/// from the return value.
fn exe_of(pid: i32, buffer: &mut [u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize]) -> Option<String> {
    // SAFETY: the buffer is exactly PROC_PIDPATHINFO_MAXSIZE bytes, the
    // maximum proc_pidpath is documented to write, and that same size is
    // what it is told it has.
    let rc = unsafe { libc::proc_pidpath(pid, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
    if rc <= 0 {
        return None;
    }
    Some(string_until_nul(buffer))
}

/// The argument vector, from the `KERN_PROCARGS2` blob: a 4-byte `argc`,
/// the NUL-terminated executable path, NUL padding, then `argc`
/// NUL-terminated arguments and finally the environment, which is ignored.
/// The call fails with `EINVAL` for a process owned by another user, which
/// is a refusal rather than an error and so reads as `None`.
fn argv_of(pid: i32, buffer: &mut [u8]) -> Option<Vec<String>> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut size = buffer.len();
    // SAFETY: `mib` holds the 3 elements named by the length argument, and
    // the buffer holds the `size` bytes sysctl is told it may fill.
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as c_uint,
            buffer.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || size < size_of::<c_int>() {
        return None;
    }

    let blob = &buffer[..size];
    let argc = c_int::from_ne_bytes(blob[..4].try_into().expect("four bytes"));
    if argc < 0 {
        return None;
    }
    let mut at = 4;
    // Step past the executable path and the NUL padding that follows it.
    while at < blob.len() && blob[at] != 0 {
        at += 1;
    }
    while at < blob.len() && blob[at] == 0 {
        at += 1;
    }

    let mut argv = Vec::with_capacity(argc as usize);
    for _ in 0..argc {
        if at >= blob.len() {
            break;
        }
        let start = at;
        while at < blob.len() && blob[at] != 0 {
            at += 1;
        }
        argv.push(String::from_utf8_lossy(&blob[start..at]).into_owned());
        at += 1;
    }
    Some(argv)
}

/// A C string held in a fixed-size buffer, up to its first NUL. Invalid
/// UTF-8 is replaced rather than rejected, so a process with odd bytes in
/// its name still appears in the listing.
fn string_until_nul(buffer: &[u8]) -> String {
    let end = buffer.iter().position(|&b| b == 0).unwrap_or(buffer.len());
    String::from_utf8_lossy(&buffer[..end]).into_owned()
}

fn errno() -> c_int {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_this_process_with_its_own_details() {
        let me = std::process::id() as i32;
        let all = all();
        let mine = all
            .iter()
            .find(|p| p.pid == me)
            .expect("this process is listed");
        let exe = std::env::current_exe().unwrap();
        let name = exe.file_name().unwrap().to_string_lossy().into_owned();
        // comm is truncated to MAXCOMLEN bytes, so compare on the shorter
        // of the two
        let cut = name.len().min(16);
        assert_eq!(mine.comm, name[..cut]);
        assert_eq!(mine.exe.as_deref(), Some(exe.to_str().unwrap()));
        assert!(!mine.argv.as_ref().expect("own argv is readable")[0].is_empty());
    }

    #[test]
    fn lists_launchd_and_sorts_descending() {
        let all = all();
        assert!(all.iter().any(|p| p.pid == 1), "launchd is listed");
        assert!(
            all.windows(2).all(|w| w[0].pid > w[1].pid),
            "sorted descending"
        );
        // Another user's argv may be unreadable, but comm is always available
        assert!(all.iter().filter(|p| p.pid > 0).all(|p| !p.comm.is_empty()));
    }

    #[test]
    fn reads_the_arguments_of_a_child_process() {
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        let pid = child.id() as i32;
        let found = all()
            .into_iter()
            .find(|p| p.pid == pid)
            .expect("child is listed");
        child.kill().ok();
        child.wait().ok();
        assert_eq!(found.comm, "sleep");
        assert_eq!(found.exe.as_deref(), Some("/bin/sleep"));
        assert_eq!(found.argv.as_ref().unwrap()[1], "30");
    }
}
