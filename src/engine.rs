// Drives the official Prime95 engine (mprime) through its text menus.
// SPDX-License-Identifier: GPL-3.0-or-later
//
// mprime has no API, but its menus are stable and every question it asks has the form
// "Label (default): ". A run is a Script of answers keyed by label; anything not in the
// script is answered with Enter, which keeps mprime's own default.

use std::io::{self, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gtk::glib;

/// Official Linux build, linked from https://www.mersenne.org/download/
pub const ENGINE_URL: &str = "https://download.mersenne.ca/gimps/v30/30.19/p95v3019b20.linux64.tar.gz";

pub fn data_dir() -> PathBuf {
    glib::user_data_dir().join("prime95-gtk")
}

/// Where mprime runs: prime.txt, results.txt and friends live here.
pub fn work_dir() -> PathBuf {
    data_dir().join("work")
}

fn downloaded_engine_dir() -> PathBuf {
    data_dir().join("engine")
}

pub fn find_engine() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(path) = std::env::var_os("PRIME95_GTK_ENGINE") {
        candidates.push(path.into());
    }
    candidates.push(downloaded_engine_dir().join("mprime"));
    candidates.push(glib::home_dir().join(".local/opt/prime95/mprime"));
    candidates.push("/opt/mprime/mprime".into());
    candidates.into_iter().find(|p| p.is_file())
}

/// Downloads and unpacks the official engine. Blocking; run it off the UI thread.
pub fn download_engine() -> Result<PathBuf, String> {
    let dir = downloaded_engine_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let status = Command::new("sh")
        .args(["-c", "curl -fsSL \"$1\" | tar xz -C \"$2\"", "sh", ENGINE_URL])
        .arg(&dir)
        .status()
        .map_err(|e| e.to_string())?;
    let mprime = dir.join("mprime");
    if status.success() && mprime.is_file() {
        Ok(mprime)
    } else {
        Err(format!("Download failed. Check your internet connection.\n{ENGINE_URL}"))
    }
}

