// Prime95 GTK: a native Linux interface for the Prime95 (mprime) CPU stress test.
// SPDX-License-Identifier: GPL-3.0-or-later

mod dialogs;
mod engine;
mod monitor;

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};

use engine::{Mode, RunEvent, Script};

const APP_ID: &str = "io.github.omaralzand.Prime95Gtk";
const APP_NAME: &str = "Prime95 GTK";

const CSS: &str = "
.pane-title { font-weight: bold; }
.dot { font-size: 1.1em; }
.dot.idle { color: alpha(currentColor, 0.35); }
.dot.running { color: #2ec27e; }
.dot.failed { color: #e01b24; }
.status-bar { padding: 4px 10px; }
.status-bar.failed { color: #e01b24; font-weight: bold; }
";

/// Error messages the engine prints when a calculation goes wrong.
const FAILURE_MARKERS: &[&str] = &[
    "FATAL ERROR",
    "Hardware failure",
    "Possible hardware",
    "ROUND OFF",
    "SUM(INPUTS) != SUM(OUTPUTS)",
    "ERROR:",
];

/// One child window: "Main thread" or "Worker #n".
struct Pane {
    frame: gtk::Frame,
    dot: gtk::Label,
    view: gtk::TextView,
    end: gtk::TextMark,
}

impl Pane {
    fn new(title: &str) -> Self {
        let dot = gtk::Label::new(Some("●"));
        dot.add_css_class("dot");
        dot.add_css_class("idle");
        let name = gtk::Label::new(Some(title));
        name.add_css_class("pane-title");
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        header.append(&dot);
        header.append(&name);

        let view = gtk::TextView::builder()
            .editable(false)
            .cursor_visible(false)
            .monospace(true)
            .wrap_mode(gtk::WrapMode::WordChar)
            .left_margin(6)
            .right_margin(6)
            .top_margin(4)
            .bottom_margin(4)
            .build();
        let buffer = view.buffer();
        let end = buffer.create_mark(None, &buffer.end_iter(), false);
        let scroller = gtk::ScrolledWindow::builder().child(&view).hexpand(true).vexpand(true).build();
        let frame = gtk::Frame::builder().label_widget(&header).child(&scroller).build();
        Pane { frame, dot, view, end }
    }

    fn append(&self, line: &str) {
        let buffer = self.view.buffer();
        buffer.insert(&mut buffer.end_iter(), &format!("{line}\n"));
        self.view.scroll_to_mark(&self.end, 0.0, false, 0.0, 1.0);
    }

    fn text(&self) -> String {
        let buffer = self.view.buffer();
        buffer.text(&buffer.start_iter(), &buffer.end_iter(), false).to_string()
    }

    fn set_state(&self, state: &str) {
        for class in ["idle", "running", "failed"] {
            self.dot.remove_css_class(class);
        }
        self.dot.add_css_class(state);
    }

    fn failed(&self) -> bool {
        self.dot.has_css_class("failed")
    }
}

struct App {
    window: gtk::ApplicationWindow,
    tiles: gtk::Grid,
    panes: RefCell<BTreeMap<u32, Rc<Pane>>>,
    status: gtk::Label,
    engine: RefCell<Option<PathBuf>>,
    run: RefCell<Option<engine::Run>>,
    mode: Cell<Option<Mode>>,
    started: Cell<Option<Instant>>,
    errors: Cell<u32>,
    max_temp: Cell<i32>,
    /// The last test started, for Test > Continue.
    last: RefCell<Option<(Mode, Script)>>,
}

impl App {
    fn pane(&self, id: u32) -> Rc<Pane> {
        if let Some(pane) = self.panes.borrow().get(&id) {
            return pane.clone();
        }
        let title = if id == 0 { "Main thread".to_string() } else { format!("Worker #{id}") };
        let pane = Rc::new(Pane::new(&title));
        self.panes.borrow_mut().insert(id, pane.clone());
        self.tile();
        pane
    }

    /// Window > Tile: arrange the panes in a grid, like Prime95's tiled child windows.
    fn tile(&self) {
        while let Some(child) = self.tiles.first_child() {
            self.tiles.remove(&child);
        }
        let panes = self.panes.borrow();
        let cols = (panes.len() as f64).sqrt().ceil().max(1.0) as i32;
        for (i, pane) in panes.values().enumerate() {
            let i = i as i32;
            self.tiles.attach(&pane.frame, i % cols, i / cols, 1, 1);
        }
    }

    fn clear(&self) {
        let main = self.pane(0);
        main.view.buffer().set_text("");
        main.set_state("idle");
        self.panes.borrow_mut().retain(|id, _| *id == 0);
        self.tile();
    }

    /// Routes "[Worker #2 Nov 1 01:41] text" to Worker #2's pane as "[Nov 1 01:41] text".
    fn route(&self, line: &str) {
        let (id, shown) = if let Some(rest) = line.strip_prefix("[Worker #") {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            let id = digits.parse().unwrap_or(1);
            (id, line.replacen(&format!("Worker #{digits} "), "", 1))
        } else {
            (0, line.replacen("Main thread ", "", 1))
        };
        let pane = self.pane(id);
        pane.append(&shown);
        if FAILURE_MARKERS.iter().any(|m| line.contains(m)) {
            pane.set_state("failed");
            self.errors.set(self.errors.get() + 1);
        } else if self.mode.get().is_some() && !pane.failed() {
            pane.set_state("running");
        }
    }

    fn refresh_status(&self) {
        let temp = monitor::cpu_temp();
        let mhz = monitor::avg_mhz();
        if let (Some(t), Some(_)) = (temp, self.mode.get()) {
            self.max_temp.set(self.max_temp.get().max(t));
        }
        let mut parts = Vec::new();
        parts.push(match self.mode.get() {
            Some(Mode::Torture) => "Torture test running".to_string(),
            Some(Mode::Benchmark) => "Benchmark running".to_string(),
            None if self.engine.borrow().is_none() => "Prime95 engine not installed".to_string(),
            None => "Idle".to_string(),
        });
        if let (Some(start), Some(_)) = (self.started.get(), self.mode.get()) {
            let s = start.elapsed().as_secs();
            parts.push(format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60));
        }
        if let Some(mhz) = mhz {
            parts.push(format!("{mhz} MHz"));
        }
        if let Some(t) = temp {
            let max = self.max_temp.get();
            parts.push(if max > 0 { format!("{t}°C (max {max}°C)") } else { format!("{t}°C") });
        }
        if self.started.get().is_some() {
            parts.push(format!("Errors: {}", self.errors.get()));
        }
        self.status.set_label(&parts.join("   │   "));
        if self.errors.get() > 0 {
            self.status.add_css_class("failed");
        } else {
            self.status.remove_css_class("failed");
        }
        let running = self.mode.get().is_some();
        let has_engine = self.engine.borrow().is_some();
        let set = |name: &str, on: bool| {
            if let Some(action) = self.window.lookup_action(name).and_downcast::<gio::SimpleAction>() {
                action.set_enabled(on);
            }
        };
        set("torture", !running);
        set("benchmark", !running);
        set("cpu", !running && has_engine);
        set("stop", running);
        set("continue", !running && self.last.borrow().is_some());
    }

    fn start(self: &Rc<Self>, mode: Mode, script: Script) {
        let Some(engine) = self.engine.borrow().clone() else { return };
        if self.mode.get().is_some() {
            return;
        }
        self.clear();
        let (tx, rx) = async_channel::unbounded();
        match engine::start(&engine, script, mode, tx) {
            Ok(run) => *self.run.borrow_mut() = Some(run),
            Err(err) => {
                self.alert("Could not start the Prime95 engine", &err.to_string());
                return;
            }
        }
        self.mode.set(Some(mode));
        self.started.set(Some(Instant::now()));
        self.errors.set(0);
        self.max_temp.set(0);
        self.pane(0).set_state("running");
        self.refresh_status();

        let app = self.clone();
        glib::spawn_future_local(async move {
            while let Ok(event) = rx.recv().await {
                match event {
                    RunEvent::Line(line) => app.route(&line),
                    RunEvent::Done => break,
                }
            }
            app.finished(mode);
        });
    }

    fn finished(&self, mode: Mode) {
        self.run.borrow_mut().take();
        self.mode.set(None);
        for pane in self.panes.borrow().values() {
            if !pane.failed() {
                pane.set_state("idle");
            }
        }
        self.refresh_status();
        let errors = self.errors.get();
        let what = if mode == Mode::Torture { "Torture test" } else { "Benchmark" };
        if errors > 0 {
            self.alert(
                &format!("{what} found {errors} error(s)"),
                "Your CPU or memory made calculation errors. The system is not stable at its current \
                 settings (overclock, undervolt, memory timings or cooling). Workers with errors are marked red.",
            );
        }
    }

    fn alert(&self, message: &str, detail: &str) {
        gtk::AlertDialog::builder().modal(true).message(message).detail(detail).build().show(Some(&self.window));
    }

    /// Runs `f` with the engine, offering to download it first if it is missing.
    fn with_engine(self: &Rc<Self>, f: impl Fn(&Rc<Self>, PathBuf) + 'static) {
        if let Some(engine) = self.engine.borrow().clone() {
            return f(self, engine);
        }
        let app = self.clone();
        dialogs::engine_setup(self.window.upcast_ref(), move |path| {
            *app.engine.borrow_mut() = Some(path.clone());
            app.refresh_status();
            f(&app, path);
        });
    }

    fn copy_window(&self) {
        // The pane holding keyboard focus, else all of them.
        let focused = gtk::prelude::GtkWindowExt::focus(&self.window);
        let panes = self.panes.borrow();
        let text = panes
            .values()
            .find(|p| focused.as_ref().is_some_and(|f| f == p.view.upcast_ref::<gtk::Widget>()))
            .map(|p| p.text())
            .unwrap_or_else(|| panes.values().map(|p| p.text()).collect::<Vec<_>>().join("\n"));
        self.window.clipboard().set_text(&text);
    }

    fn show_file(&self, title: &str, path: PathBuf, missing: &str) {
        let text = std::fs::read_to_string(&path).unwrap_or_else(|_| missing.to_string());
        dialogs::text_window(self.window.upcast_ref(), title, &text);
    }
}

fn build_menu() -> gio::Menu {
    let section = |items: &[(&str, &str)]| {
        let s = gio::Menu::new();
        for (label, action) in items {
            s.append(Some(label), Some(action));
        }
        s
    };
    let submenu = |sections: &[gio::Menu]| {
        let m = gio::Menu::new();
        for s in sections {
            m.append_section(None, s);
        }
        m
    };
    let bar = gio::Menu::new();
    bar.append_submenu(
        Some("_Test"),
        &submenu(&[section(&[("_Continue", "win.continue"), ("St_op", "win.stop")]), section(&[("E_xit", "app.quit")])]),
    );
    bar.append_submenu(
        Some("_Edit"),
        &submenu(&[section(&[("_Copy Window", "win.copy"), ("C_lear Windows", "win.clear")])]),
    );
    bar.append_submenu(
        Some("_Options"),
        &submenu(&[
            section(&[("_CPU…", "win.cpu")]),
            section(&[("_Torture Test…", "win.torture"), ("_Benchmark…", "win.benchmark")]),
        ]),
    );
    bar.append_submenu(Some("_Window"), &submenu(&[section(&[("_Tile", "win.tile")])]));
    bar.append_submenu(
        Some("_Help"),
        &submenu(&[
            section(&[("_Results File", "win.results"), ("_Stress Testing Guide", "win.guide")]),
            section(&[("_About Prime95 GTK", "win.about")]),
        ]),
    );
    bar
}

fn build_window(gtk_app: &gtk::Application) -> Rc<App> {
    let tiles = gtk::Grid::builder()
        .row_homogeneous(true)
        .column_homogeneous(true)
        .row_spacing(6)
        .column_spacing(6)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(6)
        .margin_end(6)
        .hexpand(true)
        .vexpand(true)
        .build();
    let status = gtk::Label::builder().xalign(0.0).build();
    status.add_css_class("status-bar");
    status.add_css_class("numeric");
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&tiles);
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    content.append(&status);

    let window = gtk::ApplicationWindow::builder()
        .application(gtk_app)
        .title(APP_NAME)
        .default_width(1000)
        .default_height(680)
        .show_menubar(true)
        .child(&content)
        .build();

    let app = Rc::new(App {
        window,
        tiles,
        panes: RefCell::default(),
        status,
        engine: RefCell::new(engine::find_engine()),
        run: RefCell::default(),
        mode: Cell::new(None),
        started: Cell::new(None),
        errors: Cell::new(0),
        max_temp: Cell::new(0),
        last: RefCell::default(),
    });
    app.pane(0).append(
        "Welcome to Prime95 GTK.\n\nOptions > Torture Test… checks your CPU and memory for stability.\n\
         Options > Benchmark… measures performance.\n\nCurrent temperature and clock speed are shown below.",
    );

    let add = |name: &str, f: Box<dyn Fn(&Rc<App>)>| {
        let action = gio::SimpleAction::new(name, None);
        let weak = Rc::downgrade(&app);
        action.connect_activate(move |_, _| {
            if let Some(app) = weak.upgrade() {
                f(&app);
            }
        });
        app.window.add_action(&action);
    };
    add(
        "torture",
        Box::new(|app| {
            app.with_engine(|app, engine| {
                let a = app.clone();
                dialogs::torture(app.window.upcast_ref(), engine, move |script| {
                    remember(&a, Mode::Torture, script);
                });
            })
        }),
    );
    add(
        "benchmark",
        Box::new(|app| {
            app.with_engine(|app, engine| {
                let a = app.clone();
                dialogs::benchmark(app.window.upcast_ref(), engine, move |script| {
                    remember(&a, Mode::Benchmark, script);
                });
            })
        }),
    );
    add("cpu", Box::new(|app| app.with_engine(|app, engine| dialogs::cpu_info(app.window.upcast_ref(), engine))));
    add(
        "continue",
        Box::new(|app| {
            let last = app.last.borrow().clone();
            if let Some((mode, script)) = last {
                app.start(mode, script);
            }
        }),
    );
    add(
        "stop",
        Box::new(|app| {
            if let Some(run) = app.run.borrow().as_ref() {
                run.stop();
            }
        }),
    );
    add("copy", Box::new(|app| app.copy_window()));
    add("clear", Box::new(|app| app.clear()));
    add("tile", Box::new(|app| app.tile()));
    add(
        "results",
        Box::new(|app| {
            app.show_file("results.txt", engine::work_dir().join("results.txt"), "No results yet. Run a torture test or benchmark first.")
        }),
    );
    add(
        "guide",
        Box::new(|app| {
            let path = app.engine.borrow().as_ref().and_then(|e| e.parent().map(|d| d.join("stress.txt")));
            match path {
                Some(path) => app.show_file("Stress Testing Guide", path, "stress.txt was not found next to the engine."),
                None => app.alert("Prime95 engine not installed", "Open Options > Torture Test… to download it."),
            }
        }),
    );
    add(
        "about",
        Box::new(|app| {
            gtk::AboutDialog::builder()
                .transient_for(&app.window)
                .modal(true)
                .program_name(APP_NAME)
                .logo_icon_name(APP_ID)
                .version(env!("CARGO_PKG_VERSION"))
                .comments(
                    "A native Linux interface for Prime95's torture test and benchmark.\n\n\
                     The calculations are done by the official Prime95 engine (mprime) by \
                     George Woltman / GIMPS. This interface is unofficial and not affiliated with GIMPS.",
                )
                .website("https://github.com/OmarAlzand/prime95-gtk")
                .license_type(gtk::License::Gpl30)
                .build()
                .present();
        }),
    );

    let weak = Rc::downgrade(&app);
    glib::timeout_add_seconds_local(1, move || match weak.upgrade() {
        Some(app) => {
            app.refresh_status();
            glib::ControlFlow::Continue
        }
        None => glib::ControlFlow::Break,
    });
    app.refresh_status();
    app
}

/// Starts a test and keeps its settings for Test > Continue.
fn remember(app: &Rc<App>, mode: Mode, script: Script) {
    *app.last.borrow_mut() = Some((mode, script.clone()));
    app.start(mode, script);
}

fn main() -> glib::ExitCode {
    let gtk_app = gtk::Application::builder().application_id(APP_ID).build();

    gtk_app.connect_startup(|_| {
        let provider = gtk::CssProvider::new();
        provider.load_from_string(CSS);
        if let Some(display) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        }
    });

    let state: Rc<RefCell<Option<Rc<App>>>> = Rc::default();
    {
        let state = state.clone();
        gtk_app.connect_activate(move |gtk_app| {
            if let Some(app) = state.borrow().as_ref() {
                app.window.present();
                return;
            }
            gtk_app.set_menubar(Some(&build_menu()));
            let app = build_window(gtk_app);
            app.window.present();
            *state.borrow_mut() = Some(app);
        });
    }
    {
        let state = state.clone();
        gtk_app.connect_shutdown(move |_| {
            if let Some(run) = state.borrow().as_ref().and_then(|a| a.run.borrow_mut().take()) {
                run.stop();
            }
        });
    }

    let quit = gio::ActionEntry::builder("quit").activate(|app: &gtk::Application, _, _| app.quit()).build();
    gtk_app.add_action_entries([quit]);
    gtk_app.set_accels_for_action("app.quit", &["<Ctrl>q"]);
    gtk_app.set_accels_for_action("win.torture", &["<Ctrl>t"]);
    gtk_app.set_accels_for_action("win.benchmark", &["<Ctrl>b"]);

    gtk_app.run()
}
