//! The `on_change` hook: a command the Config names, run by `vellum` (never
//! the Daemon) after every successful present of a new wallpaper.
//!
//! The command is split into words once, shell-style, when the Config is
//! parsed. `{path}` and `{monitor}` are then substituted inside those words
//! and the result is executed directly, with no shell, so a file name can
//! never inject a command. The same values are also exported as
//! `VELLUM_PATH` and `VELLUM_MONITOR` for hooks that do want a shell.
//!
//! [`fire`] only queues a run: a worker thread runs queued hooks one at a
//! time, so presenting on the next Monitor never waits for a slow hook and
//! two hooks never race each other. [`finish`] waits for the queue to drain
//! and is called once, before `vellum` exits. A hook that fails to start or
//! exits non-zero is reported on stderr (the Daemon's log, for a renderer
//! it spawned) and changes nothing else.

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::mpsc::{Sender, channel};
use std::thread::JoinHandle;

/// A parsed `on_change` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hook {
    words: Vec<String>,
}

impl Hook {
    /// Splits `text` into words like a POSIX shell would (quotes and
    /// backslashes, no expansion). A leading `~/` on a word is expanded
    /// against `home`.
    pub fn parse(text: &str, home: Option<&Path>) -> Result<Hook, String> {
        let words = shlex::split(text).ok_or(
            "has an unclosed quote or a trailing backslash; quote it like a shell command",
        )?;
        if words.is_empty() {
            return Err("is empty; name a command, like \"matugen image {path}\"".into());
        }
        let words = words
            .into_iter()
            .map(|word| match (word.strip_prefix("~/"), home) {
                (Some(rest), Some(home)) => home.join(rest).to_string_lossy().into_owned(),
                _ if word == "~" => home.map_or(word, |h| h.to_string_lossy().into_owned()),
                _ => word,
            })
            .collect();
        Ok(Hook { words })
    }

    /// The program and its arguments for a wallpaper at `path` presented on
    /// `monitor`. Each value lands inside exactly the word it was written
    /// in, however many spaces, quotes or `$` it holds.
    pub fn argv(&self, path: &str, monitor: &str) -> Vec<String> {
        self.words
            .iter()
            .map(|word| substitute(word, path, monitor))
            .collect()
    }

    fn command(&self, path: &str, monitor: &str) -> Command {
        let argv = self.argv(path, monitor);
        let mut command = Command::new(&argv[0]);
        command
            .args(&argv[1..])
            .env("VELLUM_PATH", path)
            .env("VELLUM_MONITOR", monitor)
            .stdin(Stdio::null());
        command
    }
}

