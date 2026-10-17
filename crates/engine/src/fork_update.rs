//! FORK-ONLY (UPD, FORK-PLAN §1a.77): the fork's builds look for updates among the public
//! `revolution-<version>` releases (fork rebrand, §1a.103) of the fork's release repository. Behind the engine feature
//! `fork-update`; listed in `docs/fork/FORK-ONLY.md`.

use crate::update::Channel;

pub const CHANNEL: Channel = Channel { repo: "dawgon2/revolution", tag_prefix: "revolution-" };
