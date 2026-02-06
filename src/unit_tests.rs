use clap::Parser;
use std::env;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::{NamedTempFile, TempDir};

use crate::app::{render_prompt, require_file, run_with_cli};
use crate::cli::{parse_manual_tasks, Cli, CliCommand};
use crate::config::{load_config, Commands, Config, Hooks};
use crate::doctor::run_doctor_mode;
use crate::logger::{sanitize_log_value, Logger};
use crate::run_loop::{reset_task_on_exit, run_loop, validate_config, Quit, RuntimeState};
use crate::shell::render_args;
use crate::tmux::{build_tmux_name, TmuxState};

static ENV_MUTEX: Mutex<()> = Mutex::new(());
static ORIGINAL_PATH: OnceLock<Option<std::ffi::OsString>> = OnceLock::new();

fn reset_test_env() {
    let original_path = ORIGINAL_PATH.get_or_init(|| env::var_os("PATH"));
    match original_path {
        Some(value) => env::set_var("PATH", value),
        None => env::remove_var("PATH"),
    }
    for key in [
        "NEXT_TASK_EXIT_CODE",
        "NEXT_TASK_OUTPUT_QUEUE",
        "NEXT_TASK_OUTPUT",
        "TASK_STATUS_QUEUE",
        "TASK_STATUS_OUTPUT",
        "TASK_SHOW_QUEUE",
        "TASK_SHOW_OUTPUT",
        "TRUDGER_CONFIG_PATH",
        "TRUDGER_DOCTOR_SCRATCH_DIR",
        "TRUDGER_SKIP_NOT_READY_LIMIT",
        "TRUDGER_PROMPT",
        "TRUDGER_REVIEW_PROMPT",
        "TRUDGER_TASK_ID",
        "TRUDGER_TASK_SHOW",
        "TRUDGER_TASK_STATUS",
        "CODEX_MOCK_LOG",
        "TASK_SHOW_LOG",
        "TASK_STATUS_LOG",
        "TASK_UPDATE_LOG",
        "HOOK_MOCK_LOG",
        "RESET_TASK_LOG",
        "NEXT_TASK_LOG",
        "TRUDGER_TEST_FORCE_ERR",
    ] {
        env::remove_var(key);
    }
}

#[test]
fn sanitize_log_value_replaces_controls() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();
    let value = "line\ncarriage\rtab\t";
    assert_eq!(sanitize_log_value(value), "line\\ncarriage\\rtab\\t");
}

#[test]
fn log_line_matches_shell_fixture() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();
    let temp = TempDir::new().expect("temp dir");
    let log_path = temp.path().join("trudger.log");
    let logger = Logger::new(Some(log_path.clone()));

    let command = "next-task\t--with-tab";
    let args = vec![
        "tr-1".to_string(),
        "with space".to_string(),
        "tab\targ".to_string(),
        "#start".to_string(),
        "~home".to_string(),
        "foo$bar".to_string(),
        "foo\"bar".to_string(),
        "foo\\bar".to_string(),
    ];
    let args_render = render_args(&args);
    logger.log_transition(&format!(
        "cmd start label=next-task task=tr-1 mode=bash_lc command={} args={}",
        sanitize_log_value(command),
        sanitize_log_value(&args_render)
    ));

    let log_contents = fs::read_to_string(&log_path).expect("read log file");
    let line = log_contents.lines().next().expect("log line");
    let message = line.split_once(' ').map(|x| x.1).unwrap_or("");
    let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("logs")
        .join("command-start.txt");
    let expected = fs::read_to_string(&fixture_path).expect("read fixture");
    let expected = expected.trim_end_matches('\n').trim_end_matches('\r');
    assert_eq!(message, expected);
}

#[test]
fn render_prompt_strips_frontmatter() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();
    let mut file = NamedTempFile::new().expect("temp file");
    writeln!(file, "---\nname: test\n---\nHello\nWorld").expect("write");
    let rendered = render_prompt(file.path()).expect("render");
    assert_eq!(rendered, "Hello\nWorld");
}

#[test]
fn require_file_reports_missing() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();
    let temp = TempDir::new().expect("temp dir");
    let missing = temp.path().join("missing.txt");
    let err = require_file(&missing, "prompt file").expect_err("should fail");
    assert!(
        err.contains("Missing prompt file"),
        "error should mention missing prompt file, got: {err}"
    );
}

#[test]
fn build_tmux_name_trims_fg_and_codex_suffixes() {
    assert_eq!(build_tmux_name("host: fg", "", "tr-1", &[], &[]), "host");
    assert_eq!(build_tmux_name("host: codex", "", "tr-1", &[], &[]), "host");
    assert_eq!(
        build_tmux_name("host: other", "", "tr-1", &[], &[]),
        "host: other"
    );
}

#[test]
fn build_tmux_name_formats_task_lists_and_phase_suffixes() {
    let completed = vec!["tr-1".to_string(), "tr-2".to_string()];
    let needs_human = vec!["tr-3".to_string()];

    assert_eq!(
        build_tmux_name("base", "SOLVING", "tr-9", &completed, &needs_human),
        "base COMPLETED [tr-1, tr-2] NEEDS_HUMAN [tr-3] SOLVING tr-9"
    );
    assert_eq!(
        build_tmux_name("base", "REVIEWING", "tr-9", &[], &needs_human),
        "base NEEDS_HUMAN [tr-3] REVIEWING tr-9"
    );
    assert_eq!(
        build_tmux_name("base", "ERROR", "tr-9", &completed, &[]),
        "base COMPLETED [tr-1, tr-2] HALTED ON ERROR tr-9"
    );
}

