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

use crate::app::{main_with_args, render_prompt, require_file, run_with_args, run_with_cli};
use crate::cli::{parse_manual_tasks, Cli, CliCommand};
use crate::config::{load_config, Commands, Config, Hooks};
use crate::doctor::run_doctor_mode;
use crate::logger::{sanitize_log_value, Logger};
use crate::run_loop::{reset_task_on_exit, run_loop, validate_config, Quit, RuntimeState};
use crate::shell::render_args;
use crate::tmux::{build_tmux_name, TmuxState};

pub(crate) static ENV_MUTEX: Mutex<()> = Mutex::new(());
static ORIGINAL_PATH: OnceLock<Option<std::ffi::OsString>> = OnceLock::new();

pub(crate) fn reset_test_env() {
    let original_path = ORIGINAL_PATH.get_or_init(|| env::var_os("PATH"));
    match original_path {
        Some(value) => env::set_var("PATH", value),
        None => env::remove_var("PATH"),
    }
    for key in [
        "NEXT_TASK_EXIT_CODE",
        "NEXT_TASK_OUTPUT_QUEUE",
        "NEXT_TASK_OUTPUT",
        "TASK_STATUS_EXIT_CODE",
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
        "TRUDGER_COMPLETED",
        "TRUDGER_NEEDS_HUMAN",
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

#[cfg(unix)]
fn capture_stderr<F: FnOnce()>(f: F) -> String {
    use std::io::Read;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::raw::c_int;

    extern "C" {
        fn pipe(fds: *mut c_int) -> c_int;
        fn dup(fd: c_int) -> c_int;
        fn dup2(oldfd: c_int, newfd: c_int) -> c_int;
        fn close(fd: c_int) -> c_int;
    }

    unsafe {
        let mut fds = [0 as c_int; 2];
        if pipe(fds.as_mut_ptr()) != 0 {
            panic!("pipe failed");
        }
        let read_fd = fds[0];
        let write_fd = fds[1];

        let stderr_fd = std::io::stderr().as_raw_fd();
        let saved_stderr_fd = dup(stderr_fd);
        if saved_stderr_fd < 0 {
            let _ = close(read_fd);
            let _ = close(write_fd);
            panic!("dup stderr failed");
        }

        if dup2(write_fd, stderr_fd) < 0 {
            let _ = close(saved_stderr_fd);
            let _ = close(read_fd);
            let _ = close(write_fd);
            panic!("dup2 stderr failed");
        }
        let _ = close(write_fd);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));

        if dup2(saved_stderr_fd, stderr_fd) < 0 {
            let _ = close(saved_stderr_fd);
            let _ = close(read_fd);
            panic!("dup2 restore stderr failed");
        }
        let _ = close(saved_stderr_fd);

        let mut output = Vec::new();
        let mut reader = std::fs::File::from_raw_fd(read_fd);
        reader.read_to_end(&mut output).expect("read stderr");
        let output = String::from_utf8_lossy(&output).into_owned();

        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }

        output
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

#[cfg(unix)]
#[test]
fn log_transition_warns_once_and_disables_after_error() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();
    let temp = TempDir::new().expect("temp dir");
    let log_dir = temp.path().join("missing-log-dir");
    let log_path = log_dir.join("trudger.log");
    let logger = Logger::new(Some(log_path.clone()));

    let stderr = capture_stderr(|| {
        logger.log_transition("first");
        fs::create_dir(&log_dir).expect("create log dir");
        logger.log_transition("second");
    });

    let lines: Vec<&str> = stderr
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    assert_eq!(lines.len(), 1, "expected one warning line, got: {stderr:?}");
    assert!(
        lines[0].contains("log_path="),
        "warning should include log_path=, got: {stderr:?}"
    );
    assert!(
        lines[0].contains(&log_path.display().to_string()),
        "warning should include the log path, got: {stderr:?}"
    );
    assert!(
        lines[0].contains("io_error="),
        "warning should include io_error=, got: {stderr:?}"
    );
    assert!(
        !log_path.exists(),
        "logging should be disabled after first error; log file unexpectedly exists at {}",
        log_path.display()
    );
}

#[cfg(unix)]
#[test]
fn log_transition_warns_once_under_concurrency() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();
    let temp = TempDir::new().expect("temp dir");
    let log_dir = temp.path().join("missing-log-dir");
    let log_path = log_dir.join("trudger.log");
    let logger = Arc::new(Logger::new(Some(log_path.clone())));

    let threads = 16;
    let barrier = Arc::new(std::sync::Barrier::new(threads));

    let stderr = capture_stderr(|| {
        let mut handles = Vec::new();
        for index in 0..threads {
            let logger = Arc::clone(&logger);
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                logger.log_transition(&format!("msg-{index}"));
            }));
        }
        for handle in handles {
            handle.join().expect("join");
        }
    });

    let lines: Vec<&str> = stderr
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    assert_eq!(
        lines.len(),
        1,
        "expected exactly one warning under concurrency, got: {stderr:?}"
    );
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
fn render_prompt_errors_when_file_is_missing() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();
    let temp = TempDir::new().expect("temp dir");
    let missing = temp.path().join("missing.md");
    let err = render_prompt(&missing).expect_err("expected missing prompt error");
    assert!(
        err.contains("Failed to read prompt"),
        "expected read prompt error, got: {err}"
    );
}

#[test]
fn render_prompt_can_return_empty_when_only_frontmatter() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();
    let mut file = NamedTempFile::new().expect("temp file");
    writeln!(file, "---\nname: test\n---").expect("write");
    let rendered = render_prompt(file.path()).expect("render");
    assert_eq!(rendered, "");
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
    assert!(
        hook_contents.contains("env TRUDGER_COMPLETED=tr-1"),
        "hook should see completed task IDs in TRUDGER_COMPLETED, got:\n{hook_contents}"
    );
    assert!(
        hook_contents.contains("env TRUDGER_NEEDS_HUMAN=tr-2"),
        "hook should see needs-human task IDs in TRUDGER_NEEDS_HUMAN, got:\n{hook_contents}"
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
            // Keep the hook running briefly so the interrupter can reliably observe its log.
            on_completed: "hook --done; sleep 0.05".to_string(),
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
    interrupter.join().expect("interrupter thread");
    assert_eq!(result.code, 130, "expected interrupt exit");
    assert_eq!(result.reason, "interrupted");
    assert_eq!(state.completed_tasks, vec!["tr-1"]);

    assert!(
        !next_task_log.exists(),
        "next-task should not run when manual tasks are provided"
    );

    assert!(
        task_show_log.exists(),
        "task-show should run for manual tasks"
    );
    assert!(
        task_update_log.exists(),
        "task-update should run for manual tasks"
    );
    assert!(
        task_status_log.exists(),
        "task-status should run for manual tasks"
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
    let task_status_log = temp.path().join("task-status.log");
    let reset_task_log = temp.path().join("reset-task.log");
    let status_queue = temp.path().join("status-queue.txt");
    fs::write(&status_queue, "in_progress\n").expect("write status queue");

    let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bin");
    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));
    env::set_var("TASK_STATUS_LOG", &task_status_log);
    env::set_var("TASK_STATUS_QUEUE", &status_queue);
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

    assert!(
        task_status_log.exists(),
        "task-status should run to confirm in_progress at exit"
    );

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

#[test]
fn main_function_runs_under_test_harness_args() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();
    // Simulate test-harness flags. Calling `crate::main()` directly can end up running
    // the real Trudger loop if the test binary has no args.
    let _ = main_with_args(vec![
        std::ffi::OsString::from("trudger"),
        std::ffi::OsString::from("--nocapture"),
    ]);
}

#[test]
fn run_with_args_returns_quit_on_cli_parse_failure() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let quit = run_with_args(vec![
        std::ffi::OsString::from("trudger"),
        std::ffi::OsString::from("--definitely-not-a-flag"),
    ])
    .expect_err("expected cli parse error");
    assert_eq!(quit.reason, "cli_parse");
    assert!(quit.code > 0);
}

#[cfg(unix)]
#[test]
fn main_with_args_returns_success_for_doctor_config() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let original_home = env::var_os("HOME");
    env::set_var("HOME", temp.path());

    let config = r#"
agent_command: "agent"
agent_review_command: "review"
commands:
  next_task: "exit 0"
  task_show: "printf 'SHOW'"
  task_status: |
    queue="$TRUDGER_DOCTOR_SCRATCH_DIR/status-queue.txt"
    tmp="$TRUDGER_DOCTOR_SCRATCH_DIR/status-queue.txt.tmp"
    line=""
    if [ -f "$queue" ]; then
      line=$(head -n 1 "$queue" || true)
      tail -n +2 "$queue" > "$tmp" || true
      mv "$tmp" "$queue"
      if [ -n "$line" ]; then printf '%s\n' "$line"; fi
    fi
  task_update_in_progress: "exit 0"
  reset_task: "exit 0"
