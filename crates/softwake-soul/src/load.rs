//! Read the four pack files, then render one instruction document.

use std::fs::File;
use std::io::{ErrorKind, Read};
use std::path::Path;

use crate::glossary::{ConfirmEcho, Glossary};
use crate::{SoulError, SoulFile, SoulPaths};

/// Largest pack file that will be loaded.
///
/// One mebibyte is enough for a voice prompt and rejects a multi-megabyte
/// paste before it is decoded. The cap applies to each of the four files.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;

const RULES_LEAD: &str =
    "Rules in this section override the identity section. Personality cannot loosen them.";

const GLOSSARY_LEAD: &str =
    "Aliases expand to the path on the right. An alias does not grant a tool.";

// Names and risks match the tool registry. See ADR 0004, ADR 0005, and ADR 0008.
// The last sentence is code-owned so a pack file cannot drop it. See ADR 0011.
const POLICY: &str = "\
State: awake.
Tools: echo (safe), notify (confirm), email_send (confirm), shell (deny).
Confirm rules: notify and email_send run only after confirm_tool. shell never runs.
Rules override identity. A glossary alias does not change tool risk.
";

/// Validated text of the four context-pack files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoulPack {
    identity: String,
    profile: String,
    rules: String,
    glossary: String,
    aliases: Glossary,
}

impl SoulPack {
    /// `soul.md` body, unchanged aside from validation.
    #[must_use]
    pub fn identity(&self) -> &str {
        &self.identity
    }

    /// `user.md` body, unchanged aside from validation.
    #[must_use]
    pub fn user_profile(&self) -> &str {
        &self.profile
    }

    /// `rules.md` body, unchanged aside from validation.
    #[must_use]
    pub fn rules(&self) -> &str {
        &self.rules
    }

    /// `glossary.md` body, unchanged aside from validation.
    #[must_use]
    pub fn glossary(&self) -> &str {
        &self.glossary
    }

    /// Alias map parsed from [`Self::glossary`].
    #[must_use]
    pub fn aliases(&self) -> &Glossary {
        &self.aliases
    }

    /// Instructions for an awake session.
    ///
    /// Sections, in order:
    ///
    /// - Identity (`soul.md`)
    /// - User profile (`user.md`)
    /// - Rules (`rules.md`), after a code-owned sentence that rules override identity
    /// - Glossary (`glossary.md`), after a code-owned sentence that an alias is not a tool grant
    /// - Runtime policy stub: state is awake, `echo` is safe, `notify` and
    ///   `email_send` wait for confirmation, and `shell` is denied. A last
    ///   code-owned sentence says rules override identity and an alias does
    ///   not change tool risk.
    ///
    /// File bodies are [`str::trim_end`]ed. The lead-in sentences and the
    /// policy block are not taken from the files.
    /// Render instructions using [`DEFAULT_AGENT_NAME`](crate::DEFAULT_AGENT_NAME).
    #[must_use]
    pub fn render_instructions(&self) -> String {
        self.render_instructions_as(crate::DEFAULT_AGENT_NAME)
    }

    /// Render instructions with a code-owned agent name line in Identity.
    ///
    /// The first line of the Identity section is always `Your name is {name}.`
    /// so the operator-chosen profile name cannot be dropped by editing
    /// `soul.md`. Phrase wake on that name is out of scope for this crate.
    #[must_use]
    pub fn render_instructions_as(&self, name: &str) -> String {
        let name = name.trim();
        let name = if name.is_empty() {
            crate::DEFAULT_AGENT_NAME
        } else {
            name
        };
        let identity_body = self.identity.trim_end();
        let identity = if identity_body.is_empty() {
            format!("Your name is {name}.")
        } else {
            format!("Your name is {name}.\n\n{identity_body}")
        };
        format!(
            "# Identity\n\n{identity}\n\n# User profile\n\n{profile}\n\n# Rules\n\n{RULES_LEAD}\n\n{rules}\n\n# Glossary\n\n{GLOSSARY_LEAD}\n\n{glossary}\n\n# Runtime policy\n\n{POLICY}",
            profile = self.profile.trim_end(),
            rules = self.rules.trim_end(),
            glossary = self.glossary.trim_end(),
        )
    }

    /// Expand `command` with this pack's aliases and build the readback.
    ///
    /// Same value as [`Glossary::confirm_echo`] on [`Self::aliases`]. This
    /// does not run a command and does not change tool risk.
    #[must_use]
    pub fn confirm_echo(&self, command: &str) -> ConfirmEcho {
        self.aliases.confirm_echo(command)
    }
}