#[test]
fn run_loop_executes_commands_and_hooks_with_env() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();
    let temp = TempDir::new().expect("temp dir");
    let log_path = temp.path().join("trudger.log");
    let next_task_log = temp.path().join("next-task.log");
    let task_show_log = temp.path().join("task-show.log");
    let task_status_log = temp.path().join("task-status.log");
    let task_update_log = temp.path().join("task-update.log");
    let hook_log = temp.path().join("hook.log");
    let codex_log = temp.path().join("codex.log");

    let next_task_queue = temp.path().join("next-task-queue.txt");
    fs::write(&next_task_queue, "tr-1\ntr-2\n\n").expect("write next task queue");
    let status_queue = temp.path().join("status-queue.txt");
    fs::write(&status_queue, "ready\nclosed\nready\nblocked\n").expect("write status queue");

    let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bin");
    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));
    env::set_var("NEXT_TASK_LOG", &next_task_log);
    env::set_var("TASK_SHOW_LOG", &task_show_log);
    env::set_var("TASK_STATUS_LOG", &task_status_log);
    env::set_var("TASK_UPDATE_LOG", &task_update_log);
    env::set_var("HOOK_MOCK_LOG", &hook_log);
    env::set_var("CODEX_MOCK_LOG", &codex_log);
    env::set_var("NEXT_TASK_OUTPUT_QUEUE", &next_task_queue);
    env::set_var("TASK_STATUS_QUEUE", &status_queue);
    env::set_var("TASK_SHOW_OUTPUT", "SHOW_PAYLOAD");

    let config = Config {
        agent_command: "codex --yolo exec --default".to_string(),
        agent_review_command: "codex --yolo exec --review \"$@\"".to_string(),
        commands: Commands {
            next_task: Some("next-task\t--with-tab".to_string()),
            task_show: "task-show \"$@\"".to_string(),
            task_status: "task-status".to_string(),
            task_update_in_progress: "task-update \"$@\"".to_string(),
            reset_task: "reset-task \"$@\"".to_string(),
        },
        hooks: Hooks {
            on_completed: "hook --done".to_string(),
            on_requires_human: "hook --human".to_string(),
            on_doctor_setup: None,
        },
        review_loop_limit: 2,
        log_path: log_path.display().to_string(),
    };

    let mut state = RuntimeState {
        config,
        config_path: temp.path().join("trudger.yml"),
        prompt_trudge: "Task context".to_string(),
        prompt_review: "Review context".to_string(),
        logger: Logger::new(Some(log_path.clone())),
        tmux: TmuxState::disabled(),
        interrupt_flag: Arc::new(AtomicBool::new(false)),
        manual_tasks: Vec::new(),
        completed_tasks: Vec::new(),
        needs_human_tasks: Vec::new(),
        current_task_id: None,
        current_task_show: None,
        current_task_status: None,
    };

    let result = run_loop(&mut state).expect_err("should exit after queue drained");
    assert_eq!(result.code, 0, "expected graceful exit");

    let codex_contents = fs::read_to_string(&codex_log).expect("read codex log");
    assert!(
        codex_contents.contains("envset TRUDGER_PROMPT=1"),
        "codex should see TRUDGER_PROMPT set"
    );
    assert!(
        codex_contents.contains("envset TRUDGER_REVIEW_PROMPT=1"),
        "codex should see TRUDGER_REVIEW_PROMPT set"
    );
    assert!(
        codex_contents.contains("env TRUDGER_TASK_SHOW=SHOW_PAYLOAD"),
        "codex should receive task show output"
    );
    assert!(
        codex_contents.contains("resume --last"),
        "codex review should include resume --last"
    );

    let next_task_contents = fs::read_to_string(&next_task_log).expect("read next task log");
    assert!(
        next_task_contents.contains("envset TRUDGER_PROMPT=0"),
        "next-task should not get TRUDGER_PROMPT"
    );
    assert!(
        next_task_contents.contains("envset TRUDGER_TASK_ID=0"),
        "next-task should not get TRUDGER_TASK_ID"
    );

    let task_show_contents = fs::read_to_string(&task_show_log).expect("read task-show log");
    assert!(
        task_show_contents.contains("task-show args_count=0 args="),
        "task-show should receive no args"
    );

    let hook_contents = fs::read_to_string(&hook_log).expect("read hook log");
    assert!(
        hook_contents.contains("hook args_count=1 args=--done"),
        "hook should not receive positional task args"
    );
    assert!(
        hook_contents.contains("hook args_count=1 args=--human"),
        "requires-human hook should not receive positional task args"
    );
    assert!(
        hook_contents.contains("env TRUDGER_TASK_ID=tr-1"),
        "hook should see task id in env"
    );

    let log_contents = fs::read_to_string(&log_path).expect("read log file");
    let escaped = log_contents.contains("\\t");
    let raw_tab = log_contents.contains('\t');
    let snippet = log_contents
        .lines()
        .find(|line| line.contains("command=next-task"))
        .unwrap_or("");
    assert!(
        escaped,
        "log should escape tab in command (escaped={escaped}, raw_tab={raw_tab}, bytes={:?}), got:\n{log_contents}",
        snippet.as_bytes()
    );
    assert!(
        !raw_tab,
        "log should not include raw tab characters, got:\n{log_contents}"
    );
}

#[test]
fn manual_task_not_ready_fails_fast_without_invoking_next_task() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let next_task_log = temp.path().join("next-task.log");
    let task_status_log = temp.path().join("task-status.log");
    let task_show_log = temp.path().join("task-show.log");
    let task_update_log = temp.path().join("task-update.log");
    let hook_log = temp.path().join("hook.log");
    let codex_log = temp.path().join("codex.log");

    let status_queue = temp.path().join("status-queue.txt");
    fs::write(&status_queue, "blocked\n").expect("write status queue");

    let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bin");
    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));

    env::set_var("NEXT_TASK_LOG", &next_task_log);
    env::set_var("TASK_STATUS_LOG", &task_status_log);
    env::set_var("TASK_SHOW_LOG", &task_show_log);
    env::set_var("TASK_UPDATE_LOG", &task_update_log);
    env::set_var("HOOK_MOCK_LOG", &hook_log);
    env::set_var("CODEX_MOCK_LOG", &codex_log);
    env::set_var("TASK_STATUS_QUEUE", &status_queue);

    let config = Config {
        agent_command: "codex --yolo exec --default".to_string(),
        agent_review_command: "codex --yolo exec --review \"$@\"".to_string(),
        commands: Commands {
            next_task: Some("next-task".to_string()),
            task_show: "task-show \"$@\"".to_string(),
            task_status: "task-status".to_string(),
            task_update_in_progress: "task-update \"$@\"".to_string(),
            reset_task: "reset-task \"$@\"".to_string(),
        },
        hooks: Hooks {
            on_completed: "hook --done".to_string(),
            on_requires_human: "hook --human".to_string(),
            on_doctor_setup: None,
        },
        review_loop_limit: 2,
        log_path: temp.path().join("trudger.log").display().to_string(),
    };

    let mut state = RuntimeState {
        config,
        config_path: temp.path().join("trudger.yml"),
        prompt_trudge: "Task context".to_string(),
        prompt_review: "Review context".to_string(),
        logger: Logger::new(None),
        tmux: TmuxState::disabled(),
        interrupt_flag: Arc::new(AtomicBool::new(false)),
        manual_tasks: vec!["tr-1".to_string()],
        completed_tasks: Vec::new(),
        needs_human_tasks: Vec::new(),
        current_task_id: None,
        current_task_show: None,
        current_task_status: None,
    };

    let result = run_loop(&mut state).expect_err("manual blocked task should fail fast");
    assert_eq!(result.code, 1);
    assert_eq!(result.reason, "task_not_ready:tr-1");

    assert!(!next_task_log.exists(), "next-task should not run");
    assert!(!task_show_log.exists(), "task-show should not run");
    assert!(!task_update_log.exists(), "task-update should not run");
    assert!(!hook_log.exists(), "hooks should not run");
    assert!(!codex_log.exists(), "agent should not run");
    assert!(
        task_status_log.exists(),
        "task-status should run for readiness check"
    );
}