review_loop_limit: 1
hooks:
  on_completed: "exit 0"
  on_requires_human: "exit 0"
  on_doctor_setup: |
    mkdir -p "$TRUDGER_DOCTOR_SCRATCH_DIR/.beads"
    printf '%s\n' '{"id":"tr-1","status":"open"}' > "$TRUDGER_DOCTOR_SCRATCH_DIR/.beads/issues.jsonl"
    printf 'open\nin_progress\nopen\nclosed\n' > "$TRUDGER_DOCTOR_SCRATCH_DIR/status-queue.txt"
"#;

    let mut config_file = NamedTempFile::new().expect("config");
    config_file
        .as_file_mut()
        .write_all(config.as_bytes())
        .expect("write config");

    let code = main_with_args(vec![
        std::ffi::OsString::from("trudger"),
        std::ffi::OsString::from("-c"),
        config_file.path().as_os_str().to_os_string(),
        std::ffi::OsString::from("doctor"),
    ]);

    match original_home {
        Some(value) => env::set_var("HOME", value),
        None => env::remove_var("HOME"),
    }
    reset_test_env();

    assert_eq!(code, std::process::ExitCode::SUCCESS);
}

#[cfg(unix)]
#[test]
fn log_transition_disables_after_write_error() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();
    let logger = Logger::new(Some(std::path::PathBuf::from("/dev/full")));

    let stderr = capture_stderr(|| {
        logger.log_transition("first");
        logger.log_transition("second");
    });

    let lines: Vec<&str> = stderr
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    assert_eq!(lines.len(), 1);
}

#[cfg(unix)]
#[test]
fn render_args_falls_back_when_bash_unavailable() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    env::set_var("PATH", temp.path());
    let rendered = render_args(&["with space".to_string(), "tab\targ".to_string()]);
    assert!(rendered.ends_with(' '));

    reset_test_env();
}

#[cfg(unix)]
#[test]
fn render_args_falls_back_when_bash_exits_nonzero() {
    use std::os::unix::fs::PermissionsExt;

    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let bin = temp.path().join("bin");
    fs::create_dir_all(&bin).expect("create bin dir");
    let bash = bin.join("bash");
    fs::write(&bash, "#!/usr/bin/env sh\nexit 1\n").expect("write bash");
    fs::set_permissions(&bash, fs::Permissions::from_mode(0o755)).expect("chmod bash");

    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", bin.display(), old_path));

    let rendered = render_args(&["tab\targ".to_string()]);
    assert!(rendered.ends_with(' '));

    reset_test_env();
}

#[cfg(unix)]
#[test]
fn command_exists_handles_missing_path_and_symlinks() {
    use std::os::unix::fs::symlink;
    use std::os::unix::fs::PermissionsExt;

    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let bin = temp.path().join("bin");
    fs::create_dir_all(&bin).expect("create bin dir");

    let real = bin.join("real");
    fs::write(&real, "#!/usr/bin/env bash\nexit 0\n").expect("write real");
    fs::set_permissions(&real, fs::Permissions::from_mode(0o755)).expect("chmod real");
    let link = bin.join("link");
    symlink(&real, &link).expect("symlink");

    let not_exec = bin.join("not_exec");
    fs::write(&not_exec, "#!/usr/bin/env bash\nexit 0\n").expect("write not_exec");
    fs::set_permissions(&not_exec, fs::Permissions::from_mode(0o644)).expect("chmod not_exec");
    let link_not_exec = bin.join("link_not_exec");
    symlink(&not_exec, &link_not_exec).expect("symlink not_exec");

    let dircmd = bin.join("dircmd");
    fs::create_dir_all(&dircmd).expect("create dircmd");

    let dangling_target = bin.join("missing-target");
    let dangling = bin.join("dangling");
    symlink(&dangling_target, &dangling).expect("symlink dangling");

    env::set_var("PATH", bin.display().to_string());
    assert!(crate::shell::command_exists("real"));
    assert!(crate::shell::command_exists("link"));
    assert!(!crate::shell::command_exists("not_exec"));
    assert!(!crate::shell::command_exists("link_not_exec"));
    assert!(!crate::shell::command_exists("dircmd"));
    assert!(!crate::shell::command_exists("dangling"));

    env::remove_var("PATH");
    assert!(!crate::shell::command_exists("real"));

    reset_test_env();
}

#[test]
fn run_shell_command_noops_when_command_is_empty() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let logger = Logger::new(None);
    let env = crate::shell::CommandEnv {
        cwd: None,
        config_path: "config".to_string(),
        scratch_dir: None,
        task_id: None,
        task_show: None,
        task_status: None,
        prompt: None,
        review_prompt: None,
        completed: None,
        needs_human: None,
    };

    let result = crate::shell::run_shell_command_capture("", "label", "none", &[], &env, &logger)
        .expect("capture should succeed");
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout, "");

    let exit = crate::shell::run_shell_command_status("", "label", "none", &[], &env, &logger)
        .expect("status should succeed");
    assert_eq!(exit, 0);
}

#[cfg(unix)]
#[test]
fn run_shell_command_errors_when_bash_is_missing() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    env::set_var("PATH", temp.path());

    let logger = Logger::new(None);
    let env = crate::shell::CommandEnv {
        cwd: None,
        config_path: "config".to_string(),
        scratch_dir: None,
        task_id: None,
        task_show: None,
        task_status: None,
        prompt: None,
        review_prompt: None,
        completed: None,
        needs_human: None,
    };

    let err = crate::shell::run_shell_command_capture("true", "label", "none", &[], &env, &logger)
        .expect_err("expected capture failure");
    assert!(err.contains("Failed to run command"));

    let err = crate::shell::run_shell_command_status("true", "label", "none", &[], &env, &logger)
        .expect_err("expected status failure");
    assert!(err.contains("Failed to run command"));

    reset_test_env();
}

#[test]
fn quit_sanitizes_empty_reason_and_exit_code_is_exposed() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let logger = Logger::new(None);
    let quit = crate::run_loop::quit(&logger, "", 7);
    let _code = quit.exit_code();
}

#[test]
fn validate_config_rejects_missing_and_empty_values() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let base = Config {
        agent_command: "agent".to_string(),
        agent_review_command: "review".to_string(),
        commands: Commands {
            next_task: Some("next-task".to_string()),
            task_show: "task-show".to_string(),
            task_status: "task-status".to_string(),
            task_update_in_progress: "task-update".to_string(),
            reset_task: "reset-task".to_string(),
        },
        hooks: Hooks {
            on_completed: "hook --done".to_string(),
            on_requires_human: "hook --human".to_string(),
            on_doctor_setup: None,
        },
        review_loop_limit: 2,
        log_path: "".to_string(),
    };

    let mut config = base.clone();
    config.agent_command = "  ".to_string();
    assert!(validate_config(&config, &[]).is_err());

    let mut config = base.clone();
    config.agent_review_command = "\t".to_string();
    assert!(validate_config(&config, &[]).is_err());

    let mut config = base.clone();
    config.review_loop_limit = 0;
    assert!(validate_config(&config, &[]).is_err());

    let mut config = base.clone();
    config.commands.next_task = Some("".to_string());
    assert!(validate_config(&config, &[]).is_err());

    let mut config = base.clone();
    config.commands.next_task = None;
    assert!(validate_config(&config, &[]).is_err());

    let mut config = base.clone();
    config.commands.next_task = Some("".to_string());
    assert!(validate_config(&config, &["tr-1".to_string()]).is_ok());

    let mut config = base.clone();
    config.commands.task_show = "".to_string();
    assert!(validate_config(&config, &[]).is_err());

    let mut config = base.clone();
    config.commands.task_status = "".to_string();
    assert!(validate_config(&config, &[]).is_err());

    let mut config = base.clone();
    config.commands.task_update_in_progress = "".to_string();
    assert!(validate_config(&config, &[]).is_err());

    let mut config = base.clone();
    config.commands.reset_task = "".to_string();
    assert!(validate_config(&config, &[]).is_err());

    let mut config = base.clone();
    config.hooks.on_completed = "".to_string();
    assert!(validate_config(&config, &[]).is_err());

    let mut config = base.clone();
    config.hooks.on_requires_human = "".to_string();
    assert!(validate_config(&config, &[]).is_err());
}

#[test]
fn run_loop_errors_when_next_task_command_missing() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let config = Config {
        agent_command: "agent".to_string(),
        agent_review_command: "agent-review".to_string(),
        commands: Commands {
            next_task: None,
            task_show: "task-show".to_string(),
            task_status: "task-status".to_string(),
            task_update_in_progress: "task-update".to_string(),
            reset_task: "reset-task".to_string(),
        },
        hooks: Hooks {
            on_completed: "true".to_string(),
            on_requires_human: "true".to_string(),
            on_doctor_setup: None,
        },
        review_loop_limit: 2,
        log_path: "".to_string(),
    };

    let mut state = RuntimeState {
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
        current_task_id: None,
        current_task_show: None,
        current_task_status: None,
    };

    let err = run_loop(&mut state).expect_err("expected idle");
    assert_eq!(err.code, 0);
}