fn command(engine: &Path) -> io::Result<Command> {
    let dir = work_dir();
    std::fs::create_dir_all(&dir)?;
    let ini = dir.join("prime.txt");
    if !ini.exists() {
        // Stress-testing only: never ask for a PrimeNet account or go online.
        std::fs::write(&ini, "V24OptionsConverted=1\nWGUID_version=2\nStressTester=1\nUsePrimenet=0\n")?;
    }
    let mut cmd = Command::new(engine);
    cmd.arg("-m")
        .current_dir(dir)
        .env("LD_LIBRARY_PATH", engine.parent().unwrap_or(Path::new(".")))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // Never leave a torture test running if the window goes away.
    unsafe {
        cmd.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
    Ok(cmd)
}

#[derive(Clone)]
enum Answer {
    Text(String),
    /// Choose the numbered option (menu item or list entry) whose text starts with this.
    Pick(String),
}

#[derive(Clone, Default)]
pub struct Script {
    steps: Vec<(String, Answer)>,
}

impl Script {
    pub fn menu(item: &str) -> Self {
        Self { steps: vec![("Your choice".into(), Answer::Pick(item.into()))] }
    }

    pub fn text(mut self, prompt: &str, value: impl ToString) -> Self {
        self.steps.push((prompt.into(), Answer::Text(value.to_string())));
        self
    }

    pub fn pick(mut self, prompt: &str, option: &str) -> Self {
        self.steps.push((prompt.into(), Answer::Pick(option.into())));
        self
    }

    pub fn yes_no(self, prompt: &str, yes: bool) -> Self {
        self.text(prompt, if yes { "Y" } else { "N" })
    }

    /// The reply to a prompt; None means "nothing left to do at the main menu".
    fn reply(&mut self, prompt: &str, recent: &[String]) -> Option<String> {
        if let Some(i) = self.steps.iter().position(|(key, _)| prompt.contains(key.as_str())) {
            return Some(match self.steps.remove(i).1 {
                Answer::Text(text) => text,
                Answer::Pick(option) => pick(recent, &option).unwrap_or_default(),
            });
        }
        if prompt.contains("Your choice") { None } else { Some(String::new()) }
    }
}

/// Finds "16.  Options/Torture Test" or "2 = Small FFTs (...)" and returns its number.
fn pick(lines: &[String], option: &str) -> Option<String> {
    lines.iter().find_map(|line| {
        [". ", " = "].iter().find_map(|sep| {
            let (num, rest) = line.trim().split_once(sep)?;
            (num.parse::<u32>().is_ok() && rest.trim().starts_with(option)).then(|| num.to_string())
        })
    })
}

enum Event<'a> {
    Line(&'a str),
    Prompt(&'a str),
}

enum Flow {
    /// Answer from the script (or Enter for mprime's default).
    Default,
    Answer(String),
    /// Leave the prompt unanswered (mprime keeps working in the background).
    Wait,
    /// Close stdin; mprime treats that like Ctrl+C at the menu and exits cleanly.
    CloseInput,
    Kill,
}

fn drive(mut out: impl Read, mut stdin: Option<ChildStdin>, mut script: Script, mut handle: impl FnMut(Event) -> Flow) {
    let mut buf = [0u8; 8192];
    let mut partial = String::new();
    let mut recent: Vec<String> = Vec::new();
    while let Ok(n) = out.read(&mut buf) {
        if n == 0 {
            break;
        }
        partial.push_str(&String::from_utf8_lossy(&buf[..n]));
        while let Some(i) = partial.find('\n') {
            let line: String = partial.drain(..=i).collect();
            let line = line.trim_end();
            match handle(Event::Line(line)) {
                Flow::Kill => return,
                Flow::CloseInput => stdin = None,
                _ => {}
            }
            recent.push(line.to_string());
        }
        if !partial.ends_with(": ") {
            continue;
        }
        let prompt = std::mem::take(&mut partial);
        let prompt = prompt.trim();
        let reply = match handle(Event::Prompt(prompt)) {
            Flow::Kill => return,
            Flow::Wait => None,
            Flow::CloseInput => {
                stdin = None;
                None
            }
            Flow::Answer(answer) => Some(answer),
            Flow::Default => {
                let reply = script.reply(prompt, &recent);
                if reply.is_none() {
                    stdin = None;
                }
                reply
            }
        };
        recent.clear();
        if let (Some(reply), Some(input)) = (reply, stdin.as_mut())
            && writeln!(input, "{reply}").is_err()
        {
            stdin = None;
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Torture,
    Benchmark,
}

pub enum RunEvent {
    Line(String),
    Done,
}

pub struct Run {
    pid: i32,
}

impl Run {
    /// Asks mprime to stop its workers and exit, like Test > Stop.
    pub fn stop(&self) {
        unsafe {
            libc::kill(self.pid, libc::SIGTERM);
        }
    }
}

pub fn start(engine: &Path, script: Script, mode: Mode, tx: async_channel::Sender<RunEvent>) -> io::Result<Run> {
    let mut child = command(engine)?.spawn()?;
    let pid = child.id() as i32;
    let out = child.stdout.take().expect("piped stdout");
    let stdin = child.stdin.take();
    std::thread::spawn(move || {
        let mut accepted = false;
        drive(out, stdin, script, |event| match event {
            Event::Line(line) => {
                if line.starts_with('[') {
                    let _ = tx.send_blocking(RunEvent::Line(line.to_string()));
                }
                let finished = line.contains("benchmark complete") || line.contains("Worker stopped");
                if mode == Mode::Benchmark && accepted && finished { Flow::CloseInput } else { Flow::Default }
            }
            Event::Prompt(prompt) if prompt.contains("Accept the answers") => {
                accepted = true;
                Flow::Answer("Y".into())
            }
            // Torture tests block the menu until they end; benchmarks return to it at once.
            Event::Prompt(prompt) if accepted && prompt.contains("Your choice") => match mode {
                Mode::Torture => Flow::CloseInput,
                Mode::Benchmark => Flow::Wait,
            },
            Event::Prompt(_) => Flow::Default,
        });
        let _ = child.wait();
        let _ = tx.send_blocking(RunEvent::Done);
    });
    Ok(Run { pid })
}

/// What mprime asked (with its defaults) and printed while walking a menu, stopping
/// before anything is accepted. This is how the dialogs show the engine's real defaults.
#[derive(Default, Debug)]
pub struct Query {
    pub prompts: Vec<(String, Option<String>)>,
    pub lines: Vec<String>,
}

impl Query {
    pub fn asked(&self, key: &str) -> bool {
        self.prompts.iter().any(|(label, _)| label.contains(key))
    }

    pub fn default_of(&self, key: &str) -> Option<&str> {
        self.prompts.iter().find(|(label, _)| label.contains(key)).and_then(|(_, d)| d.as_deref())
    }

    pub fn number(&self, key: &str) -> Option<f64> {
        self.default_of(key)?.trim().parse().ok()
    }

    pub fn yes(&self, key: &str) -> Option<bool> {
        self.default_of(key).map(|d| d.eq_ignore_ascii_case("Y"))
    }
}

fn split_prompt(prompt: &str) -> (String, Option<String>) {
    let prompt = prompt.trim_end_matches(':').trim();
    match prompt.rsplit_once(" (") {
        Some((label, default)) if default.ends_with(')') => {
            (label.to_string(), Some(default.trim_end_matches(')').to_string()))
        }
        _ => (prompt.to_string(), None),
    }
}

pub fn query(engine: &Path, script: Script) -> io::Result<Query> {
    let mut child = command(engine)?.spawn()?;
    let pid = child.id() as i32;
    let finished = Arc::new(AtomicBool::new(false));
    {
        let finished = finished.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(10));
            if !finished.load(Ordering::SeqCst) {
                unsafe { libc::kill(pid, libc::SIGKILL) };
            }
        });
    }
    let mut result = Query::default();
    let out = child.stdout.take().expect("piped stdout");
    drive(out, child.stdin.take(), script, |event| match event {
        Event::Line(line) => {
            result.lines.push(line.to_string());
            Flow::Default
        }
        Event::Prompt(prompt) if prompt.contains("Accept the answers") => Flow::Kill,
        Event::Prompt(prompt) => {
            if !prompt.contains("Your choice") {
                result.prompts.push(split_prompt(prompt));
            }
            Flow::Default
        }
    });
    let _ = child.kill();
    let _ = child.wait();
    finished.store(true, Ordering::SeqCst);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_menu_items_and_list_entries() {
        let lines: Vec<String> = ["\t15.  Options/Preferences", "\t16.  Options/Torture Test", "  1 = Smallest FFTs (L1/L2).", "  2 = Small FFTs (L1/L2/L3).", "Blend is the default.  NOTE: x"]
            .map(String::from)
            .into();
        assert_eq!(pick(&lines, "Options/Torture Test").as_deref(), Some("16"));
        assert_eq!(pick(&lines, "Small FFTs").as_deref(), Some("2"));
        assert_eq!(pick(&lines, "Smallest FFTs").as_deref(), Some("1"));
        assert_eq!(pick(&lines, "Blend"), None);
    }

    #[test]
    fn splits_prompts() {
        assert_eq!(split_prompt("Min FFT size (in K) (41):"), ("Min FFT size (in K)".into(), Some("41".into())));
        assert_eq!(
            split_prompt("Benchmark type (0 = Throughput, 1 = FFT timings, 2 = Trial factoring) (0):"),
            ("Benchmark type (0 = Throughput, 1 = FFT timings, 2 = Trial factoring)".into(), Some("0".into()))
        );
        assert_eq!(split_prompt("Your choice:"), ("Your choice".into(), None));
    }

    #[test]
    fn script_matches_most_specific_prompt_in_order() {
        let mut s = Script::default().yes_no("Disable AVX-512", true).yes_no("Disable AVX2", true).yes_no("Disable AVX", false);
        assert_eq!(s.reply("Disable AVX2 (fused multiply add) (N):", &[]).as_deref(), Some("Y"));
        assert_eq!(s.reply("Disable AVX (N):", &[]).as_deref(), Some("N"));
        assert_eq!(s.reply("Something else (3):", &[]).as_deref(), Some(""));
        assert_eq!(s.reply("Your choice:", &[]), None);
    }
}

/// Checks against a real engine: `cargo test -- --ignored --nocapture`
#[cfg(test)]
mod engine_tests {
    use super::*;

    /// Uses a throwaway data folder so test settings never reach the real prime.txt.
    fn engine() -> PathBuf {
        let tmp = std::env::temp_dir().join("prime95-gtk-tests");
        unsafe { std::env::set_var("XDG_DATA_HOME", &tmp) };
        find_engine().expect("no mprime engine installed")
    }

    #[test]
    #[ignore]
    fn reads_torture_defaults() {
        let q = query(&engine(), Script::menu("Options/Torture Test").pick("Type of torture test", "Small FFTs").yes_no("Customize settings", true)).unwrap();
        println!("{:#?}", q.prompts);
        assert!(q.number("Number of cores to torture test").unwrap() >= 1.0);
        assert!(q.number("Min FFT size").is_some() && q.number("Max FFT size").is_some());
    }

    #[test]
    #[ignore]
    fn torture_runs_and_stops() {
        let script = Script::menu("Options/Torture Test")
            .text("Number of cores to torture test", 2)
            .pick("Type of torture test", "Smallest FFTs")
            .yes_no("Customize settings", false)
            .yes_no("Run a weaker torture test", false);
        let (tx, rx) = async_channel::unbounded();
        let run = start(&engine(), script, Mode::Torture, tx).unwrap();
        std::thread::sleep(Duration::from_secs(4));
        run.stop();
        let mut lines = Vec::new();
        while let Ok(event) = rx.recv_blocking() {
            match event {
                RunEvent::Line(l) => lines.push(l),
                RunEvent::Done => break,
            }
        }
        println!("{}", lines.join("\n"));
        assert!(lines.iter().any(|l| l.starts_with("[Worker #2")));
        assert!(lines.iter().any(|l| l.contains("Torture Test completed")));
    }

    #[test]
    #[ignore]
    fn benchmark_finishes_by_itself() {
        let script = Script::menu("Options/Benchmark")
            .text("Benchmark type", 1)
            .text("Minimum FFT size", 1024)
            .text("Maximum FFT size", 1024)
            .text("Number of CPU cores", 1)
            .yes_no("Benchmark hyperthreading", false);
        let (tx, rx) = async_channel::unbounded();
        let _run = start(&engine(), script, Mode::Benchmark, tx).unwrap();
        let mut lines = Vec::new();
        while let Ok(event) = rx.recv_blocking() {
            match event {
                RunEvent::Line(l) => lines.push(l),
                RunEvent::Done => break,
            }
        }
        println!("{}", lines.join("\n"));
        assert!(lines.iter().any(|l| l.contains("benchmark complete")));
    }
}