#[test]
fn manual_task_runs_solve_review_and_hooks_without_invoking_next_task() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let log_path = temp.path().join("trudger.log");
    let next_task_log = temp.path().join("next-task.log");
    let task_show_log = temp.path().join("task-show.log");
    let task_status_log = temp.path().join("task-status.log");
    let task_update_log = temp.path().join("task-update.log");
    let hook_log = temp.path().join("hook.log");
    let codex_log = temp.path().join("codex.log");

    let status_queue = temp.path().join("status-queue.txt");
    fs::write(&status_queue, "ready\nclosed\n").expect("write status queue");

    let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bin");
    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));
    env::set_var("NEXT_TASK_EXIT_CODE", "1");
    env::set_var("NEXT_TASK_LOG", &next_task_log);
    env::set_var("TASK_SHOW_LOG", &task_show_log);
    env::set_var("TASK_STATUS_LOG", &task_status_log);
    env::set_var("TASK_UPDATE_LOG", &task_update_log);
    env::set_var("HOOK_MOCK_LOG", &hook_log);
    env::set_var("CODEX_MOCK_LOG", &codex_log);
    env::set_var("TASK_STATUS_QUEUE", &status_queue);
    env::set_var("TASK_SHOW_OUTPUT", "SHOW_PAYLOAD");

    let config = Config {
        agent_command: "codex --yolo exec --default".to_string(),
        agent_review_command: "codex --yolo exec --review \"$@\"".to_string(),
        commands: Commands {
            next_task: Some("next-task".to_string()),
            task_show: "task-show \"$@\"".to_string(),
            task_status: "task-status".to_string(),
            task_update_in_progress: "task-update \"$@\"".to_string(),
            reset_task: "reset-task \"$@\"".to_string(),
        },
        hooks: Hooks {
            on_completed: "hook --done".to_string(),
            on_requires_human: "hook --human".to_string(),
            on_doctor_setup: None,
        },
        review_loop_limit: 2,
        log_path: log_path.display().to_string(),
    };

    let interrupt_flag = Arc::new(AtomicBool::new(false));
    let interrupter_flag = Arc::clone(&interrupt_flag);
    let interrupter_hook_log = hook_log.clone();
    let interrupter = thread::spawn(move || {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            if interrupter_hook_log.exists() {
                interrupter_flag.store(true, std::sync::atomic::Ordering::SeqCst);
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
    });

    let mut state = RuntimeState {
        config,
        config_path: temp.path().join("trudger.yml"),
        prompt_trudge: "Task context".to_string(),
        prompt_review: "Review context".to_string(),
        logger: Logger::new(Some(log_path)),
        tmux: TmuxState::disabled(),
        interrupt_flag,
        manual_tasks: vec!["tr-1".to_string()],
        completed_tasks: Vec::new(),
        needs_human_tasks: Vec::new(),
        current_task_id: None,
        current_task_show: None,
        current_task_status: None,
    };

    let result = run_loop(&mut state).expect_err("interrupter should stop the loop");
    let _ = interrupter.join();
    assert_eq!(result.code, 130, "expected interrupt exit");
    assert_eq!(state.completed_tasks, vec!["tr-1"]);

    assert!(
        !next_task_log.exists(),
        "next-task should not run when manual tasks are provided"
    );

    let codex_contents = fs::read_to_string(&codex_log).expect("read codex log");
    assert!(
        codex_contents.contains("envset TRUDGER_PROMPT=1"),
        "agent solve should receive TRUDGER_PROMPT"
    );
    assert!(
        codex_contents.contains("envset TRUDGER_REVIEW_PROMPT=1"),
        "agent review should receive TRUDGER_REVIEW_PROMPT"
    );

    let hook_contents = fs::read_to_string(&hook_log).expect("read hook log");
    assert!(
        hook_contents.contains("hook args_count=1 args=--done"),
        "expected completed hook, got:\n{hook_contents}"
    );
    assert!(
        hook_contents.contains("env TRUDGER_TASK_ID=tr-1"),
        "hook should see task id in env, got:\n{hook_contents}"
    );
}