#[test]
fn run_loop_propagates_next_task_exit_code_other_than_1() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bin");
    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));

    env::set_var("NEXT_TASK_EXIT_CODE", "2");
    env::set_var("NEXT_TASK_OUTPUT", "tr-1");

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
            on_doctor_setup: None,
        },
        review_loop_limit: 2,
        log_path: "".to_string(),
    };

    let mut state = RuntimeState {
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
        current_task_id: None,
        current_task_show: None,
        current_task_status: None,
    };

    let err = run_loop(&mut state).expect_err("expected next_task failure");
    assert_eq!(err.code, 2);
}

#[test]
fn run_loop_errors_when_selected_task_has_empty_status() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bin");
    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));

    env::set_var("NEXT_TASK_OUTPUT", "tr-1");
    let status_queue = temp.path().join("status-queue.txt");
    fs::write(&status_queue, "\n").expect("write status queue");
    env::set_var("TASK_STATUS_QUEUE", &status_queue);

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
            on_doctor_setup: None,
        },
        review_loop_limit: 2,
        log_path: "".to_string(),
    };

    let mut state = RuntimeState {
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
        current_task_id: None,
        current_task_show: None,
        current_task_status: None,
    };

    let err = run_loop(&mut state).expect_err("expected missing status");
    assert_eq!(err.code, 1);
}

#[test]
fn run_loop_errors_when_manual_task_is_empty_string() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let config = Config {
        agent_command: "agent".to_string(),
        agent_review_command: "agent-review".to_string(),
        commands: Commands {
            next_task: None,
            task_show: "task-show".to_string(),
            task_status: "printf 'open\\n'".to_string(),
            task_update_in_progress: "task-update".to_string(),
            reset_task: "reset-task".to_string(),
        },
        hooks: Hooks {
            on_completed: "true".to_string(),
            on_requires_human: "true".to_string(),
            on_doctor_setup: None,
        },
        review_loop_limit: 2,
        log_path: "".to_string(),
    };

    let mut state = RuntimeState {
        config,
        config_path: temp.path().join("trudger.yml"),
        prompt_trudge: "Task context".to_string(),
        prompt_review: "Review context".to_string(),
        logger: Logger::new(None),
        tmux: TmuxState::disabled(),
        interrupt_flag: Arc::new(AtomicBool::new(false)),
        manual_tasks: vec!["".to_string()],
        completed_tasks: Vec::new(),
        needs_human_tasks: Vec::new(),
        current_task_id: None,
        current_task_show: None,
        current_task_status: None,
    };

    let err = run_loop(&mut state).expect_err("expected empty task");
    assert_eq!(err.code, 0);
}

#[test]
fn run_loop_errors_when_update_in_progress_fails() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let config = Config {
        agent_command: "agent".to_string(),
        agent_review_command: "agent-review".to_string(),
        commands: Commands {
            next_task: Some("printf 'tr-1'".to_string()),
            task_show: "task-show".to_string(),
            task_status: "printf 'open\\n'".to_string(),
            task_update_in_progress: "exit 1".to_string(),
            reset_task: "reset-task".to_string(),
        },
        hooks: Hooks {
            on_completed: "true".to_string(),
            on_requires_human: "true".to_string(),
            on_doctor_setup: None,
        },
        review_loop_limit: 2,
        log_path: "".to_string(),
    };

    let mut state = RuntimeState {
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
        current_task_id: None,
        current_task_show: None,
        current_task_status: None,
    };

    let err = run_loop(&mut state).expect_err("expected update failure");
    assert_eq!(err.code, 1);
}

#[test]
fn run_loop_errors_when_task_show_fails_during_solve() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let config = Config {
        agent_command: "agent".to_string(),
        agent_review_command: "agent-review".to_string(),
        commands: Commands {
            next_task: Some("printf 'tr-1'".to_string()),
            task_show: "exit 1".to_string(),
            task_status: "printf 'open\\n'".to_string(),
            task_update_in_progress: "true".to_string(),
            reset_task: "reset-task".to_string(),
        },
        hooks: Hooks {
            on_completed: "true".to_string(),
            on_requires_human: "true".to_string(),
            on_doctor_setup: None,
        },
        review_loop_limit: 2,
        log_path: "".to_string(),
    };

    let mut state = RuntimeState {
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
        current_task_id: None,
        current_task_show: None,
        current_task_status: None,
    };

    let err = run_loop(&mut state).expect_err("expected show failure");
    assert_eq!(err.code, 1);
}

#[test]
fn run_loop_errors_when_agent_solve_fails() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let config = Config {
        agent_command: "exit 1".to_string(),
        agent_review_command: "agent-review".to_string(),
        commands: Commands {
            next_task: Some("printf 'tr-1'".to_string()),
            task_show: "printf 'SHOW\\n'".to_string(),
            task_status: "printf 'open\\n'".to_string(),
            task_update_in_progress: "true".to_string(),
            reset_task: "reset-task".to_string(),
        },
        hooks: Hooks {
            on_completed: "true".to_string(),
            on_requires_human: "true".to_string(),
            on_doctor_setup: None,
        },
        review_loop_limit: 2,
        log_path: "".to_string(),
    };

    let mut state = RuntimeState {
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
        current_task_id: None,
        current_task_show: None,
        current_task_status: None,
    };

    let err = run_loop(&mut state).expect_err("expected solve failure");
    assert_eq!(err.code, 1);
}

#[cfg(unix)]
#[test]
fn run_loop_errors_when_task_show_fails_during_review() {
    use std::os::unix::fs::PermissionsExt;

    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let call_count = temp.path().join("task-show-count.txt");
    let task_show = temp.path().join("task-show");
    fs::write(
        &task_show,
        format!(
            "#!/usr/bin/env bash\nset -euo pipefail\ncount=0\nif [[ -f \"{0}\" ]]; then count=$(cat \"{0}\" || echo 0); fi\ncount=$((count+1))\necho \"$count\" > \"{0}\"\nif [[ $count -eq 1 ]]; then echo \"SHOW1\"; exit 0; fi\nexit 1\n",
            call_count.display()
        ),
    )
    .expect("write task-show");
    fs::set_permissions(&task_show, fs::Permissions::from_mode(0o755)).expect("chmod task-show");

    let config = Config {
        agent_command: "true".to_string(),
        agent_review_command: "true".to_string(),
        commands: Commands {
            next_task: Some("printf 'tr-1'".to_string()),
            task_show: task_show.display().to_string(),
            task_status: "printf 'open\\n'".to_string(),
            task_update_in_progress: "true".to_string(),
            reset_task: "reset-task".to_string(),
        },
        hooks: Hooks {
            on_completed: "true".to_string(),
            on_requires_human: "true".to_string(),
            on_doctor_setup: None,
        },
        review_loop_limit: 2,
        log_path: "".to_string(),
    };

    let mut state = RuntimeState {
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
        current_task_id: None,
        current_task_show: None,
        current_task_status: None,
    };

    let err = run_loop(&mut state).expect_err("expected review show failure");
    assert_eq!(err.code, 1);
}

#[test]
fn run_loop_errors_when_agent_review_fails() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let config = Config {
        agent_command: "true".to_string(),
        agent_review_command: "exit 1".to_string(),
        commands: Commands {
            next_task: Some("printf 'tr-1'".to_string()),
            task_show: "printf 'SHOW\\n'".to_string(),
            task_status: "printf 'open\\n'".to_string(),
            task_update_in_progress: "true".to_string(),
            reset_task: "reset-task".to_string(),
        },
        hooks: Hooks {
            on_completed: "true".to_string(),
            on_requires_human: "true".to_string(),
            on_doctor_setup: None,
        },
        review_loop_limit: 2,
        log_path: "".to_string(),
    };

    let mut state = RuntimeState {
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
        current_task_id: None,
        current_task_show: None,
        current_task_status: None,
    };

    let err = run_loop(&mut state).expect_err("expected review failure");
    assert_eq!(err.code, 1);
}

#[test]
fn run_loop_errors_when_on_completed_hook_fails() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bin");
    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));

    env::set_var("NEXT_TASK_OUTPUT", "tr-1");
    let status_queue = temp.path().join("status-queue.txt");
    fs::write(&status_queue, "open\nclosed\n").expect("write status queue");
    env::set_var("TASK_STATUS_QUEUE", &status_queue);

    let config = Config {
        agent_command: "true".to_string(),
        agent_review_command: "true".to_string(),
        commands: Commands {
            next_task: Some("next-task".to_string()),
            task_show: "task-show".to_string(),
            task_status: "task-status".to_string(),
            task_update_in_progress: "true".to_string(),
            reset_task: "reset-task".to_string(),
        },
        hooks: Hooks {
            on_completed: "exit 1".to_string(),
            on_requires_human: "true".to_string(),
            on_doctor_setup: None,
        },
        review_loop_limit: 2,
        log_path: "".to_string(),
    };

    let mut state = RuntimeState {
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
        current_task_id: None,
        current_task_show: None,
        current_task_status: None,
    };

    let err = run_loop(&mut state).expect_err("expected hook failure");
    assert_eq!(err.code, 1);
}