/// Replaces `{path}` and `{monitor}` in one left-to-right pass, so a value
/// that itself contains `{monitor}` is left alone.
fn substitute(word: &str, path: &str, monitor: &str) -> String {
    let mut out = String::with_capacity(word.len());
    let mut rest = word;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        if let Some(after) = tail.strip_prefix("{path}") {
            out.push_str(path);
            rest = after;
        } else if let Some(after) = tail.strip_prefix("{monitor}") {
            out.push_str(monitor);
            rest = after;
        } else {
            out.push('{');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out
}

/// The hook from the Config this process loaded. Outer `None`: no Config
/// loaded yet.
static HOOK: Mutex<Option<Option<Hook>>> = Mutex::new(None);

struct Job {
    hook: Hook,
    path: String,
    monitor: String,
}

static WORKER: Mutex<Option<(Sender<Job>, JoinHandle<()>)>> = Mutex::new(None);

/// Records the hook of the Config just loaded. `config::load` calls this,
/// so every command that loads its Config arms the hook.
pub fn remember(hook: Option<&Hook>) {
    *HOOK.lock().unwrap_or_else(|e| e.into_inner()) = Some(hook.cloned());
}

/// Queues the hook, if the Config has one, for a wallpaper at `path` that
/// was just presented on `monitor`.
pub fn fire(path: &str, monitor: &str) {
    let loaded = HOOK.lock().unwrap_or_else(|e| e.into_inner()).is_some();
    if !loaded {
        crate::config::load(); // calls remember()
    }
    let Some(hook) = HOOK
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .flatten()
    else {
        return;
    };
    let job = Job {
        hook,
        path: path.to_owned(),
        monitor: monitor.to_owned(),
    };
    let mut worker = WORKER.lock().unwrap_or_else(|e| e.into_inner());
    let (sender, _) = worker.get_or_insert_with(|| {
        let (sender, jobs) = channel::<Job>();
        let handle = std::thread::spawn(move || jobs.into_iter().for_each(|job| run(&job)));
        (sender, handle)
    });
    // The worker only stops when the sender is dropped, in finish().
    let _ = sender.send(job);
}

/// Waits for every queued hook to finish. Called once before exiting.
pub fn finish() {
    let worker = WORKER.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some((sender, handle)) = worker {
        drop(sender);
        let _ = handle.join();
    }
}

/// Takes a failure report; returns false if it could not.
type Reporter = Box<dyn Fn(String) -> bool + Send>;

/// Where failures go instead of stderr, and failures it could not take.
static REPORTER: Mutex<(Option<Reporter>, Vec<String>)> = Mutex::new((None, Vec::new()));

/// Sends hook failures to `reporter` instead of stderr: the TUI shows them
/// as toasts, since stderr would scribble over its screen. `None` puts
/// them back on stderr, first printing any the reporter refused (say,
/// because the TUI was already shutting down).
pub fn report_to(reporter: Option<Reporter>) {
    let mut slot = REPORTER.lock().unwrap_or_else(|e| e.into_inner());
    if reporter.is_none() {
        for why in slot.1.drain(..) {
            eprintln!("vellum: {why}");
        }
    }
    slot.0 = reporter;
}

fn report(why: String) {
    let mut slot = REPORTER.lock().unwrap_or_else(|e| e.into_inner());
    match &slot.0 {
        Some(reporter) => {
            if !reporter(why.clone()) {
                slot.1.push(why);
            }
        }
        None => eprintln!("vellum: {why}"),
    }
}

fn run(job: &Job) {
    let program = &job.hook.words[0];
    let monitor = &job.monitor;
    match job.hook.command(&job.path, monitor).status() {
        Ok(status) if status.success() => {}
        Ok(status) => {
            report(format!(
                "the on_change hook `{program}` failed on {monitor} ({status})"
            ));
        }
        Err(err) => {
            report(format!(
                "the on_change hook `{program}` could not run on {monitor}: {err}"
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hook(text: &str) -> Hook {
        Hook::parse(text, Some(Path::new("/home/me"))).unwrap()
    }

    #[test]
    fn placeholders_are_substituted_inside_their_word() {
        let argv = hook("matugen image {path} --name=out-{monitor}.json").argv("/w/a.png", "DP-1");
        assert_eq!(
            argv,
            ["matugen", "image", "/w/a.png", "--name=out-DP-1.json"]
        );
    }

    #[test]
    fn a_hostile_file_name_stays_one_argument() {
        let path = "/w/$(rm -rf ~); `x` 'q\" a.png";
        assert_eq!(
            hook("wal -i {path}").argv(path, "DP-1"),
            ["wal", "-i", path]
        );
        assert_eq!(
            hook("wal -i \"{path}\"").argv(path, "DP-1"),
            ["wal", "-i", path]
        );
        assert_eq!(
            hook("wal -i '{path}'").argv(path, "DP-1"),
            ["wal", "-i", path]
        );
    }

    #[test]
    fn values_are_not_substituted_twice() {
        let argv = hook("echo {path}").argv("/w/{monitor}.png", "DP-1");
        assert_eq!(argv, ["echo", "/w/{monitor}.png"]);
    }

    #[test]
    fn unknown_braces_are_kept() {
        let argv = hook("sh -c 'echo {x} {path}'").argv("/a", "M");
        assert_eq!(argv, ["sh", "-c", "echo {x} /a"]);
    }

    #[test]
    fn quoting_and_tilde_work_like_a_shell() {
        let argv = hook("~/bin/hook \"two words\" ~/x a~/b").argv("/a", "M");
        assert_eq!(
            argv,
            ["/home/me/bin/hook", "two words", "/home/me/x", "a~/b"]
        );
    }

    #[test]
    fn failures_go_to_the_reporter_and_refused_ones_wait_for_stderr() {
        let (tx, rx) = channel::<String>();
        report_to(Some(Box::new(move |why| tx.send(why).is_ok())));
        run(&Job {
            hook: hook("false"),
            path: "/w/a.png".into(),
            monitor: "DP-1".into(),
        });
        let why = rx.recv().unwrap();
        assert!(
            why.starts_with("the on_change hook `false` failed on DP-1"),
            "{why}"
        );
        drop(rx);
        report("refused".into());
        assert_eq!(REPORTER.lock().unwrap().1, ["refused"]);
        report_to(None);
        assert!(REPORTER.lock().unwrap().1.is_empty());
    }

    #[test]
    fn broken_commands_are_rejected() {
        assert!(
            Hook::parse("matugen 'image", None)
                .unwrap_err()
                .contains("unclosed quote")
        );
        assert!(Hook::parse("   ", None).unwrap_err().contains("empty"));
    }
}