#[test]
fn review_loop_limit_retries_until_closed() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();
    let temp = TempDir::new().expect("temp dir");
    let log_path = temp.path().join("trudger.log");
    let task_update_log = temp.path().join("task-update.log");
    let hook_log = temp.path().join("hook.log");

    let next_task_queue = temp.path().join("next-task-queue.txt");
    fs::write(&next_task_queue, "tr-1\n\n").expect("write next task queue");
    let status_queue = temp.path().join("status-queue.txt");
    fs::write(&status_queue, "ready\nopen\nclosed\n").expect("write status queue");

    let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bin");
    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));
    env::set_var("NEXT_TASK_OUTPUT_QUEUE", &next_task_queue);
    env::set_var("TASK_STATUS_QUEUE", &status_queue);
    env::set_var("TASK_SHOW_OUTPUT", "SHOW_PAYLOAD");
    env::set_var("TASK_UPDATE_LOG", &task_update_log);
    env::set_var("HOOK_MOCK_LOG", &hook_log);

    let config = Config {
        agent_command: "codex --yolo exec --default".to_string(),
        agent_review_command: "codex --yolo exec --review \"$@\"".to_string(),
        commands: Commands {
            next_task: Some("next-task".to_string()),
            task_show: "task-show \"$@\"".to_string(),
            task_status: "task-status".to_string(),
            task_update_in_progress: "task-update \"$@\"".to_string(),
            reset_task: "reset-task \"$@\"".to_string(),
        },
        hooks: Hooks {
            on_completed: "hook --done".to_string(),
            on_requires_human: "hook --human".to_string(),
            on_doctor_setup: None,
        },
        review_loop_limit: 2,
        log_path: log_path.display().to_string(),
    };

    let mut state = RuntimeState {
        config,
        config_path: temp.path().join("trudger.yml"),
        prompt_trudge: "Task context".to_string(),
        prompt_review: "Review context".to_string(),
        logger: Logger::new(Some(log_path)),
        tmux: TmuxState::disabled(),
        interrupt_flag: Arc::new(AtomicBool::new(false)),
        manual_tasks: Vec::new(),
        completed_tasks: Vec::new(),
        needs_human_tasks: Vec::new(),
        current_task_id: None,
        current_task_show: None,
        current_task_status: None,
    };

    let result = run_loop(&mut state).expect_err("should exit after queue drained");
    assert_eq!(result.code, 0, "expected graceful exit");

    let hook_contents = fs::read_to_string(&hook_log).expect("read hook log");
    assert!(
        hook_contents.contains("hook args_count=1 args=--done"),
        "expected completed hook"
    );
    assert!(
        !hook_contents.contains("--human"),
        "requires-human hook should not run when closed within limit, got:\n{hook_contents}"
    );

    let update_contents = fs::read_to_string(&task_update_log).expect("read task-update log");
    assert_eq!(
        update_contents.matches("args=--status in_progress").count(),
        2,
        "expected task_update_in_progress to run once per solve loop, got:\n{update_contents}"
    );
    assert!(
        !update_contents.contains("args=--status blocked"),
        "should not block task when closed within limit, got:\n{update_contents}"
    );
}

#[test]
fn review_loop_limit_exhaustion_marks_blocked_and_requires_human() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();
    let temp = TempDir::new().expect("temp dir");
    let log_path = temp.path().join("trudger.log");
    let task_update_log = temp.path().join("task-update.log");
    let hook_log = temp.path().join("hook.log");

    let next_task_queue = temp.path().join("next-task-queue.txt");
    fs::write(&next_task_queue, "tr-1\n\n").expect("write next task queue");
    let status_queue = temp.path().join("status-queue.txt");
    fs::write(&status_queue, "ready\nopen\nopen\n").expect("write status queue");

    let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bin");
    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));
    env::set_var("NEXT_TASK_OUTPUT_QUEUE", &next_task_queue);
    env::set_var("TASK_STATUS_QUEUE", &status_queue);
    env::set_var("TASK_SHOW_OUTPUT", "SHOW_PAYLOAD");
    env::set_var("TASK_UPDATE_LOG", &task_update_log);
    env::set_var("HOOK_MOCK_LOG", &hook_log);

    let config = Config {
        agent_command: "codex --yolo exec --default".to_string(),
        agent_review_command: "codex --yolo exec --review \"$@\"".to_string(),
        commands: Commands {
            next_task: Some("next-task".to_string()),
            task_show: "task-show \"$@\"".to_string(),
            task_status: "task-status".to_string(),
            task_update_in_progress: "task-update \"$@\"".to_string(),
            reset_task: "reset-task \"$@\"".to_string(),
        },
        hooks: Hooks {
            on_completed: "hook --done".to_string(),
            on_requires_human: "hook --human".to_string(),
            on_doctor_setup: None,
        },
        review_loop_limit: 2,
        log_path: log_path.display().to_string(),
    };

    let mut state = RuntimeState {
        config,
        config_path: temp.path().join("trudger.yml"),
        prompt_trudge: "Task context".to_string(),
        prompt_review: "Review context".to_string(),
        logger: Logger::new(Some(log_path)),
        tmux: TmuxState::disabled(),
        interrupt_flag: Arc::new(AtomicBool::new(false)),
        manual_tasks: Vec::new(),
        completed_tasks: Vec::new(),
        needs_human_tasks: Vec::new(),
        current_task_id: None,
        current_task_show: None,
        current_task_status: None,
    };

    let result = run_loop(&mut state).expect_err("should exit after queue drained");
    assert_eq!(result.code, 0, "expected graceful exit");
    assert_eq!(state.needs_human_tasks, vec!["tr-1"]);
    assert!(state.completed_tasks.is_empty());

    let hook_contents = fs::read_to_string(&hook_log).expect("read hook log");
    assert!(
        hook_contents.contains("hook args_count=1 args=--human"),
        "expected requires-human hook after exhaustion, got:\n{hook_contents}"
    );
    assert!(
        hook_contents.contains("env TRUDGER_TASK_STATUS=blocked"),
        "expected hook to see blocked status after exhaustion, got:\n{hook_contents}"
    );
    assert!(
        !hook_contents.contains("--done"),
        "completed hook should not run after exhaustion, got:\n{hook_contents}"
    );

    let update_contents = fs::read_to_string(&task_update_log).expect("read task-update log");
    assert_eq!(
        update_contents.matches("args=--status in_progress").count(),
        2,
        "expected task_update_in_progress to run once per solve loop, got:\n{update_contents}"
    );
    assert!(
        update_contents.contains("args=--status blocked"),
        "expected task to be marked blocked after exhaustion, got:\n{update_contents}"
    );
}

#[test]
fn next_task_exit_1_exits_zero_without_running_commands() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();
    let temp = TempDir::new().expect("temp dir");
    let log_path = temp.path().join("trudger.log");
    let task_show_log = temp.path().join("task-show.log");
    let codex_log = temp.path().join("codex.log");

    let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bin");
    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));
    env::set_var("NEXT_TASK_EXIT_CODE", "1");
    env::set_var("TASK_SHOW_LOG", &task_show_log);
    env::set_var("CODEX_MOCK_LOG", &codex_log);

    let config = Config {
        agent_command: "codex --yolo exec --default".to_string(),
        agent_review_command: "codex --yolo exec --review \"$@\"".to_string(),
        commands: Commands {
            next_task: Some("next-task".to_string()),
            task_show: "task-show \"$@\"".to_string(),
            task_status: "task-status".to_string(),
            task_update_in_progress: "task-update \"$@\"".to_string(),
            reset_task: "reset-task \"$@\"".to_string(),
        },
        hooks: Hooks {
            on_completed: "hook --done".to_string(),
            on_requires_human: "hook --human".to_string(),
            on_doctor_setup: None,
        },
        review_loop_limit: 2,
        log_path: log_path.display().to_string(),
    };

    let mut state = RuntimeState {
        config,
        config_path: temp.path().join("trudger.yml"),
        prompt_trudge: "Task context".to_string(),
        prompt_review: "Review context".to_string(),
        logger: Logger::new(Some(log_path)),
        tmux: TmuxState::disabled(),
        interrupt_flag: Arc::new(AtomicBool::new(false)),
        manual_tasks: Vec::new(),
        completed_tasks: Vec::new(),
        needs_human_tasks: Vec::new(),
        current_task_id: None,
        current_task_show: None,
        current_task_status: None,
    };

    let result = run_loop(&mut state).expect_err("should exit when next-task returns exit 1");
    assert_eq!(result.code, 0, "expected graceful exit");
    assert!(
        !codex_log.exists(),
        "codex should not run when next_task exits 1"
    );
    assert!(
        !task_show_log.exists(),
        "task_show should not run when next_task exits 1"
    );
}