#[test]
fn run_loop_errors_when_on_requires_human_hook_fails_on_blocked_status() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bin");
    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));

    env::set_var("NEXT_TASK_OUTPUT", "tr-1");
    let status_queue = temp.path().join("status-queue.txt");
    fs::write(&status_queue, "open\nblocked\n").expect("write status queue");
    env::set_var("TASK_STATUS_QUEUE", &status_queue);

    let config = Config {
        agent_command: "true".to_string(),
        agent_review_command: "true".to_string(),
        commands: Commands {
            next_task: Some("next-task".to_string()),
            task_show: "task-show".to_string(),
            task_status: "task-status".to_string(),
            task_update_in_progress: "true".to_string(),
            reset_task: "reset-task".to_string(),
        },
        hooks: Hooks {
            on_completed: "true".to_string(),
            on_requires_human: "exit 1".to_string(),
            on_doctor_setup: None,
        },
        review_loop_limit: 2,
        log_path: "".to_string(),
    };

    let mut state = RuntimeState {
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
        current_task_id: None,
        current_task_show: None,
        current_task_status: None,
    };

    let err = run_loop(&mut state).expect_err("expected hook failure");
    assert_eq!(err.code, 1);
}

#[test]
fn run_loop_errors_when_blocked_status_update_fails_after_exhausting_review_loop() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bin");
    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));

    env::set_var("NEXT_TASK_OUTPUT", "tr-1");
    let status_queue = temp.path().join("status-queue.txt");
    fs::write(&status_queue, "open\nopen\n").expect("write status queue");
    env::set_var("TASK_STATUS_QUEUE", &status_queue);

    let config = Config {
        agent_command: "true".to_string(),
        agent_review_command: "true".to_string(),
        commands: Commands {
            next_task: Some("next-task".to_string()),
            task_show: "task-show".to_string(),
            task_status: "task-status".to_string(),
            task_update_in_progress: "if [[ \"$*\" == *\"blocked\"* ]]; then exit 1; fi; exit 0"
                .to_string(),
            reset_task: "reset-task".to_string(),
        },
        hooks: Hooks {
            on_completed: "true".to_string(),
            on_requires_human: "true".to_string(),
            on_doctor_setup: None,
        },
        review_loop_limit: 1,
        log_path: "".to_string(),
    };

    let mut state = RuntimeState {
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
        current_task_id: None,
        current_task_show: None,
        current_task_status: None,
    };

    let err = run_loop(&mut state).expect_err("expected blocked update failure");
    assert_eq!(err.code, 1);
}

#[test]
fn run_loop_errors_when_on_requires_human_hook_fails_after_exhausting_review_loop() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bin");
    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));

    env::set_var("NEXT_TASK_OUTPUT", "tr-1");
    let status_queue = temp.path().join("status-queue.txt");
    fs::write(&status_queue, "open\nopen\n").expect("write status queue");
    env::set_var("TASK_STATUS_QUEUE", &status_queue);

    let config = Config {
        agent_command: "true".to_string(),
        agent_review_command: "true".to_string(),
        commands: Commands {
            next_task: Some("next-task".to_string()),
            task_show: "task-show".to_string(),
            task_status: "task-status".to_string(),
            task_update_in_progress: "true".to_string(),
            reset_task: "reset-task".to_string(),
        },
        hooks: Hooks {
            on_completed: "true".to_string(),
            on_requires_human: "exit 1".to_string(),
            on_doctor_setup: None,
        },
        review_loop_limit: 1,
        log_path: "".to_string(),
    };

    let mut state = RuntimeState {
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
        current_task_id: None,
        current_task_show: None,
        current_task_status: None,
    };

    let err = run_loop(&mut state).expect_err("expected hook failure");
    assert_eq!(err.code, 1);
}

#[test]
fn reset_task_on_exit_is_noop_for_ok_or_missing_task_id() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let logger = Logger::new(None);
    let state = RuntimeState {
        config: Config {
            agent_command: "agent".to_string(),
            agent_review_command: "review".to_string(),
            commands: Commands {
                next_task: None,
                task_show: "task-show".to_string(),
                task_status: "task-status".to_string(),
                task_update_in_progress: "task-update".to_string(),
                reset_task: "reset-task".to_string(),
            },
            hooks: Hooks {
                on_completed: "true".to_string(),
                on_requires_human: "true".to_string(),
                on_doctor_setup: None,
            },
            review_loop_limit: 2,
            log_path: "".to_string(),
        },
        config_path: temp.path().join("trudger.yml"),
        prompt_trudge: "Task context".to_string(),
        prompt_review: "Review context".to_string(),
        logger,
        tmux: TmuxState::disabled(),
        interrupt_flag: Arc::new(AtomicBool::new(false)),
        manual_tasks: Vec::new(),
        completed_tasks: Vec::new(),
        needs_human_tasks: Vec::new(),
        current_task_id: None,
        current_task_show: None,
        current_task_status: None,
    };

    reset_task_on_exit(&state, &Ok(()));
    reset_task_on_exit(
        &state,
        &Err(Quit {
            code: 1,
            reason: "error".to_string(),
        }),
    );
}

#[test]
fn reset_task_on_exit_logs_failure_when_reset_task_fails() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let state = RuntimeState {
        config: Config {
            agent_command: "agent".to_string(),
            agent_review_command: "review".to_string(),
            commands: Commands {
                next_task: None,
                task_show: "task-show".to_string(),
                task_status: "printf 'in_progress\\n'".to_string(),
                task_update_in_progress: "task-update".to_string(),
                reset_task: "exit 1".to_string(),
            },
            hooks: Hooks {
                on_completed: "true".to_string(),
                on_requires_human: "true".to_string(),
                on_doctor_setup: None,
            },
            review_loop_limit: 2,
            log_path: "".to_string(),
        },
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

    reset_task_on_exit(
        &state,
        &Err(Quit {
            code: 1,
            reason: "error".to_string(),
        }),
    );
}

#[test]
fn reset_task_on_exit_is_noop_for_blank_task_id() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let state = RuntimeState {
        config: Config {
            agent_command: "agent".to_string(),
            agent_review_command: "review".to_string(),
            commands: Commands {
                next_task: None,
                task_show: "task-show".to_string(),
                task_status: "printf 'in_progress\\n'".to_string(),
                task_update_in_progress: "task-update".to_string(),
                reset_task: "reset-task".to_string(),
            },
            hooks: Hooks {
                on_completed: "true".to_string(),
                on_requires_human: "true".to_string(),
                on_doctor_setup: None,
            },
            review_loop_limit: 2,
            log_path: "".to_string(),
        },
        config_path: temp.path().join("trudger.yml"),
        prompt_trudge: "Task context".to_string(),
        prompt_review: "Review context".to_string(),
        logger: Logger::new(None),
        tmux: TmuxState::disabled(),
        interrupt_flag: Arc::new(AtomicBool::new(false)),
        manual_tasks: Vec::new(),
        completed_tasks: Vec::new(),
        needs_human_tasks: Vec::new(),
        current_task_id: Some("   \n\t".to_string()),
        current_task_show: None,
        current_task_status: None,
    };

    reset_task_on_exit(
        &state,
        &Err(Quit {
            code: 1,
            reason: "error".to_string(),
        }),
    );
}

#[test]
fn reset_task_on_exit_skips_reset_when_task_status_is_empty() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let reset_task_log = temp.path().join("reset-task.log");
    let state = RuntimeState {
        config: Config {
            agent_command: "agent".to_string(),
            agent_review_command: "review".to_string(),
            commands: Commands {
                next_task: None,
                task_show: "task-show".to_string(),
                task_status: "printf ''".to_string(),
                task_update_in_progress: "task-update".to_string(),
                reset_task: format!("printf reset > {}", reset_task_log.display()),
            },
            hooks: Hooks {
                on_completed: "true".to_string(),
                on_requires_human: "true".to_string(),
                on_doctor_setup: None,
            },
            review_loop_limit: 2,
            log_path: "".to_string(),
        },
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

    reset_task_on_exit(
        &state,
        &Err(Quit {
            code: 1,
            reason: "error".to_string(),
        }),
    );

    assert!(
        !reset_task_log.exists(),
        "reset-task should not run for empty status"
    );
}

