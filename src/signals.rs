//! Signal names and numbers the way GNU coreutils `operand2sig` and gnulib
//! `sig2str` handle them, restricted to the signals Darwin defines.

/// The highest signal number GNU accepts on the command line.
const SIGNUM_BOUND: i32 = 64;

/// Ordered like gnulib's table so that number-to-name lookups pick the same
/// name GNU prints: standard names first, XSI names, then the BSD extras.
/// SIGPOLL and SIGIOT are Darwin aliases for SIGEMT and SIGABRT.
const SIGNALS: &[(&str, i32)] = &[
    ("ABRT", libc::SIGABRT),
    ("ALRM", libc::SIGALRM),
    ("BUS", libc::SIGBUS),
    ("CHLD", libc::SIGCHLD),
    ("CONT", libc::SIGCONT),
    ("FPE", libc::SIGFPE),
    ("HUP", libc::SIGHUP),
    ("ILL", libc::SIGILL),
    ("INT", libc::SIGINT),
    ("KILL", libc::SIGKILL),
    ("PIPE", libc::SIGPIPE),
    ("QUIT", libc::SIGQUIT),
    ("SEGV", libc::SIGSEGV),
    ("STOP", libc::SIGSTOP),
    ("TERM", libc::SIGTERM),
    ("TSTP", libc::SIGTSTP),
    ("TTIN", libc::SIGTTIN),
    ("TTOU", libc::SIGTTOU),
    ("USR1", libc::SIGUSR1),
    ("USR2", libc::SIGUSR2),
    ("POLL", libc::SIGEMT),
    ("PROF", libc::SIGPROF),
    ("SYS", libc::SIGSYS),
    ("TRAP", libc::SIGTRAP),
    ("URG", libc::SIGURG),
    ("VTALRM", libc::SIGVTALRM),
    ("XCPU", libc::SIGXCPU),
    ("XFSZ", libc::SIGXFSZ),
    ("IOT", libc::SIGABRT),
    ("EMT", libc::SIGEMT),
    ("INFO", libc::SIGINFO),
    ("IO", libc::SIGIO),
    ("WINCH", libc::SIGWINCH),
    ("EXIT", 0),
];

/// Parse a signal operand: a name with or without the SIG prefix in any
/// case, or a decimal number. Numbers are masked the way GNU masks them, so
/// a shell exit status such as 143 maps back to TERM. `None` is GNU's
/// "invalid signal".
pub fn parse_signal(text: &str) -> Option<i32> {
    if text.as_bytes().first().is_some_and(u8::is_ascii_digit) {
        if !text.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let mut number: i32 = text.parse().ok()?;
        number &= if number >= 0xFF { 0xFF } else { 0x7F };
        return (0..=SIGNUM_BOUND).contains(&number).then_some(number);
    }
    let upper = text.to_ascii_uppercase();
    lookup(&upper).or_else(|| upper.strip_prefix("SIG").and_then(lookup))
}

fn lookup(name: &str) -> Option<i32> {
    SIGNALS.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
}

/// The name GNU prints for a signal, without the SIG prefix. `None` for 0
/// and for numbers without a name; callers print the number instead.
pub fn signal_name(signal: i32) -> Option<&'static str> {
    if signal == 0 {
        return None;
    }
    SIGNALS.iter().find(|(_, s)| *s == signal).map(|(n, _)| *n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_in_any_case_with_or_without_prefix() {
        assert_eq!(parse_signal("TERM"), Some(libc::SIGTERM));
        assert_eq!(parse_signal("term"), Some(libc::SIGTERM));
        assert_eq!(parse_signal("SIGTERM"), Some(libc::SIGTERM));
        assert_eq!(parse_signal("sigkill"), Some(libc::SIGKILL));
        assert_eq!(parse_signal("Hup"), Some(libc::SIGHUP));
        assert_eq!(parse_signal("USR1"), Some(30));
        assert_eq!(parse_signal("IOT"), Some(libc::SIGABRT));
        assert_eq!(parse_signal("EXIT"), Some(0));
    }

    #[test]
    fn numbers_with_gnu_masking() {
        assert_eq!(parse_signal("15"), Some(15));
        assert_eq!(parse_signal("0"), Some(0));
        assert_eq!(parse_signal("143"), Some(15));
        assert_eq!(parse_signal("271"), Some(15));
        assert_eq!(parse_signal("64"), Some(64));
        assert_eq!(parse_signal("65"), None);
        assert_eq!(parse_signal("99999999999"), None);
    }

    #[test]
    fn rejects_garbage() {
        for bad in ["", "FOO", "+5", "-1", "1 ", " 1", "RTMIN", "SIG", "1.5"] {
            assert_eq!(parse_signal(bad), None, "{bad:?} should be invalid");
        }
    }

    #[test]
    fn names_for_verbose_output() {
        assert_eq!(signal_name(libc::SIGTERM), Some("TERM"));
        assert_eq!(signal_name(libc::SIGKILL), Some("KILL"));
        assert_eq!(signal_name(libc::SIGABRT), Some("ABRT"));
        assert_eq!(signal_name(30), Some("USR1"));
        assert_eq!(signal_name(0), None);
        assert_eq!(signal_name(99), None);
    }
}