#[test]
fn hook_uses_env_task_id_in_shell() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();
    let temp = TempDir::new().expect("temp dir");
    let log_path = temp.path().join("trudger.log");
    let next_task_log = temp.path().join("next-task.log");
    let task_show_log = temp.path().join("task-show.log");
    let task_status_log = temp.path().join("task-status.log");
    let task_update_log = temp.path().join("task-update.log");
    let hook_log = temp.path().join("hook.log");
    let codex_log = temp.path().join("codex.log");

    let next_task_queue = temp.path().join("next-task-queue.txt");
    fs::write(&next_task_queue, "tr-55\n\n").expect("write next task queue");
    let status_queue = temp.path().join("status-queue.txt");
    fs::write(&status_queue, "ready\nclosed\n").expect("write status queue");

    let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bin");
    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));
    env::set_var("NEXT_TASK_LOG", &next_task_log);
    env::set_var("TASK_SHOW_LOG", &task_show_log);
    env::set_var("TASK_STATUS_LOG", &task_status_log);
    env::set_var("TASK_UPDATE_LOG", &task_update_log);
    env::set_var("HOOK_MOCK_LOG", &hook_log);
    env::set_var("CODEX_MOCK_LOG", &codex_log);
    env::set_var("NEXT_TASK_OUTPUT_QUEUE", &next_task_queue);
    env::set_var("TASK_STATUS_QUEUE", &status_queue);
    env::set_var("TASK_SHOW_OUTPUT", "SHOW_PAYLOAD");

    let config = Config {
        agent_command: "codex --yolo exec --default".to_string(),
        agent_review_command: "codex --yolo exec --review \"$@\"".to_string(),
        commands: Commands {
            next_task: Some("next-task\t--with-tab".to_string()),
            task_show: "task-show \"$@\"".to_string(),
            task_status: "task-status".to_string(),
            task_update_in_progress: "task-update \"$@\"".to_string(),
            reset_task: "reset-task \"$@\"".to_string(),
        },
        hooks: Hooks {
            on_completed: "hook --done \"$TRUDGER_TASK_ID\"".to_string(),
            on_requires_human: "hook --human \"$TRUDGER_TASK_ID\"".to_string(),
            on_doctor_setup: None,
        },
        review_loop_limit: 2,
        log_path: log_path.display().to_string(),
    };

    let mut state = RuntimeState {
        config,
        config_path: temp.path().join("trudger.yml"),
        prompt_trudge: "Task context".to_string(),
        prompt_review: "Review context".to_string(),
        logger: Logger::new(Some(log_path)),
        tmux: TmuxState::disabled(),
        interrupt_flag: Arc::new(AtomicBool::new(false)),
        manual_tasks: Vec::new(),
        completed_tasks: Vec::new(),
        needs_human_tasks: Vec::new(),
        current_task_id: None,
        current_task_show: None,
        current_task_status: None,
    };

    let result = run_loop(&mut state).expect_err("should exit after queue drained");
    assert_eq!(result.code, 0, "expected graceful exit");

    let hook_contents = fs::read_to_string(&hook_log).expect("read hook log");
    assert!(
        hook_contents.contains("hook args_count=2 args=--done tr-55"),
        "hook should receive task id via TRUDGER_TASK_ID"
    );
}

#[test]
fn skip_not_ready_respects_limit() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();
    let temp = TempDir::new().expect("temp dir");
    let log_path = temp.path().join("trudger.log");
    let task_show_log = temp.path().join("task-show.log");
    let codex_log = temp.path().join("codex.log");

    let next_task_queue = temp.path().join("next-task-queue.txt");
    fs::write(&next_task_queue, "tr-1\ntr-2\n").expect("write next task queue");
    let status_queue = temp.path().join("status-queue.txt");
    fs::write(&status_queue, "blocked\nstalled\n").expect("write status queue");

    let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bin");
    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));
    env::set_var("NEXT_TASK_OUTPUT_QUEUE", &next_task_queue);
    env::set_var("TASK_STATUS_QUEUE", &status_queue);
    env::set_var("TASK_SHOW_LOG", &task_show_log);
    env::set_var("CODEX_MOCK_LOG", &codex_log);
    env::set_var("TRUDGER_SKIP_NOT_READY_LIMIT", "2");

    let config = Config {
        agent_command: "codex --yolo exec --default".to_string(),
        agent_review_command: "codex --yolo exec --review \"$@\"".to_string(),
        commands: Commands {
            next_task: Some("next-task".to_string()),
            task_show: "task-show \"$@\"".to_string(),
            task_status: "task-status".to_string(),
            task_update_in_progress: "task-update \"$@\"".to_string(),
            reset_task: "reset-task \"$@\"".to_string(),
        },
        hooks: Hooks {
            on_completed: "hook --done".to_string(),
            on_requires_human: "hook --human".to_string(),
            on_doctor_setup: None,
        },
        review_loop_limit: 2,
        log_path: log_path.display().to_string(),
    };

    let mut state = RuntimeState {
        config,
        config_path: temp.path().join("trudger.yml"),
        prompt_trudge: "Task context".to_string(),
        prompt_review: "Review context".to_string(),
        logger: Logger::new(Some(log_path)),
        tmux: TmuxState::disabled(),
        interrupt_flag: Arc::new(AtomicBool::new(false)),
        manual_tasks: Vec::new(),
        completed_tasks: Vec::new(),
        needs_human_tasks: Vec::new(),
        current_task_id: None,
        current_task_show: None,
        current_task_status: None,
    };

    let result = run_loop(&mut state).expect_err("should exit when no ready task found");
    assert_eq!(result.code, 0, "expected idle exit code");
    assert_eq!(result.reason, "no_ready_task");
    assert!(
        !codex_log.exists(),
        "codex should not run when tasks are not ready"
    );
    assert!(
        !task_show_log.exists(),
        "task_show should not run when tasks are not ready"
    );
}