#[test]
fn reset_task_on_exit_skips_reset_when_task_status_command_fails_to_spawn() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let old_path = env::var_os("PATH");
    env::set_var("PATH", "");

    let temp = TempDir::new().expect("temp dir");
    let state = RuntimeState {
        config: Config {
            agent_command: "agent".to_string(),
            agent_review_command: "review".to_string(),
            commands: Commands {
                next_task: None,
                task_show: "task-show".to_string(),
                task_status: "task-status".to_string(),
                task_update_in_progress: "task-update".to_string(),
                reset_task: "reset-task".to_string(),
            },
            hooks: Hooks {
                on_completed: "true".to_string(),
                on_requires_human: "true".to_string(),
                on_doctor_setup: None,
            },
            review_loop_limit: 2,
            log_path: "".to_string(),
        },
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

    reset_task_on_exit(
        &state,
        &Err(Quit {
            code: 1,
            reason: "error".to_string(),
        }),
    );

    match old_path {
        Some(value) => env::set_var("PATH", value),
        None => env::remove_var("PATH"),
    }
}

#[test]
fn hook_failure_after_closed_does_not_reset_task_on_exit() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let task_status_log = temp.path().join("task-status.log");
    let reset_task_log = temp.path().join("reset-task.log");
    let status_queue = temp.path().join("status-queue.txt");
    fs::write(&status_queue, "closed\n").expect("write status queue");

    let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bin");
    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));
    env::set_var("TASK_STATUS_LOG", &task_status_log);
    env::set_var("TASK_STATUS_QUEUE", &status_queue);
    env::set_var("RESET_TASK_LOG", &reset_task_log);

    let state = RuntimeState {
        config: Config {
            agent_command: "agent".to_string(),
            agent_review_command: "review".to_string(),
            commands: Commands {
                next_task: None,
                task_show: "task-show".to_string(),
                task_status: "task-status".to_string(),
                task_update_in_progress: "task-update".to_string(),
                reset_task: "reset-task".to_string(),
            },
            hooks: Hooks {
                on_completed: "true".to_string(),
                on_requires_human: "true".to_string(),
                on_doctor_setup: None,
            },
            review_loop_limit: 1,
            log_path: "".to_string(),
        },
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

    reset_task_on_exit(
        &state,
        &Err(Quit {
            code: 1,
            reason: "error:hook_failed".to_string(),
        }),
    );

    assert!(task_status_log.exists(), "task-status should run at exit");
    assert!(
        !reset_task_log.exists(),
        "reset-task should not run when task is closed"
    );
}

#[test]
fn hook_failure_after_blocked_does_not_reset_task_on_exit() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let task_status_log = temp.path().join("task-status.log");
    let reset_task_log = temp.path().join("reset-task.log");
    let status_queue = temp.path().join("status-queue.txt");
    fs::write(&status_queue, "blocked\n").expect("write status queue");

    let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bin");
    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));
    env::set_var("TASK_STATUS_LOG", &task_status_log);
    env::set_var("TASK_STATUS_QUEUE", &status_queue);
    env::set_var("RESET_TASK_LOG", &reset_task_log);

    let state = RuntimeState {
        config: Config {
            agent_command: "agent".to_string(),
            agent_review_command: "review".to_string(),
            commands: Commands {
                next_task: None,
                task_show: "task-show".to_string(),
                task_status: "task-status".to_string(),
                task_update_in_progress: "task-update".to_string(),
                reset_task: "reset-task".to_string(),
            },
            hooks: Hooks {
                on_completed: "true".to_string(),
                on_requires_human: "true".to_string(),
                on_doctor_setup: None,
            },
            review_loop_limit: 1,
            log_path: "".to_string(),
        },
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

    reset_task_on_exit(
        &state,
        &Err(Quit {
            code: 1,
            reason: "error:hook_failed".to_string(),
        }),
    );

    assert!(task_status_log.exists(), "task-status should run at exit");
    assert!(
        !reset_task_log.exists(),
        "reset-task should not run when task is blocked"
    );
}

#[test]
fn solve_failure_while_in_progress_invokes_reset_task_on_exit() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let task_status_log = temp.path().join("task-status.log");
    let reset_task_log = temp.path().join("reset-task.log");
    let status_queue = temp.path().join("status-queue.txt");
    fs::write(&status_queue, "in_progress\n").expect("write status queue");

    let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bin");
    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));
    env::set_var("TASK_STATUS_LOG", &task_status_log);
    env::set_var("TASK_STATUS_QUEUE", &status_queue);
    env::set_var("RESET_TASK_LOG", &reset_task_log);

    let state = RuntimeState {
        config: Config {
            agent_command: "agent".to_string(),
            agent_review_command: "review".to_string(),
            commands: Commands {
                next_task: None,
                task_show: "task-show".to_string(),
                task_status: "task-status".to_string(),
                task_update_in_progress: "task-update".to_string(),
                reset_task: "reset-task".to_string(),
            },
            hooks: Hooks {
                on_completed: "true".to_string(),
                on_requires_human: "true".to_string(),
                on_doctor_setup: None,
            },
            review_loop_limit: 1,
            log_path: "".to_string(),
        },
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

    reset_task_on_exit(
        &state,
        &Err(Quit {
            code: 1,
            reason: "solve_failed:tr-1".to_string(),
        }),
    );

    assert!(task_status_log.exists(), "task-status should run at exit");
    assert!(
        reset_task_log.exists(),
        "reset-task should run for in_progress"
    );
}

#[test]
fn sigint_while_in_progress_invokes_reset_task_on_exit() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let task_status_log = temp.path().join("task-status.log");
    let reset_task_log = temp.path().join("reset-task.log");
    let status_queue = temp.path().join("status-queue.txt");
    fs::write(&status_queue, "in_progress\n").expect("write status queue");

    let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bin");
    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));
    env::set_var("TASK_STATUS_LOG", &task_status_log);
    env::set_var("TASK_STATUS_QUEUE", &status_queue);
    env::set_var("RESET_TASK_LOG", &reset_task_log);

    let state = RuntimeState {
        config: Config {
            agent_command: "agent".to_string(),
            agent_review_command: "review".to_string(),
            commands: Commands {
                next_task: None,
                task_show: "task-show".to_string(),
                task_status: "task-status".to_string(),
                task_update_in_progress: "task-update".to_string(),
                reset_task: "reset-task".to_string(),
            },
            hooks: Hooks {
                on_completed: "true".to_string(),
                on_requires_human: "true".to_string(),
                on_doctor_setup: None,
            },
            review_loop_limit: 1,
            log_path: "".to_string(),
        },
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

    reset_task_on_exit(
        &state,
        &Err(Quit {
            code: 130,
            reason: "interrupted".to_string(),
        }),
    );

    assert!(task_status_log.exists(), "task-status should run at exit");
    assert!(
        reset_task_log.exists(),
        "reset-task should run for in_progress"
    );
}

#[test]
fn status_check_failure_at_exit_does_not_invoke_reset_task() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let task_status_log = temp.path().join("task-status.log");
    let reset_task_log = temp.path().join("reset-task.log");
    let status_queue = temp.path().join("status-queue.txt");
    fs::write(&status_queue, "in_progress\n").expect("write status queue");

    let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bin");
    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));
    env::set_var("TASK_STATUS_EXIT_CODE", "2");
    env::set_var("TASK_STATUS_LOG", &task_status_log);
    env::set_var("TASK_STATUS_QUEUE", &status_queue);
    env::set_var("RESET_TASK_LOG", &reset_task_log);

    let state = RuntimeState {
        config: Config {
            agent_command: "agent".to_string(),
            agent_review_command: "review".to_string(),
            commands: Commands {
                next_task: None,
                task_show: "task-show".to_string(),
                task_status: "task-status".to_string(),
                task_update_in_progress: "task-update".to_string(),
                reset_task: "reset-task".to_string(),
            },
            hooks: Hooks {
                on_completed: "true".to_string(),
                on_requires_human: "true".to_string(),
                on_doctor_setup: None,
            },
            review_loop_limit: 1,
            log_path: "".to_string(),
        },
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

    reset_task_on_exit(
        &state,
        &Err(Quit {
            code: 1,
            reason: "error".to_string(),
        }),
    );

    assert!(task_status_log.exists(), "task-status should run at exit");
    assert!(
        !reset_task_log.exists(),
        "reset-task should not run when status check fails"
    );
}

#[test]
fn run_with_cli_bootstraps_when_default_config_missing() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    env::remove_var("TMUX");
    let old_home = env::var_os("HOME");
    let temp = TempDir::new().expect("temp dir");
    env::set_var("HOME", temp.path());

    let err = run_with_cli(Cli {
        config: None,
        task: Vec::new(),
        positional: Vec::new(),
        command: None,
    })
    .expect_err("expected bootstrap missing config error");
    assert_eq!(err.code, 1);

    match old_home {
        Some(value) => env::set_var("HOME", value),
        None => env::remove_var("HOME"),
    };
}

#[test]
fn run_with_cli_rejects_invalid_manual_task_values() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let err = run_with_cli(Cli {
        config: None,
        task: vec!["tr-1,,tr-2".to_string()],
        positional: Vec::new(),
        command: None,
    })
    .expect_err("expected manual task parse error");
    assert_eq!(err.code, 1);
}

