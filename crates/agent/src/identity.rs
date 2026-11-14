//! Privilege separation (report section 4.6): run each job as the user who
//! submitted it, like slurmd, instead of as the agent.
//!
//! A root agent resolves the job's `user` to a local account in the parent
//! (NSS lookups are not async-signal-safe) and the child drops to it between
//! `fork` and `exec` with three plain syscalls: `setgroups`, `setgid`,
//! `setuid`, in that order (after `setuid` the others would be refused).
//! Unknown users and uid 0 are refused: a job never runs as root. An
//! unprivileged agent cannot switch users, so its jobs run as the agent.

use std::ffi::CString;
use std::io;

/// A resolved local account a job runs as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobIdentity {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    /// Supplementary groups, including `gid` (as `initgroups` would set).
    pub groups: Vec<u32>,
}

/// Whether this process can switch users (effective uid 0).
pub fn is_privileged() -> bool {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

/// Resolve `user` to a non-root local account with its groups.
pub fn resolve(user: &str) -> Result<JobIdentity, String> {
    let id = lookup(user)?;
    if id.uid == 0 {
        return Err(format!("refusing to run a job as root (user '{user}')"));
    }
    Ok(id)
}

/// `getpwnam_r` + `getgrouplist`, without the root check.
pub fn lookup(user: &str) -> Result<JobIdentity, String> {
    if user.is_empty() {
        return Err("job has no user".into());
    }
    let cname = CString::new(user).map_err(|_| format!("invalid user name '{user}'"))?;
    let (uid, gid) = passwd_entry(&cname, user)?;
    let groups = group_list(&cname, gid).map_err(|e| format!("groups of '{user}': {e}"))?;
    Ok(JobIdentity {
        name: user.to_string(),
        uid,
        gid,
        groups,
    })
}

fn passwd_entry(cname: &CString, user: &str) -> Result<(u32, u32), String> {
    let mut buf = vec![0u8; 1024];
    loop {
        // SAFETY: `pwd` and `buf` outlive the call; getpwnam_r writes only
        // into them and sets `result` to null or to `&pwd`.
        let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        let rc = unsafe {
            libc::getpwnam_r(
                cname.as_ptr(),
                &mut pwd,
                buf.as_mut_ptr().cast(),
                buf.len(),
                &mut result,
            )
        };
        if rc == libc::ERANGE && buf.len() < (1 << 20) {
            buf.resize(buf.len() * 2, 0);
            continue;
        }
        if rc != 0 {
            return Err(format!(
                "looking up user '{user}': {}",
                io::Error::from_raw_os_error(rc)
            ));
        }
        if result.is_null() {
            return Err(format!("no local account named '{user}'"));
        }
        return Ok((pwd.pw_uid, pwd.pw_gid));
    }
}

fn group_list(cname: &CString, gid: u32) -> io::Result<Vec<u32>> {
    let mut n: libc::c_int = 16;
    loop {
        let mut groups: Vec<libc::gid_t> = vec![0; n as usize];
        let mut count = n;
        // SAFETY: `groups` has room for `count` entries; on -1 `count` holds
        // the number needed and nothing past the buffer is written.
        let rc =
            unsafe { libc::getgrouplist(cname.as_ptr(), gid, groups.as_mut_ptr(), &mut count) };
        if rc >= 0 {
            groups.truncate(count.max(0) as usize);
            return Ok(groups);
        }
        if count <= n || n > 65_536 {
            return Err(io::Error::other("getgrouplist failed"));
        }
        n = count;
    }
}

/// Drop to `id` in the child, between fork and exec: `setgroups`, `setgid`,
/// `setuid`. Only raw syscalls on data prepared by the parent, so it is
/// async-signal-safe. Fails the spawn if any step fails.
pub fn drop_privileges(id: &JobIdentity) -> io::Result<()> {
    // SAFETY: plain syscalls on owned, fully initialised values.
    unsafe {
        if libc::setgroups(id.groups.len(), id.groups.as_ptr()) != 0 {
            return Err(io::Error::last_os_error());
        }
        if libc::setgid(id.gid) != 0 {
            return Err(io::Error::last_os_error());
        }
        if libc::setuid(id.uid) != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_resolves_but_is_refused_as_a_job_user() {
        let root = lookup("root").unwrap();
        assert_eq!((root.uid, root.gid), (0, 0));
        let err = resolve("root").unwrap_err();
        assert!(err.contains("refusing"), "{err}");
    }

    #[test]
    fn unknown_and_empty_users_are_refused() {
        assert!(resolve("gnomon-no-such-user-x9q")
            .unwrap_err()
            .contains("no local account"));
        assert!(resolve("").is_err());
        assert!(resolve("bad\0name").is_err());
    }

    #[test]
    fn an_ordinary_account_resolves_with_its_primary_group() {
        // `nobody` exists on every Linux distribution the agent targets.
        let n = resolve("nobody").unwrap();
        assert_ne!(n.uid, 0);
        assert!(n.groups.contains(&n.gid), "{n:?}");
    }
}