#[test]
fn missing_status_after_review_errors() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();
    let temp = TempDir::new().expect("temp dir");
    let log_path = temp.path().join("trudger.log");
    let codex_log = temp.path().join("codex.log");

    let next_task_queue = temp.path().join("next-task-queue.txt");
    fs::write(&next_task_queue, "tr-1\n").expect("write next task queue");
    let status_queue = temp.path().join("status-queue.txt");
    fs::write(&status_queue, "ready\n\n").expect("write status queue");

    let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bin");
    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));
    env::set_var("NEXT_TASK_OUTPUT_QUEUE", &next_task_queue);
    env::set_var("TASK_STATUS_QUEUE", &status_queue);
    env::set_var("TASK_SHOW_OUTPUT", "SHOW_PAYLOAD");
    env::set_var("CODEX_MOCK_LOG", &codex_log);

    let config = Config {
        agent_command: "codex --yolo exec --default".to_string(),
        agent_review_command: "codex --yolo exec --review \"$@\"".to_string(),
        commands: Commands {
            next_task: Some("next-task".to_string()),
            task_show: "task-show \"$@\"".to_string(),
            task_status: "task-status".to_string(),
            task_update_in_progress: "task-update \"$@\"".to_string(),
            reset_task: "reset-task \"$@\"".to_string(),
        },
        hooks: Hooks {
            on_completed: "hook --done".to_string(),
            on_requires_human: "hook --human".to_string(),
            on_doctor_setup: None,
        },
        review_loop_limit: 2,
        log_path: log_path.display().to_string(),
    };

    let mut state = RuntimeState {
        config,
        config_path: temp.path().join("trudger.yml"),
        prompt_trudge: "Task context".to_string(),
        prompt_review: "Review context".to_string(),
        logger: Logger::new(Some(log_path)),
        tmux: TmuxState::disabled(),
        interrupt_flag: Arc::new(AtomicBool::new(false)),
        manual_tasks: Vec::new(),
        completed_tasks: Vec::new(),
        needs_human_tasks: Vec::new(),
        current_task_id: None,
        current_task_show: None,
        current_task_status: None,
    };

    let result = run_loop(&mut state).expect_err("should error on missing status");
    assert_eq!(result.code, 1);
    assert!(
        result.reason.contains("task_missing_status_after_review"),
        "expected missing status reason, got: {}",
        result.reason
    );
}

#[test]
fn reset_task_runs_on_exit_with_active_task() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();
    let temp = TempDir::new().expect("temp dir");
    let reset_task_log = temp.path().join("reset-task.log");

    let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bin");
    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));
    env::set_var("RESET_TASK_LOG", &reset_task_log);

    let config = Config {
        agent_command: "codex --yolo exec --default".to_string(),
        agent_review_command: "codex --yolo exec --review \"$@\"".to_string(),
        commands: Commands {
            next_task: Some("next-task".to_string()),
            task_show: "task-show \"$@\"".to_string(),
            task_status: "task-status".to_string(),
            task_update_in_progress: "task-update \"$@\"".to_string(),
            reset_task: "reset-task \"$@\"".to_string(),
        },
        hooks: Hooks {
            on_completed: "hook --done".to_string(),
            on_requires_human: "hook --human".to_string(),
            on_doctor_setup: None,
        },
        review_loop_limit: 2,
        log_path: temp.path().join("trudger.log").display().to_string(),
    };

    let state = RuntimeState {
        config,
        config_path: temp.path().join("trudger.yml"),
        prompt_trudge: "Task context".to_string(),
        prompt_review: "Review context".to_string(),
        logger: Logger::new(None),
        tmux: TmuxState::disabled(),
        interrupt_flag: Arc::new(AtomicBool::new(false)),
        manual_tasks: Vec::new(),
        completed_tasks: Vec::new(),
        needs_human_tasks: Vec::new(),
        current_task_id: Some("tr-1".to_string()),
        current_task_show: None,
        current_task_status: None,
    };

    let result = Err(Quit {
        code: 1,
        reason: "error".to_string(),
    });
    reset_task_on_exit(&state, &result);

    let contents = fs::read_to_string(&reset_task_log).expect("read reset task log");
    assert!(
        contents.contains("reset-task args_count=0 args="),
        "reset-task should run without args"
    );
    assert!(
        contents.contains("env TRUDGER_TASK_ID=tr-1"),
        "reset-task should receive task id in env"
    );
}

#[test]
fn parse_manual_tasks_trims_and_rejects_empty_segments() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let tasks =
        parse_manual_tasks(&[" tr-1, tr-2 ".to_string(), "tr-3".to_string()]).expect("parse tasks");
    assert_eq!(tasks, vec!["tr-1", "tr-2", "tr-3"]);

    let err = parse_manual_tasks(&["tr-1,,tr-2".to_string()]).expect_err("should error");
    assert!(
        err.contains("empty segment"),
        "expected empty segment error, got: {err}"
    );
}

#[test]
fn clap_parses_doctor_subcommand_and_positional_args() {
    let cli = Cli::try_parse_from(["trudger", "doctor"]).expect("parse doctor");
    assert!(
        matches!(cli.command, Some(CliCommand::Doctor)),
        "expected doctor subcommand"
    );
    assert!(
        cli.positional.is_empty(),
        "doctor should have no positionals"
    );

    let cli = Cli::try_parse_from(["trudger", "doctor", "-t", "tr-1"]).expect("parse doctor");
    assert!(
        matches!(cli.command, Some(CliCommand::Doctor)),
        "expected doctor subcommand"
    );
    assert_eq!(cli.task, vec!["tr-1"], "expected task flag capture");

    let cli = Cli::try_parse_from(["trudger", "tr-1"]).expect("parse positional");
    assert!(
        cli.command.is_none(),
        "positional should not be a subcommand"
    );
    assert_eq!(cli.positional, vec!["tr-1"]);
}