#[test]
fn run_with_cli_rejects_wizard_positional_args() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let err = run_with_cli(Cli {
        config: None,
        task: Vec::new(),
        positional: vec!["extra".to_string()],
        command: Some(CliCommand::Wizard),
    })
    .expect_err("expected positional args error");
    assert_eq!(err.code, 1);
    assert!(err.reason.contains("wizard mode"));
}

#[test]
fn run_with_cli_rejects_wizard_task_flag() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let err = run_with_cli(Cli {
        config: None,
        task: vec!["tr-1".to_string()],
        positional: Vec::new(),
        command: Some(CliCommand::Wizard),
    })
    .expect_err("expected wizard -t error");
    assert_eq!(err.code, 1);
    assert!(err.reason.contains("wizard mode"));
}

#[test]
fn run_with_cli_invokes_wizard_runner_without_tty() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    env::remove_var("TMUX");
    let old_home = env::var_os("HOME");
    let temp = TempDir::new().expect("temp dir");
    env::set_var("HOME", temp.path());

    let err = run_with_cli(Cli {
        config: None,
        task: Vec::new(),
        positional: Vec::new(),
        command: Some(CliCommand::Wizard),
    })
    .expect_err("expected wizard TTY error");
    assert_eq!(err.code, 1);
    assert!(err.reason.contains("requires an interactive terminal"));

    match old_home {
        Some(value) => env::set_var("HOME", value),
        None => env::remove_var("HOME"),
    };
}

#[test]
fn run_with_cli_errors_when_home_missing() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let old_home = env::var_os("HOME");
    env::remove_var("HOME");

    let err = run_with_cli(Cli {
        config: None,
        task: Vec::new(),
        positional: Vec::new(),
        command: None,
    })
    .expect_err("expected missing HOME error");
    assert_eq!(err.code, 1);

    match old_home {
        Some(value) => env::set_var("HOME", value),
        None => env::remove_var("HOME"),
    };
}

#[test]
fn run_with_cli_errors_when_explicit_config_is_missing() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    env::remove_var("TMUX");
    let old_home = env::var_os("HOME");
    let temp = TempDir::new().expect("temp dir");
    env::set_var("HOME", temp.path());

    let missing = temp.path().join("missing.yml");
    let err = run_with_cli(Cli {
        config: Some(missing.clone()),
        task: Vec::new(),
        positional: Vec::new(),
        command: None,
    })
    .expect_err("expected missing config error");
    assert_eq!(err.code, 1);
    assert!(err.reason.contains(&missing.display().to_string()));

    match old_home {
        Some(value) => env::set_var("HOME", value),
        None => env::remove_var("HOME"),
    };
}

#[test]
fn run_with_cli_errors_when_config_is_invalid_yaml() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    env::remove_var("TMUX");
    let old_home = env::var_os("HOME");
    let temp = TempDir::new().expect("temp dir");
    env::set_var("HOME", temp.path());

    let config_path = temp.path().join("trudger.yml");
    fs::write(&config_path, "agent_command: [").expect("write config");

    let err = run_with_cli(Cli {
        config: Some(config_path),
        task: Vec::new(),
        positional: Vec::new(),
        command: None,
    })
    .expect_err("expected invalid config error");
    assert_eq!(err.code, 1);

    match old_home {
        Some(value) => env::set_var("HOME", value),
        None => env::remove_var("HOME"),
    };
}

#[test]
fn run_with_cli_errors_when_validate_config_fails_and_log_path_is_empty() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    env::remove_var("TMUX");
    let old_home = env::var_os("HOME");
    let temp = TempDir::new().expect("temp dir");
    env::set_var("HOME", temp.path());

    let config_path = temp.path().join("trudger.yml");
    fs::write(
        &config_path,
        r#"
agent_command: "agent"
agent_review_command: "agent-review"
commands:
  task_show: "task-show"
  task_status: "task-status"
  task_update_in_progress: "task-update"
  reset_task: "reset-task"
review_loop_limit: 2
hooks:
  on_completed: "true"
  on_requires_human: "true"
"#,
    )
    .expect("write config");

    let err = run_with_cli(Cli {
        config: Some(config_path),
        task: Vec::new(),
        positional: Vec::new(),
        command: None,
    })
    .expect_err("expected validate_config failure");
    assert_eq!(err.code, 1);

    match old_home {
        Some(value) => env::set_var("HOME", value),
        None => env::remove_var("HOME"),
    };
}

#[test]
fn run_with_cli_errors_when_prompt_files_missing() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    env::remove_var("TMUX");
    let old_home = env::var_os("HOME");
    let temp = TempDir::new().expect("temp dir");
    env::set_var("HOME", temp.path());

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
  task_update_in_progress: "task-update"
  reset_task: "reset-task"
review_loop_limit: 2
hooks:
  on_completed: "true"
  on_requires_human: "true"
"#,
    )
    .expect("write config");

    let err = run_with_cli(Cli {
        config: Some(config_path.clone()),
        task: Vec::new(),
        positional: Vec::new(),
        command: None,
    })
    .expect_err("expected prompt missing error");
    assert_eq!(err.code, 1);

    let prompts_dir = temp.path().join(".codex").join("prompts");
    fs::create_dir_all(&prompts_dir).expect("create prompts dir");
    fs::write(prompts_dir.join("trudge.md"), "hello").expect("write trudge.md");

    let err = run_with_cli(Cli {
        config: Some(config_path),
        task: Vec::new(),
        positional: Vec::new(),
        command: None,
    })
    .expect_err("expected review prompt missing error");
    assert_eq!(err.code, 1);

    match old_home {
        Some(value) => env::set_var("HOME", value),
        None => env::remove_var("HOME"),
    };
}

#[cfg(unix)]
#[test]
fn run_with_cli_errors_when_trudge_prompt_is_unreadable() {
    use std::os::unix::fs::PermissionsExt;

    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    env::remove_var("TMUX");
    let old_home = env::var_os("HOME");
    let temp = TempDir::new().expect("temp dir");
    env::set_var("HOME", temp.path());

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
  task_update_in_progress: "task-update"
  reset_task: "reset-task"
review_loop_limit: 2
hooks:
  on_completed: "true"
  on_requires_human: "true"
"#,
    )
    .expect("write config");

    let prompts_dir = temp.path().join(".codex").join("prompts");
    fs::create_dir_all(&prompts_dir).expect("create prompts dir");
    let trudge_prompt = prompts_dir.join("trudge.md");
    let review_prompt = prompts_dir.join("trudge_review.md");
    fs::write(&trudge_prompt, "hello").expect("write trudge.md");
    fs::write(&review_prompt, "review").expect("write trudge_review.md");

    fs::set_permissions(&trudge_prompt, fs::Permissions::from_mode(0o000))
        .expect("chmod trudge.md");

    let err = run_with_cli(Cli {
        config: Some(config_path),
        task: Vec::new(),
        positional: Vec::new(),
        command: None,
    })
    .expect_err("expected trudge prompt read error");
    assert_eq!(err.code, 1);
    assert!(
        err.reason.contains("Failed to read prompt"),
        "expected prompt read error, got: {}",
        err.reason
    );

    match old_home {
        Some(value) => env::set_var("HOME", value),
        None => env::remove_var("HOME"),
    };
}

#[cfg(unix)]
#[test]
fn run_with_cli_errors_when_review_prompt_is_unreadable() {
    use std::os::unix::fs::PermissionsExt;

    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    env::remove_var("TMUX");
    let old_home = env::var_os("HOME");
    let temp = TempDir::new().expect("temp dir");
    env::set_var("HOME", temp.path());

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
  task_update_in_progress: "task-update"
  reset_task: "reset-task"
review_loop_limit: 2
hooks:
  on_completed: "true"
  on_requires_human: "true"
"#,
    )
    .expect("write config");

    let prompts_dir = temp.path().join(".codex").join("prompts");
    fs::create_dir_all(&prompts_dir).expect("create prompts dir");
    let trudge_prompt = prompts_dir.join("trudge.md");
    let review_prompt = prompts_dir.join("trudge_review.md");
    fs::write(&trudge_prompt, "hello").expect("write trudge.md");
    fs::write(&review_prompt, "review").expect("write trudge_review.md");

    fs::set_permissions(&review_prompt, fs::Permissions::from_mode(0o000))
        .expect("chmod trudge_review.md");

    let err = run_with_cli(Cli {
        config: Some(config_path),
        task: Vec::new(),
        positional: Vec::new(),
        command: None,
    })
    .expect_err("expected review prompt read error");
    assert_eq!(err.code, 1);
    assert!(
        err.reason.contains("Failed to read prompt"),
        "expected prompt read error, got: {}",
        err.reason
    );

    match old_home {
        Some(value) => env::set_var("HOME", value),
        None => env::remove_var("HOME"),
    };
}

