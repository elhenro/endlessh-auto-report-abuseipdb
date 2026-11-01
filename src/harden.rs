//! process-wide settings, made before anything secret is in memory
#![allow(unsafe_code)]

/// a process under the same uid on the host could otherwise read our memory, and the container's
/// files with the key among them, through /proc/<pid>. the image keeps the old image's uid, nobody,
/// because the `reported` volumes of existing installs belong to it, and host services may run as it too
pub fn no_inspection() {
    // SAFETY: sets a flag on this process, passes no pointers
    #[cfg(target_os = "linux")]
    unsafe {
        libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0);
    }
}

extern "C" fn exit_now(signal: libc::c_int) {
    // SAFETY: _exit is async-signal-safe and touches no state of the interrupted thread
    unsafe { libc::_exit(128 + signal) }
}

/// without an init in the container this is pid 1, where the kernel drops signals nobody handles:
/// `docker stop` would wait out its timeout. exit codes as the old reporter: 143 and 130
pub fn exit_on_signals() {
    for signal in [libc::SIGTERM, libc::SIGINT] {
        let handler: extern "C" fn(libc::c_int) = exit_now;
        // SAFETY: the handler only calls _exit
        unsafe {
            libc::signal(signal, handler as libc::sighandler_t);
        }
    }
}
