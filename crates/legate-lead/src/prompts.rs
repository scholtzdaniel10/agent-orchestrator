//! Prompt text embedded so a rebuild cannot drift from the files it was copied from.
//!
//! [`LEAD_INSTRUCTIONS`] is the string `lead.ts` joins with blank lines. [`LEAD_DELEGATE_SKILL`]
//! is `legate-delegate/SKILL.md` from commit `6f5ca70` (plans, Run, and Grok). That commit is not
//! in this branch, so `resources/plugins` still has the older skill; the bytes here are the
//! updated file.

/// The system text sent on a lead's first turn. Byte-identical to the TypeScript constant.
pub const LEAD_INSTRUCTIONS: &str = include_str!("../prompts/lead_instructions.txt");

/// The lead skill from commit 6f5ca70. Byte-identical to that blob.
pub const LEAD_DELEGATE_SKILL: &str = include_str!("../prompts/legate-delegate.SKILL.md");

#[cfg(test)]
mod tests {
    use super::{LEAD_DELEGATE_SKILL, LEAD_INSTRUCTIONS};

    #[test]
    fn instructions_match_the_lead_rules() {
        assert!(LEAD_INSTRUCTIONS.contains("propose_jobs"));
        assert!(LEAD_INSTRUCTIONS.contains("Nothing runs until the person presses Run"));
        assert!(!LEAD_INSTRUCTIONS.contains("send_job"));
        assert!(!LEAD_INSTRUCTIONS.contains("read-only for now"));
        assert_eq!(LEAD_INSTRUCTIONS.matches("\n\n").count(), 3);
        assert!(!LEAD_INSTRUCTIONS.ends_with('\n'));
    }

    #[test]
    fn delegate_skill_is_the_plans_and_run_wording() {
        assert!(LEAD_DELEGATE_SKILL.starts_with("---\nname: legate-delegate\n"));
        assert!(LEAD_DELEGATE_SKILL.contains("presses **Run**"));
        assert!(LEAD_DELEGATE_SKILL.contains("propose_jobs"));
        assert!(LEAD_DELEGATE_SKILL.contains("Grok, then Cursor"));
        assert!(LEAD_DELEGATE_SKILL.ends_with('\n'));
    }
}