#[test]
fn run_with_cli_can_force_error_and_ctrlc_handler_error_is_non_fatal() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    env::remove_var("TMUX");
    env::set_var("TRUDGER_TEST_FORCE_ERR", "1");

    let old_home = env::var_os("HOME");
    let temp = TempDir::new().expect("temp dir");
    env::set_var("HOME", temp.path());

    let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bin");
    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));

    let config_path = temp.path().join("trudger.yml");
    fs::write(
        &config_path,
        r#"
agent_command: "true"
agent_review_command: "true"
commands:
  next_task: "next-task"
  task_show: "task-show"
  task_status: "task-status"
  task_update_in_progress: "true"
  reset_task: "true"
review_loop_limit: 2
hooks:
  on_completed: "true"
  on_requires_human: "true"
log_path: ""
"#,
    )
    .expect("write config");

    let prompts_dir = temp.path().join(".codex").join("prompts");
    fs::create_dir_all(&prompts_dir).expect("create prompts dir");
    fs::write(prompts_dir.join("trudge.md"), "hello").expect("write trudge.md");
    fs::write(prompts_dir.join("trudge_review.md"), "review").expect("write trudge_review.md");

    let cli = Cli {
        config: Some(config_path.clone()),
        task: Vec::new(),
        positional: Vec::new(),
        command: None,
    };

    let err = run_with_cli(cli).expect_err("expected forced error");
    assert_eq!(err.code, 1);

    let err = run_with_cli(Cli {
        config: Some(config_path),
        task: Vec::new(),
        positional: Vec::new(),
        command: None,
    })
    .expect_err("expected forced error");
    assert_eq!(err.code, 1);

    env::remove_var("TRUDGER_TEST_FORCE_ERR");
    match old_home {
        Some(value) => env::set_var("HOME", value),
        None => env::remove_var("HOME"),
    };
}

#[cfg(unix)]
#[test]
fn ctrlc_handler_runs_on_sigint() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    env::remove_var("TMUX");
    env::set_var("TRUDGER_TEST_FORCE_ERR", "1");

    let old_home = env::var_os("HOME");
    let temp = TempDir::new().expect("temp dir");
    env::set_var("HOME", temp.path());

    let config_path = temp.path().join("trudger.yml");
    fs::write(
        &config_path,
        r#"
agent_command: "true"
agent_review_command: "true"
commands:
  next_task: "next-task"
  task_show: "task-show"
  task_status: "task-status"
  task_update_in_progress: "true"
  reset_task: "true"
review_loop_limit: 2
hooks:
  on_completed: "true"
  on_requires_human: "true"
log_path: ""
"#,
    )
    .expect("write config");

    let prompts_dir = temp.path().join(".codex").join("prompts");
    fs::create_dir_all(&prompts_dir).expect("create prompts dir");
    fs::write(prompts_dir.join("trudge.md"), "hello").expect("write trudge.md");
    fs::write(prompts_dir.join("trudge_review.md"), "review").expect("write trudge_review.md");

    let _ = run_with_cli(Cli {
        config: Some(config_path),
        task: Vec::new(),
        positional: Vec::new(),
        command: None,
    });

    let pid = std::process::id().to_string();
    let _ = std::process::Command::new("kill")
        .args(["-s", "INT", &pid])
        .status()
        .expect("kill");
    std::thread::sleep(std::time::Duration::from_millis(50));

    env::remove_var("TRUDGER_TEST_FORCE_ERR");
    match old_home {
        Some(value) => env::set_var("HOME", value),
        None => env::remove_var("HOME"),
    };
}

#[test]
fn run_with_cli_runs_run_loop_and_restores_tmux() {
    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    env::remove_var("TMUX");
    let old_home = env::var_os("HOME");
    let temp = TempDir::new().expect("temp dir");
    env::set_var("HOME", temp.path());

    let fixtures_bin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bin");
    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", fixtures_bin.display(), old_path));

    env::set_var("NEXT_TASK_EXIT_CODE", "1");

    let config_path = temp.path().join("trudger.yml");
    fs::write(
        &config_path,
        r#"
agent_command: "true"
agent_review_command: "true"
commands:
  next_task: "next-task"
  task_show: "task-show"
  task_status: "task-status"
  task_update_in_progress: "true"
  reset_task: "true"
review_loop_limit: 2
hooks:
  on_completed: "true"
  on_requires_human: "true"
log_path: ""
"#,
    )
    .expect("write config");

    let prompts_dir = temp.path().join(".codex").join("prompts");
    fs::create_dir_all(&prompts_dir).expect("create prompts dir");
    fs::write(prompts_dir.join("trudge.md"), "hello").expect("write trudge.md");
    fs::write(prompts_dir.join("trudge_review.md"), "review").expect("write trudge_review.md");

    let err = run_with_cli(Cli {
        config: Some(config_path),
        task: Vec::new(),
        positional: Vec::new(),
        command: None,
    })
    .expect_err("expected idle exit");
    assert_eq!(err.code, 0);

    match old_home {
        Some(value) => env::set_var("HOME", value),
        None => env::remove_var("HOME"),
    };
}

#[cfg(unix)]
#[test]
fn tmux_state_enabled_reads_title_and_updates_pane_name() {
    use std::os::unix::fs::PermissionsExt;

    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let bin = temp.path().join("bin");
    fs::create_dir_all(&bin).expect("create bin dir");
    let tmux_log = temp.path().join("tmux.log");

    let tmux_script = bin.join("tmux");
    fs::write(
        &tmux_script,
        format!(
            "#!/usr/bin/env bash\nset -euo pipefail\nlog=\"{}\"\nif [[ \"${{1-}}\" == \"display-message\" ]]; then\n  fmt=\"${{3-}}\"\n  if [[ \"$fmt\" == \"#S\" ]]; then printf '%s\\n' 'session-1'; exit 0; fi\n  if [[ \"$fmt\" == \"#{{pane_title}}\" ]]; then printf '%s\\n' 'base COMPLETED [tr-1] SOLVING tr-2'; exit 0; fi\n  exit 1\nfi\nif [[ \"${{1-}}\" == \"select-pane\" ]]; then\n  printf 'select %s\\n' \"$*\" >> \"$log\"\n  exit 0\nfi\nexit 1\n",
            tmux_log.display()
        ),
    )
    .expect("write tmux script");
    fs::set_permissions(&tmux_script, fs::Permissions::from_mode(0o755)).expect("chmod tmux");

    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", bin.display(), old_path));
    env::set_var("TMUX", "1");
    env::remove_var("TRUDGER_TMUX_SESSION_NAME");
    env::remove_var("TRUDGER_TMUX_ORIGINAL_PANE_TITLE");

    let state = TmuxState::new();
    state.update_name("SOLVING", "tr-9", &["tr-1".to_string()], &[]);
    state.restore();

    let log_contents = fs::read_to_string(&tmux_log).unwrap_or_default();
    assert!(log_contents.contains("-T base"));
    assert!(log_contents.contains("SOLVING tr-9"));
    assert!(log_contents.contains("SOLVING tr-2"));
}

#[cfg(unix)]
#[test]
fn tmux_state_ignores_tmux_display_failures() {
    use std::os::unix::fs::PermissionsExt;

    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let bin = temp.path().join("bin");
    fs::create_dir_all(&bin).expect("create bin dir");
    let tmux_log = temp.path().join("tmux.log");

    let tmux_script = bin.join("tmux");
    fs::write(
        &tmux_script,
        format!(
            "#!/usr/bin/env sh\nset -eu\nlog=\"{}\"\nif [ \"${{1-}}\" = \"display-message\" ]; then exit 1; fi\nif [ \"${{1-}}\" = \"select-pane\" ]; then\n  printf 'select %s\\n' \"$*\" >> \"$log\"\n  exit 0\nfi\nexit 1\n",
            tmux_log.display()
        ),
    )
    .expect("write tmux script");
    fs::set_permissions(&tmux_script, fs::Permissions::from_mode(0o755)).expect("chmod tmux");

    let hostname_script = bin.join("hostname");
    fs::write(
        &hostname_script,
        "#!/usr/bin/env sh\nset -eu\nprintf '%s\\n' 'myhost'\nexit 0\n",
    )
    .expect("write hostname script");
    fs::set_permissions(&hostname_script, fs::Permissions::from_mode(0o755))
        .expect("chmod hostname");

    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", bin.display(), old_path));
    env::set_var("TMUX", "1");
    env::remove_var("TRUDGER_TMUX_SESSION_NAME");
    env::remove_var("TRUDGER_TMUX_ORIGINAL_PANE_TITLE");

    let _state = TmuxState::new();

    let log_contents = fs::read_to_string(&tmux_log).unwrap_or_default();
    assert!(log_contents.contains("myhost"));
}

