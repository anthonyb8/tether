//! Starting programs that outlive the command that started them.

use std::os::unix::process::CommandExt;
use std::process::Command;

/// Puts the child in its own process group, so a Ctrl-C meant for the
/// terminal's tether does not also close the browser or background connect
/// it started.
pub fn detached(cmd: &mut Command) -> &mut Command {
    cmd.process_group(0)
}
