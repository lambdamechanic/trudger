use std::fs;

use std::path::Path;

const PREDEFINED_ZAI_COMMAND: &str = "pi_trudge --prompt-env TRUDGER_AGENT_PROMPT";
const MACHINE_LOCAL_PI_HELPER_PATH: &str = ".local/bin/pi_trudge";

#[test]
fn checked_in_config_artifacts_reference_packaged_zai_command() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let artifact_paths = [
        root.join("sample_configuration")
            .join("trudgeable-with-hooks.yml"),
        root.join("sample_configuration").join("robot-triage.yml"),
        root.join("config_templates")
            .join("agents")
            .join("claude.yml"),
        root.join("config_templates")
            .join("agents")
            .join("codex.yml"),
        root.join("config_templates").join("agents").join("pi.yml"),
    ];

    for path in artifact_paths {
        let contents = fs::read_to_string(&path).expect("read artifact");
        assert!(
            contents.contains(PREDEFINED_ZAI_COMMAND),
            "artifact {} missing packaged z.ai command",
            path.display()
        );
        assert!(
            !contents.contains(MACHINE_LOCAL_PI_HELPER_PATH),
            "artifact {} contains machine-local helper path",
            path.display()
        );
    }
}

#[test]
fn readme_documents_legacy_trudge_migration() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let readme = fs::read_to_string(root.join("README.md")).expect("read README");

    assert!(
        readme.contains("~/.config/trudger.yml"),
        "migration docs should mention ~/.config/trudger.yml"
    );
    assert!(
        readme.contains("~/.config/trudge.yml"),
        "migration docs should mention legacy ~/.config/trudge.yml"
    );
    assert!(
        readme.contains("z.ai") && readme.contains("pi_trudge --prompt-env TRUDGER_AGENT_PROMPT"),
        "migration docs should mention z.ai packaged invocation"
    );
}