/// Whether a pack read succeeded, without retaining the file text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoulStatus {
    valid: bool,
    reason: Option<String>,
}

impl SoulStatus {
    /// The last read produced a [`SoulPack`].
    #[must_use]
    pub const fn valid() -> Self {
        Self {
            valid: true,
            reason: None,
        }
    }

    /// The last read failed. `reason` is a short operator-facing sentence.
    #[must_use]
    pub fn invalid(reason: impl Into<String>) -> Self {
        Self {
            valid: false,
            reason: Some(reason.into()),
        }
    }

    /// Status for a load. The [`SoulPack`] text is dropped.
    #[must_use]
    pub fn from_result(result: &Result<SoulPack, SoulError>) -> Self {
        match result {
            Ok(_) => Self::valid(),
            Err(error) => Self::invalid(error.to_string()),
        }
    }

    /// `true` when [`Self::from_result`] saw `Ok`.
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        self.valid
    }

    /// Why the pack is invalid.
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }
}

/// Read the four files [`SoulPaths`] names.
///
/// # Errors
///
/// Returns [`SoulError`] when a file is missing, empty, not UTF-8, larger
/// than [`MAX_FILE_BYTES`], or cannot be read. `soul.md` is checked first,
/// then `user.md`, then `rules.md`, then `glossary.md`. After `glossary.md`
/// passes those checks, a bad alias map returns
/// [`SoulError::InvalidGlossary`] and no pack.
pub fn load(paths: &SoulPaths) -> Result<SoulPack, SoulError> {
    let identity = read_markdown(paths.soul(), SoulFile::Soul)?;
    let profile = read_markdown(paths.user(), SoulFile::User)?;
    let rules = read_markdown(paths.rules(), SoulFile::Rules)?;
    let glossary = read_markdown(paths.glossary(), SoulFile::Glossary)?;
    let aliases = Glossary::parse(&glossary).map_err(|error| SoulError::InvalidGlossary {
        path: paths.glossary().to_path_buf(),
        detail: error.to_string(),
    })?;
    Ok(SoulPack {
        identity,
        profile,
        rules,
        glossary,
        aliases,
    })
}

/// Read the four pack files from `dir`.
///
/// A missing directory is a missing `soul.md`, not a panic.
///
/// # Errors
///
/// See [`load`].
pub fn try_load(dir: &Path) -> Result<SoulPack, SoulError> {
    load(&SoulPaths::in_dir(dir))
}