#[test]
fn doctor_rejects_task_flag_with_clear_error() {
    let err = run_with_cli(Cli {
        config: None,
        task: vec!["tr-1".to_string()],
        positional: Vec::new(),
        command: Some(CliCommand::Doctor),
    })
    .expect_err("expected doctor task-flag rejection");
    assert_eq!(err.code, 1);
    assert!(
        err.reason.contains("-t/--task"),
        "expected -t/--task error, got: {}",
        err.reason
    );
}

#[test]
fn positional_task_ids_are_rejected_with_migration_hint() {
    let err = run_with_cli(Cli {
        config: None,
        task: Vec::new(),
        positional: vec!["tr-1".to_string()],
        command: None,
    })
    .expect_err("expected positional task id rejection");
    assert_eq!(err.code, 1);
    assert_eq!(err.reason, "positional_args_not_supported");
}

#[test]
fn doctor_does_not_require_prompts_and_cleans_scratch_dir() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let old_home = env::var_os("HOME");
    let original_cwd = env::current_dir().expect("cwd");
    let temp = TempDir::new().expect("temp dir");
    let invocation = temp.path().join("invocation");
    fs::create_dir_all(&invocation).expect("create invocation dir");
    env::set_current_dir(&invocation).expect("set cwd");

    let hook_log = temp.path().join("hook.log");
    let next_task_log = temp.path().join("next-task.log");
    let task_show_log = temp.path().join("task-show.log");
    let task_status_log = temp.path().join("task-status.log");
    let task_update_log = temp.path().join("task-update.log");
    let reset_task_log = temp.path().join("reset-task.log");
    env::set_var("HOOK_MOCK_LOG", &hook_log);
    env::set_var("NEXT_TASK_LOG", &next_task_log);
    env::set_var("TASK_SHOW_LOG", &task_show_log);
    env::set_var("TASK_STATUS_LOG", &task_status_log);
    env::set_var("TASK_UPDATE_LOG", &task_update_log);
    env::set_var("RESET_TASK_LOG", &reset_task_log);

    // Ensure the setup hook sees these as unset even if present in the parent process env.
    env::set_var("TRUDGER_TASK_ID", "PARENT_TASK");
    env::set_var("TRUDGER_TASK_SHOW", "PARENT_SHOW");
    env::set_var("TRUDGER_TASK_STATUS", "PARENT_STATUS");
    env::set_var("TRUDGER_PROMPT", "PARENT_PROMPT");
    env::set_var("TRUDGER_REVIEW_PROMPT", "PARENT_REVIEW_PROMPT");

    let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bin");
    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));

    let beads_dir = invocation.join(".beads");
    fs::create_dir_all(&beads_dir).expect("create .beads dir");
    fs::write(
        beads_dir.join("issues.jsonl"),
        r#"{"id":"tr-open","status":"open"}
{"id":"tr-closed","status":"closed"}
"#,
    )
    .expect("write issues.jsonl");

    env::set_var("NEXT_TASK_OUTPUT", "tr-open");
    env::set_var("TASK_SHOW_OUTPUT", "SHOW_PAYLOAD");
    let status_queue = temp.path().join("status-queue.txt");
    fs::write(&status_queue, "open\nin_progress\nopen\nclosed\n").expect("write status queue");
    env::set_var("TASK_STATUS_QUEUE", &status_queue);

    let config_path = temp.path().join("trudger.yml");
    fs::write(
        &config_path,
        r#"
agent_command: "agent"
agent_review_command: "agent-review"
commands:
  next_task: "next-task"
  task_show: "task-show"
  task_status: "task-status"
  task_update_in_progress: "task-update \"$@\""
  reset_task: "reset-task"
review_loop_limit: 2
log_path: "./.trudger.log"
hooks:
  on_completed: "hook --done"
  on_requires_human: "hook --human"
  on_doctor_setup: 'hook --doctor-setup; rm -rf "$TRUDGER_DOCTOR_SCRATCH_DIR/.beads"; cp -R ".beads" "$TRUDGER_DOCTOR_SCRATCH_DIR/"'
"#,
    )
    .expect("write config");

    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("create home");
    env::set_var("HOME", &home);

    let cli = Cli {
        config: Some(config_path.clone()),
        task: Vec::new(),
        positional: Vec::new(),
        command: Some(CliCommand::Doctor),
    };

    run_with_cli(cli).expect("doctor should succeed without prompts");

    let hook_contents = fs::read_to_string(&hook_log).expect("read hook log");
    assert!(
        hook_contents.contains(&format!("cwd {}", invocation.display())),
        "setup hook should run from invocation cwd, got:\n{hook_contents}"
    );
    assert!(
        hook_contents.contains("envset TRUDGER_CONFIG_PATH=1"),
        "setup hook should receive TRUDGER_CONFIG_PATH"
    );
    assert!(
        hook_contents.contains("envset TRUDGER_DOCTOR_SCRATCH_DIR=1"),
        "setup hook should receive TRUDGER_DOCTOR_SCRATCH_DIR"
    );
    assert!(
        hook_contents.contains("envset TRUDGER_TASK_ID=0"),
        "setup hook should not receive TRUDGER_TASK_ID"
    );
    assert!(
        hook_contents.contains("envset TRUDGER_TASK_SHOW=0"),
        "setup hook should not receive TRUDGER_TASK_SHOW"
    );
    assert!(
        hook_contents.contains("envset TRUDGER_TASK_STATUS=0"),
        "setup hook should not receive TRUDGER_TASK_STATUS"
    );
    assert!(
        hook_contents.contains("envset TRUDGER_PROMPT=0"),
        "setup hook should not receive TRUDGER_PROMPT"
    );
    assert!(
        hook_contents.contains("envset TRUDGER_REVIEW_PROMPT=0"),
        "setup hook should not receive TRUDGER_REVIEW_PROMPT"
    );

    let scratch_dir = hook_contents
        .lines()
        .find_map(|line| line.strip_prefix("env TRUDGER_DOCTOR_SCRATCH_DIR="))
        .unwrap_or("")
        .trim()
        .to_string();
    assert!(!scratch_dir.is_empty(), "missing scratch dir in hook log");

    let next_task_contents = fs::read_to_string(&next_task_log).expect("read next-task log");
    assert!(
        next_task_contents.contains(&format!("cwd {}", scratch_dir)),
        "next-task should run from scratch dir, got:\n{next_task_contents}"
    );
    assert!(
        next_task_contents.contains("envset TRUDGER_DOCTOR_SCRATCH_DIR=1"),
        "next-task should receive TRUDGER_DOCTOR_SCRATCH_DIR"
    );
    assert!(
        next_task_contents.contains("envset TRUDGER_TASK_ID=0"),
        "next-task should not receive TRUDGER_TASK_ID, got:\n{next_task_contents}"
    );

    let task_show_contents = fs::read_to_string(&task_show_log).expect("read task-show log");
    assert!(
        task_show_contents.contains(&format!("cwd {}", scratch_dir)),
        "task-show should run from scratch dir, got:\n{task_show_contents}"
    );
    assert!(
        task_show_contents.contains("env TRUDGER_TASK_ID=tr-open"),
        "task-show should receive TRUDGER_TASK_ID=tr-open, got:\n{task_show_contents}"
    );
    assert!(
        task_show_contents.contains("envset TRUDGER_PROMPT=0"),
        "task-show should not receive TRUDGER_PROMPT, got:\n{task_show_contents}"
    );

    let task_status_contents = fs::read_to_string(&task_status_log).expect("read task-status log");
    assert!(
        task_status_contents.contains(&format!("cwd {}", scratch_dir)),
        "task-status should run from scratch dir, got:\n{task_status_contents}"
    );
    assert!(
        task_status_contents.contains("env TRUDGER_TASK_ID=tr-open"),
        "task-status should run for tr-open, got:\n{task_status_contents}"
    );
    assert!(
        task_status_contents.contains("env TRUDGER_TASK_ID=tr-closed"),
        "task-status should run for tr-closed (closed parsing), got:\n{task_status_contents}"
    );

    let task_update_contents = fs::read_to_string(&task_update_log).expect("read task-update log");
    assert!(
        task_update_contents.contains(&format!("cwd {}", scratch_dir)),
        "task-update should run from scratch dir, got:\n{task_update_contents}"
    );
    assert!(
        task_update_contents.contains("args_count=2 args=--status in_progress"),
        "task-update should set in_progress, got:\n{task_update_contents}"
    );

    let reset_task_contents = fs::read_to_string(&reset_task_log).expect("read reset-task log");
    assert!(
        reset_task_contents.contains(&format!("cwd {}", scratch_dir)),
        "reset-task should run from scratch dir, got:\n{reset_task_contents}"
    );
    assert_eq!(
        reset_task_contents
            .matches("reset-task args_count=0 args=")
            .count(),
        2,
        "expected reset-task to run twice, got:\n{reset_task_contents}"
    );

    assert!(
        hook_contents.contains(&format!("cwd {}", scratch_dir)),
        "hooks should run from scratch dir after setup, got:\n{hook_contents}"
    );
    assert!(
        hook_contents.contains("hook args_count=1 args=--done"),
        "doctor should execute hooks.on_completed, got:\n{hook_contents}"
    );
    assert!(
        hook_contents.contains("hook args_count=1 args=--human"),
        "doctor should execute hooks.on_requires_human, got:\n{hook_contents}"
    );
    assert!(
        !Path::new(&scratch_dir).exists(),
        "scratch dir should be cleaned up: {scratch_dir}"
    );

    env::set_current_dir(&original_cwd).expect("restore cwd");
    match old_home {
        Some(value) => env::set_var("HOME", value),
        None => env::remove_var("HOME"),
    };
}

