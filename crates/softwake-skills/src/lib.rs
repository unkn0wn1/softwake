//! On-disk Softwake skills ([ADR 0014](../../docs/ADR-0014-skills-hub.md)).
//!
//! Authored Markdown under `$XDG_DATA_HOME/softwake/skills/` (else
//! `~/.local/share/softwake/skills/`). Skills are not soul-pack files.

mod store;

pub use store::{
    Skill, SkillError, SkillSource, delete_skill, list_skills, load_skill, resolve_skills_dir,
    resolve_skills_dir_from, save_skill, slugify,
};

/// Maximum skill body section length Softwake will accept (bytes).
pub const MAX_SECTION_BYTES: usize = 64 * 1024;

/// Maximum skills listed in one catalog appendix.
pub const MAX_CATALOG_ENTRIES: usize = 32;