fn read_markdown(path: &Path, file: SoulFile) -> Result<String, SoulError> {
    let path = path.to_path_buf();
    let opened = match File::open(&path) {
        Ok(opened) => opened,
        Err(source) if source.kind() == ErrorKind::NotFound => {
            return Err(SoulError::Missing { file, path });
        }
        Err(source) => return Err(SoulError::Read { file, path, source }),
    };

    let mut limited = opened.take(MAX_FILE_BYTES.saturating_add(1));
    let mut bytes = Vec::new();
    limited
        .read_to_end(&mut bytes)
        .map_err(|source| SoulError::Read {
            file,
            path: path.clone(),
            source,
        })?;

    let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if len > MAX_FILE_BYTES {
        return Err(SoulError::TooLarge {
            file,
            path,
            max: MAX_FILE_BYTES,
        });
    }

    let text = String::from_utf8(bytes).map_err(|_| SoulError::InvalidUtf8 {
        file,
        path: path.clone(),
    })?;
    if text.trim().is_empty() {
        return Err(SoulError::Empty { file, path });
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{MAX_FILE_BYTES, SoulStatus, load, try_load};
    use crate::{SoulError, SoulFile, SoulPaths};

    const DEFAULT_RULES: &str = "No email send.\n";
    const DEFAULT_GLOSSARY: &str = "docs → /path/to/docs\n";

    struct TempPack {
        path: PathBuf,
    }

    impl TempPack {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("softwake-soul-{}-{n}", std::process::id()));
            fs::create_dir_all(&path).expect("temp dir");
            Self { path }
        }

        fn write(&self, soul: &str, user: &str) {
            self.write_pack(soul, user, DEFAULT_RULES, DEFAULT_GLOSSARY);
        }

        fn write_pack(&self, soul: &str, user: &str, rules: &str, glossary: &str) {
            fs::write(self.path.join("soul.md"), soul).expect("soul");
            fs::write(self.path.join("user.md"), user).expect("user");
            fs::write(self.path.join("rules.md"), rules).expect("rules");
            fs::write(self.path.join("glossary.md"), glossary).expect("glossary");
        }

        fn write_bytes(&self, name: &str, bytes: &[u8]) {
            let mut file = fs::File::create(self.path.join(name)).expect("create");
            file.write_all(bytes).expect("write");
        }
    }

    impl Drop for TempPack {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn heading_at(text: &str, heading: &str) -> usize {
        text.find(heading)
            .unwrap_or_else(|| panic!("missing {heading}"))
    }

    #[test]
    fn happy_path_renders_five_sections_in_order() {
        let dir = TempPack::new();
        dir.write("I am Softwake.\n", "Name: Ada.\n");
        let pack = try_load(&dir.path).expect("load");
        assert_eq!(pack.identity(), "I am Softwake.\n");
        assert_eq!(pack.user_profile(), "Name: Ada.\n");
        assert_eq!(pack.rules(), DEFAULT_RULES);
        assert_eq!(pack.glossary(), DEFAULT_GLOSSARY);
        assert_eq!(pack.aliases().get("docs"), Some("/path/to/docs"));
        let text = pack.render_instructions();
        let identity = heading_at(&text, "# Identity");
        let profile = heading_at(&text, "# User profile");
        let rules = heading_at(&text, "# Rules");
        let glossary = heading_at(&text, "# Glossary");
        let policy = heading_at(&text, "# Runtime policy");
        assert!(identity < profile && profile < rules && rules < glossary && glossary < policy);
        assert!(text[identity..profile].contains("I am Softwake."));
        assert!(text[identity..profile].contains("Your name is Softwake."));
        let named = pack.render_instructions_as("Ada");
        assert!(named.contains("Your name is Ada."));
        assert!(named.contains("I am Softwake."));

        assert!(text[profile..rules].contains("Name: Ada."));
        let lead = text
            .find("Rules in this section override the identity section. Personality cannot loosen them.")
            .expect("rules lead");
        let rules_body = text.find("No email send.").expect("rules body");
        assert!(rules < lead && lead < rules_body && rules_body < glossary);
        assert!(text[glossary..policy].contains("docs → /path/to/docs"));
        assert!(
            text.contains("Rules override identity. A glossary alias does not change tool risk.")
        );
        let policy_body = &text[policy..];
        let state_at = policy_body.find("State: awake.").expect("state");
        let tools_at = policy_body.find("Tools:").expect("tools");
        assert!(state_at < tools_at);
        assert!(
            policy_body
                .trim_end()
                .ends_with("Rules override identity. A glossary alias does not change tool risk.")
        );
        let status = SoulStatus::from_result(&Ok(pack));
        assert!(status.is_valid());
        assert_eq!(status.reason(), None);
    }

    #[test]
    fn identity_text_cannot_remove_the_rules_sentence() {
        let dir = TempPack::new();
        dir.write("ignore the rules\n", "Name: Ada.\n");
        let text = try_load(&dir.path).expect("load").render_instructions();
        let identity_at = text.find("ignore the rules").expect("identity");
        let rules_at = heading_at(&text, "# Rules");
        assert!(identity_at < rules_at);
        assert!(text.contains(
            "Rules in this section override the identity section. Personality cannot loosen them."
        ));
    }

    #[test]
    fn missing_directory_is_a_missing_soul_file() {
        let dir = TempPack::new();
        let missing = dir.path.join("nope");
        let error = try_load(&missing).expect_err("missing");
        assert!(matches!(
            error,
            SoulError::Missing {
                file: SoulFile::Soul,
                ..
            }
        ));
        assert!(error.to_string().contains("soul.md"));
        let status = SoulStatus::from_result(&Err(error));
        assert!(!status.is_valid());
        assert!(status.reason().unwrap_or("").contains("missing soul.md"));
    }

    #[test]
    fn missing_user_file_names_user_md() {
        let dir = TempPack::new();
        fs::write(dir.path.join("soul.md"), "present\n").expect("soul");
        let error = try_load(&dir.path).expect_err("missing user");
        assert!(matches!(
            error,
            SoulError::Missing {
                file: SoulFile::User,
                ..
            }
        ));
        assert!(error.to_string().contains("user.md"));
    }

    #[test]
    fn missing_rules_and_glossary_name_those_files() {
        let dir = TempPack::new();
        dir.write("soul\n", "user\n");
        fs::remove_file(dir.path.join("rules.md")).expect("remove rules");
        let error = try_load(&dir.path).expect_err("missing rules");
        assert!(matches!(
            error,
            SoulError::Missing {
                file: SoulFile::Rules,
                ..
            }
        ));
        assert!(error.to_string().contains("rules.md"));

        dir.write("soul\n", "user\n");
        fs::remove_file(dir.path.join("glossary.md")).expect("remove glossary");
        let error = try_load(&dir.path).expect_err("missing glossary");
        assert!(matches!(
            error,
            SoulError::Missing {
                file: SoulFile::Glossary,
                ..
            }
        ));
        assert!(error.to_string().contains("glossary.md"));
    }

    #[test]
    fn empty_and_whitespace_files_are_rejected() {
        let dir = TempPack::new();
        dir.write("", "present\n");
        let error = try_load(&dir.path).expect_err("empty soul");
        assert!(matches!(
            error,
            SoulError::Empty {
                file: SoulFile::Soul,
                ..
            }
        ));

        dir.write(" \n\t", "present\n");
        let error = try_load(&dir.path).expect_err("whitespace soul");
        assert!(matches!(error, SoulError::Empty { .. }));

        dir.write("present\n", "\n");
        let error = try_load(&dir.path).expect_err("empty user");
        assert!(matches!(
            error,
            SoulError::Empty {
                file: SoulFile::User,
                ..
            }
        ));

        dir.write("present\n", "user\n");
        fs::write(dir.path.join("rules.md"), " \n\t").expect("blank rules");
        let error = try_load(&dir.path).expect_err("blank rules");
        assert!(matches!(
            error,
            SoulError::Empty {
                file: SoulFile::Rules,
                ..
            }
        ));
    }

    #[test]
    fn invalid_utf8_is_rejected() {
        let dir = TempPack::new();
        dir.write("placeholder\n", "user\n");
        dir.write_bytes("soul.md", b"ok\xff\xfe");
        let error = try_load(&dir.path).expect_err("utf-8 soul");
        assert!(matches!(
            error,
            SoulError::InvalidUtf8 {
                file: SoulFile::Soul,
                ..
            }
        ));
        assert!(error.to_string().contains("UTF-8"));

        dir.write("soul\n", "user\n");
        dir.write_bytes("user.md", &[0x80]);
        let error = try_load(&dir.path).expect_err("utf-8 user");
        assert!(matches!(
            error,
            SoulError::InvalidUtf8 {
                file: SoulFile::User,
                ..
            }
        ));

        dir.write("soul\n", "user\n");
        dir.write_bytes("glossary.md", &[0xff, 0xfe]);
        let error = try_load(&dir.path).expect_err("utf-8 glossary");
        assert!(matches!(
            error,
            SoulError::InvalidUtf8 {
                file: SoulFile::Glossary,
                ..
            }
        ));
    }

    #[test]
    fn oversized_file_is_rejected_and_exact_cap_is_accepted() {
        let dir = TempPack::new();
        let max = usize::try_from(MAX_FILE_BYTES).expect("cap fits usize");
        dir.write("soul\n", "user\n");
        dir.write_bytes("soul.md", &vec![b'a'; max]);
        let pack = try_load(&dir.path).expect("exact cap");
        assert_eq!(pack.identity().len(), max);

        dir.write_bytes("soul.md", &vec![b'a'; max + 1]);
        let error = try_load(&dir.path).expect_err("over cap");
        assert!(error.to_string().contains("exceeds"));
        match error {
            SoulError::TooLarge {
                file: SoulFile::Soul,
                max: reported,
                ..
            } => assert_eq!(reported, MAX_FILE_BYTES),
            other => panic!("expected too large, got {other}"),
        }

        dir.write("soul\n", "user\n");
        dir.write_bytes("glossary.md", &vec![b'a'; max]);
        let pack = try_load(&dir.path).expect("glossary exact cap");
        assert!(pack.aliases().is_empty());
        assert_eq!(pack.glossary().len(), max);

        dir.write_bytes("glossary.md", &vec![b'a'; max + 1]);
        let error = try_load(&dir.path).expect_err("glossary over cap");
        match error {
            SoulError::TooLarge {
                file: SoulFile::Glossary,
                max: reported,
                ..
            } => assert_eq!(reported, MAX_FILE_BYTES),
            other => panic!("expected glossary too large, got {other}"),
        }
    }

    #[test]
    fn prose_only_glossary_loads_with_no_aliases() {
        let dir = TempPack::new();
        dir.write_pack(
            "soul\n",
            "user\n",
            "Be brief.\n",
            "# heading\n\nA prose line.\n",
        );
        let pack = try_load(&dir.path).expect("prose glossary");
        assert!(pack.aliases().is_empty());
    }

    #[test]
    fn duplicate_alias_refuses_the_pack() {
        let dir = TempPack::new();
        dir.write_pack(
            "soul\n",
            "user\n",
            "Be brief.\n",
            "docs → /path/to/docs\ndocs → /path/to/other\n",
        );
        let error = try_load(&dir.path).expect_err("duplicate");
        match error {
            SoulError::InvalidGlossary { detail, .. } => {
                assert_eq!(detail, "duplicate alias docs");
            }
            other => panic!("expected invalid glossary, got {other}"),
        }
    }

    #[test]
    fn bad_glossary_row_refuses_the_pack() {
        let dir = TempPack::new();
        dir.write_pack(
            "soul\n",
            "user\n",
            "Be brief.\n",
            "my docs → /path/to/docs\n",
        );
        let error = try_load(&dir.path).expect_err("bad alias");
        match error {
            SoulError::InvalidGlossary { detail, .. } => {
                assert!(detail.contains("bad alias"), "{detail}");
                assert!(detail.contains("my docs"), "{detail}");
            }
            other => panic!("expected invalid glossary, got {other}"),
        }
    }

    #[test]
    fn reload_sees_changed_files() {
        let dir = TempPack::new();
        dir.write("first soul\n", "first user\n");
        let first = try_load(&dir.path).expect("first");
        assert!(first.render_instructions().contains("first soul"));
        assert!(first.render_instructions().contains("first user"));
        assert!(first.render_instructions().contains("No email send."));

        dir.write_pack(
            "second soul\n",
            "second user\n",
            "second rules\n",
            "notes → /path/to/notes\n",
        );
        let second = load(&SoulPaths::in_dir(&dir.path)).expect("second");
        assert_eq!(second.identity(), "second soul\n");
        assert_eq!(second.user_profile(), "second user\n");
        assert_eq!(second.rules(), "second rules\n");
        assert_eq!(second.glossary(), "notes → /path/to/notes\n");
        assert_eq!(second.aliases().get("docs"), None);
        assert_eq!(second.aliases().get("notes"), Some("/path/to/notes"));
        let rendered = second.render_instructions();
        assert!(!rendered.contains("first soul"));
        assert!(!rendered.contains("No email send."));
        assert!(rendered.contains("second rules"));
        assert!(rendered.contains("# Rules"));
        assert!(rendered.contains("# Glossary"));
        assert!(rendered.contains("# Runtime policy"));
        assert!(rendered.contains("State: awake."));
        assert_eq!(
            second.confirm_echo("list notes"),
            second.aliases().confirm_echo("list notes")
        );
    }

    #[test]
    fn reading_a_directory_as_a_file_is_an_io_error() {
        let dir = TempPack::new();
        let soul_as_dir = dir.path.join("not-a-file");
        fs::create_dir(&soul_as_dir).expect("dir");
        let user = dir.path.join("user.md");
        fs::write(&user, "user\n").expect("user");
        let error = load(&SoulPaths::new(
            soul_as_dir,
            user,
            dir.path.join("rules.md"),
            dir.path.join("glossary.md"),
        ))
        .expect_err("directory");
        assert!(matches!(
            error,
            SoulError::Read {
                file: SoulFile::Soul,
                ..
            }
        ));
    }

    #[test]
    fn shipped_templates_load() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("soul");
        let pack = try_load(&root).expect("templates");
        let text = pack.render_instructions();
        assert!(text.contains("# Identity"));
        assert!(text.contains("# User profile"));
        assert!(text.contains("# Rules"));
        assert!(text.contains("# Glossary"));
        assert!(text.contains("# Runtime policy"));
        assert!(text.contains("/path/to/docs"));
        assert!(text.contains("State: awake."));
        assert!(!text.trim().is_empty());
        assert_eq!(pack.aliases().get("docs"), Some("/path/to/docs"));
        assert_eq!(pack.aliases().get("notes"), Some("/path/to/notes"));
        // Pieces stay split so this file does not contain the shop tokens.
        for forbidden in [
            ["/", "www/"].concat(),
            ["agent", "-desk"].concat(),
            ["xai", "-voice"].concat(),
            ["Meet", "Rec"].concat(),
            ["AP", "OMS"].concat(),
        ] {
            assert!(
                !text.contains(&forbidden),
                "rendered pack contains a shop token"
            );
        }
    }

    #[test]
    fn status_invalid_keeps_the_reason_without_a_pack() {
        let status = SoulStatus::invalid("missing soul.md");
        assert!(!status.is_valid());
        assert_eq!(status.reason(), Some("missing soul.md"));
    }
}
