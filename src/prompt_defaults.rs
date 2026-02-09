#![allow(dead_code)]
// This module is introduced by tr-ruf.1 to embed wizard prompt defaults; it is
// intentionally a small, stable API that follow-on tasks will start consuming.

use std::path::{Path, PathBuf};

pub(crate) const TRUDGE_PROMPT_REL: &str = ".codex/prompts/trudge.md";
pub(crate) const TRUDGE_REVIEW_PROMPT_REL: &str = ".codex/prompts/trudge_review.md";

const TRUDGE_PROMPT_DEFAULT: &str = include_str!("../prompts/trudge.md");
const TRUDGE_REVIEW_PROMPT_DEFAULT: &str = include_str!("../prompts/trudge_review.md");

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DefaultPrompt {
    pub(crate) path: PathBuf,
    pub(crate) contents: &'static str,
}

pub(crate) fn default_trudge_prompt_contents() -> &'static str {
    TRUDGE_PROMPT_DEFAULT
}

pub(crate) fn default_trudge_review_prompt_contents() -> &'static str {
    TRUDGE_REVIEW_PROMPT_DEFAULT
}

pub(crate) fn default_prompt_paths(home_dir: &Path) -> (PathBuf, PathBuf) {
    (
        home_dir.join(TRUDGE_PROMPT_REL),
        home_dir.join(TRUDGE_REVIEW_PROMPT_REL),
    )
}

pub(crate) fn default_prompts(home_dir: &Path) -> [DefaultPrompt; 2] {
    let (trudge_path, review_path) = default_prompt_paths(home_dir);
    [
        DefaultPrompt {
            path: trudge_path,
            contents: default_trudge_prompt_contents(),
        },
        DefaultPrompt {
            path: review_path,
            contents: default_trudge_review_prompt_contents(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_prompt_defaults_are_non_empty() {
        assert!(
            !default_trudge_prompt_contents().trim().is_empty(),
            "trudge prompt default should not be empty"
        );
        assert!(
            !default_trudge_review_prompt_contents().trim().is_empty(),
            "trudge_review prompt default should not be empty"
        );
    }

    #[test]
    fn default_prompts_return_expected_destinations() {
        let home = Path::new("/home/example");
        let prompts = default_prompts(home);
        assert_eq!(prompts[0].path, home.join(TRUDGE_PROMPT_REL));
        assert_eq!(prompts[1].path, home.join(TRUDGE_REVIEW_PROMPT_REL));
    }
}
