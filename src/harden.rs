//! process-wide settings, made before anything secret is in memory
#![allow(unsafe_code)]

/// a process under the same uid on the host could otherwise read our memory, and the container's
/// files with the key among them, through /proc/<pid>. the image keeps the old image's uid, nobody,
/// because the `reported` volumes of existing installs belong to it, and host services may run as it too
pub fn no_inspection() {
    // SAFETY: sets a flag on this process, passes no pointers
    #[cfg(target_os = "linux")]
    {
        let rc = unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) };
        if rc != 0 {
            // a seccomp profile or a missing privilege can refuse this. keep running - the tarpit is the
            // point - but say the key is readable through /proc by anyone with our uid
            let e = std::io::Error::last_os_error();
            log!("warning: could not make the process non-dumpable, the api token may be readable: {e}");
        }
    }
}

extern "C" fn exit_now(signal: libc::c_int) {
    // SAFETY: _exit is async-signal-safe and touches no state of the interrupted thread
    unsafe { libc::_exit(128 + signal) }
}

/// as pid 1 the kernel drops signals nobody handles, so `docker stop` would wait out its timeout
pub fn exit_on_signals() {
    for signal in [libc::SIGTERM, libc::SIGINT] {
        let handler: extern "C" fn(libc::c_int) = exit_now;
        // SAFETY: the handler only calls _exit
        unsafe {
            libc::signal(signal, handler as libc::sighandler_t);
        }
    }
}