#[cfg(unix)]
#[test]
fn tmux_state_default_base_name_uses_hostname_fallbacks() {
    use std::os::unix::fs::PermissionsExt;

    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let bin = temp.path().join("bin");
    fs::create_dir_all(&bin).expect("create bin dir");
    let tmux_log = temp.path().join("tmux.log");

    let tmux_script = bin.join("tmux");
    fs::write(
        &tmux_script,
        format!(
            "#!/usr/bin/env bash\nset -euo pipefail\nlog=\"{}\"\nif [[ \"${{1-}}\" == \"display-message\" ]]; then\n  fmt=\"${{3-}}\"\n  if [[ \"$fmt\" == \"#S\" ]]; then printf '%s\\n' 'session-1'; exit 0; fi\n  if [[ \"$fmt\" == \"#{{pane_title}}\" ]]; then printf '%s\\n' ''; exit 0; fi\n  exit 1\nfi\nif [[ \"${{1-}}\" == \"select-pane\" ]]; then\n  printf 'select %s\\n' \"$*\" >> \"$log\"\n  exit 0\nfi\nexit 1\n",
            tmux_log.display()
        ),
    )
    .expect("write tmux script");
    fs::set_permissions(&tmux_script, fs::Permissions::from_mode(0o755)).expect("chmod tmux");

    let hostname_script = bin.join("hostname");
    fs::write(
        &hostname_script,
        "#!/usr/bin/env bash\nset -euo pipefail\nif [[ \"${1-}\" == \"-s\" ]]; then\n  printf '%s\\n' ''\n  exit 0\nfi\nprintf '%s\\n' 'myhost'\nexit 0\n",
    )
    .expect("write hostname script");
    fs::set_permissions(&hostname_script, fs::Permissions::from_mode(0o755))
        .expect("chmod hostname");

    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", bin.display(), old_path));
    env::set_var("TMUX", "1");
    env::remove_var("TRUDGER_TMUX_SESSION_NAME");
    env::remove_var("TRUDGER_TMUX_ORIGINAL_PANE_TITLE");

    let state = TmuxState::new();
    state.restore();

    let log_contents = fs::read_to_string(&tmux_log).unwrap_or_default();
    assert!(log_contents.contains("myhost"));
}

#[cfg(unix)]
#[test]
fn tmux_state_hostname_falls_back_to_default_on_failures() {
    use std::os::unix::fs::PermissionsExt;

    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let bin = temp.path().join("bin");
    fs::create_dir_all(&bin).expect("create bin dir");
    let tmux_log = temp.path().join("tmux.log");

    let tmux_script = bin.join("tmux");
    fs::write(
        &tmux_script,
        format!(
            "#!/usr/bin/env bash\nset -euo pipefail\nlog=\"{}\"\nif [[ \"${{1-}}\" == \"display-message\" ]]; then\n  fmt=\"${{3-}}\"\n  if [[ \"$fmt\" == \"#S\" ]]; then printf '%s\\n' 'session-1'; exit 0; fi\n  if [[ \"$fmt\" == \"#{{pane_title}}\" ]]; then printf '%s\\n' ''; exit 0; fi\n  exit 1\nfi\nif [[ \"${{1-}}\" == \"select-pane\" ]]; then\n  printf 'select %s\\n' \"$*\" >> \"$log\"\n  exit 0\nfi\nexit 1\n",
            tmux_log.display()
        ),
    )
    .expect("write tmux script");
    fs::set_permissions(&tmux_script, fs::Permissions::from_mode(0o755)).expect("chmod tmux");

    let hostname_script = bin.join("hostname");
    fs::write(
        &hostname_script,
        "#!/usr/bin/env bash\nset -euo pipefail\nexit 1\n",
    )
    .expect("write hostname script");
    fs::set_permissions(&hostname_script, fs::Permissions::from_mode(0o755))
        .expect("chmod hostname");

    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", bin.display(), old_path));
    env::set_var("TMUX", "1");
    env::remove_var("TRUDGER_TMUX_SESSION_NAME");
    env::remove_var("TRUDGER_TMUX_ORIGINAL_PANE_TITLE");

    let _state = TmuxState::new();

    let log_contents = fs::read_to_string(&tmux_log).unwrap_or_default();
    assert!(log_contents.contains("host"));
}

#[cfg(unix)]
#[test]
fn tmux_state_uses_env_session_name_and_title_when_set() {
    use std::os::unix::fs::PermissionsExt;

    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let bin = temp.path().join("bin");
    fs::create_dir_all(&bin).expect("create bin dir");
    let tmux_log = temp.path().join("tmux.log");

    let tmux_script = bin.join("tmux");
    fs::write(
        &tmux_script,
        format!(
            "#!/usr/bin/env sh\nset -eu\nlog=\"{}\"\nif [ \"${{1-}}\" = \"display-message\" ]; then\n  echo \"unexpected display-message\" >&2\n  exit 2\nfi\nif [ \"${{1-}}\" = \"select-pane\" ]; then\n  printf 'select %s\\n' \"$*\" >> \"$log\"\n  exit 0\nfi\nexit 1\n",
            tmux_log.display()
        ),
    )
    .expect("write tmux script");
    fs::set_permissions(&tmux_script, fs::Permissions::from_mode(0o755)).expect("chmod tmux");

    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", bin.display(), old_path));
    env::set_var("TMUX", "1");
    env::set_var("TRUDGER_TMUX_SESSION_NAME", "session-from-env");
    env::set_var("TRUDGER_TMUX_ORIGINAL_PANE_TITLE", "title-from-env");

    let state = TmuxState::new();
    state.restore();

    let log_contents = fs::read_to_string(&tmux_log).unwrap_or_default();
    assert!(
        log_contents.contains("select select-pane -T title-from-env"),
        "expected pane title select, got:\n{log_contents}"
    );
}

#[cfg(unix)]
#[test]
fn tmux_state_env_vars_whitespace_fall_back_to_tmux_display() {
    use std::os::unix::fs::PermissionsExt;

    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let bin = temp.path().join("bin");
    fs::create_dir_all(&bin).expect("create bin dir");
    let tmux_log = temp.path().join("tmux.log");

    let tmux_script = bin.join("tmux");
    fs::write(
        &tmux_script,
        format!(
            "#!/usr/bin/env sh\nset -eu\nlog=\"{}\"\nif [ \"${{1-}}\" = \"display-message\" ]; then\n  fmt=\"${{3-}}\"\n  if [ \"$fmt\" = \"#S\" ]; then printf '%s\\n' 'session-1'; exit 0; fi\n  if [ \"$fmt\" = \"#{{pane_title}}\" ]; then printf '%s\\n' 'title-from-tmux'; exit 0; fi\n  exit 1\nfi\nif [ \"${{1-}}\" = \"select-pane\" ]; then\n  printf 'select %s\\n' \"$*\" >> \"$log\"\n  exit 0\nfi\nexit 1\n",
            tmux_log.display()
        ),
    )
    .expect("write tmux script");
    fs::set_permissions(&tmux_script, fs::Permissions::from_mode(0o755)).expect("chmod tmux");

    let old_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", format!("{}:{}", bin.display(), old_path));
    env::set_var("TMUX", "1");
    env::set_var("TRUDGER_TMUX_SESSION_NAME", "   ");
    env::set_var("TRUDGER_TMUX_ORIGINAL_PANE_TITLE", "  ");

    let state = TmuxState::new();
    state.restore();

    assert_eq!(
        env::var("TRUDGER_TMUX_SESSION_NAME").unwrap_or_default(),
        "session-1"
    );
    assert_eq!(
        env::var("TRUDGER_TMUX_ORIGINAL_PANE_TITLE").unwrap_or_default(),
        "title-from-tmux"
    );

    let log_contents = fs::read_to_string(&tmux_log).unwrap_or_default();
    assert!(
        log_contents.contains("select select-pane -T title-from-tmux"),
        "expected pane title select, got:\n{log_contents}"
    );
}

#[cfg(unix)]
#[test]
fn tmux_state_tolerates_tmux_spawn_failures() {
    use std::os::unix::fs::PermissionsExt;

    let _guard = ENV_MUTEX.lock().unwrap();
    reset_test_env();

    let temp = TempDir::new().expect("temp dir");
    let bin = temp.path().join("bin");
    fs::create_dir_all(&bin).expect("create bin dir");

    let tmux_script = bin.join("tmux");
    fs::write(&tmux_script, "#!/does/not/exist\nexit 0\n").expect("write tmux script");
    fs::set_permissions(&tmux_script, fs::Permissions::from_mode(0o755)).expect("chmod tmux");

    env::set_var("PATH", bin.display().to_string());
    env::set_var("TMUX", "1");
    env::set_var("TRUDGER_TMUX_SESSION_NAME", " ");
    env::set_var("TRUDGER_TMUX_ORIGINAL_PANE_TITLE", " ");

    let state = TmuxState::new();
    state.update_name("SOLVING", "tr-1", &[], &[]);
    state.restore();
}