#[test]
fn doctor_cleanup_failure_is_an_error() {
    use std::os::unix::fs::PermissionsExt;

    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let original_cwd = env::current_dir().expect("cwd");
    let temp = TempDir::new().expect("temp dir");
    let invocation = temp.path().join("invocation");
    fs::create_dir_all(&invocation).expect("create invocation dir");
    env::set_current_dir(&invocation).expect("set cwd");

    let scratch_path_file = temp.path().join("scratch-path.txt");
    let hook = format!(
        "printf '%s' \"$TRUDGER_DOCTOR_SCRATCH_DIR\" > \"{}\"; \
             mkdir -p \"$TRUDGER_DOCTOR_SCRATCH_DIR/locked\"; \
             printf 'hi' > \"$TRUDGER_DOCTOR_SCRATCH_DIR/locked/file\"; \
             chmod 555 \"$TRUDGER_DOCTOR_SCRATCH_DIR/locked\"",
        scratch_path_file.display()
    );

    let config = Config {
        agent_command: "agent".to_string(),
        agent_review_command: "agent-review".to_string(),
        commands: Commands {
            next_task: Some("next-task".to_string()),
            task_show: "task-show".to_string(),
            task_status: "task-status".to_string(),
            task_update_in_progress: "task-update".to_string(),
            reset_task: "reset-task".to_string(),
        },
        hooks: Hooks {
            on_completed: "true".to_string(),
            on_requires_human: "true".to_string(),
            on_doctor_setup: Some(hook),
        },
        review_loop_limit: 2,
        log_path: temp.path().join("trudger.log").display().to_string(),
    };
    let logger = Logger::new(None);

    let result = run_doctor_mode(&config, &temp.path().join("trudger.yml"), &logger)
        .expect_err("expected cleanup failure");
    assert_eq!(result.code, 1);
    assert!(
        result.reason.contains("doctor scratch cleanup failed"),
        "expected cleanup failure reason, got: {}",
        result.reason
    );

    let scratch_dir = fs::read_to_string(&scratch_path_file)
        .unwrap_or_default()
        .trim()
        .to_string();
    if !scratch_dir.is_empty() {
        let locked_dir = Path::new(&scratch_dir).join("locked");
        let _ = fs::set_permissions(&locked_dir, fs::Permissions::from_mode(0o755));
        let _ = fs::remove_dir_all(&scratch_dir);
    }

    env::set_current_dir(&original_cwd).expect("restore cwd");
}

#[test]
fn sample_configs_load_and_validate() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for name in ["trudgeable-with-hooks", "robot-triage"] {
        let path = root
            .join("sample_configuration")
            .join(format!("{}.yml", name));
        let loaded = load_config(&path).expect("load sample config");
        validate_config(&loaded.config, &[]).expect("validate sample config");
        let hook = loaded
            .config
            .hooks
            .on_doctor_setup
            .as_deref()
            .unwrap_or("")
            .trim();
        assert!(
            !hook.is_empty(),
            "sample config {} should include hooks.on_doctor_setup",
            name
        );
    }
}
