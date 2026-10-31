// The Options dialogs, laid out like Prime95's Windows dialogs.
// SPDX-License-Identifier: GPL-3.0-or-later

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{glib, Align, Orientation};

use crate::engine::{self, Query, Script};
use crate::monitor;

/// Runs an engine query on a worker thread and hands the result to `done` on the UI thread.
fn query_async(engine: PathBuf, script: Script, done: impl FnOnce(Query) + 'static) {
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking(engine::query(&engine, script));
    });
    glib::spawn_future_local(async move {
        if let Ok(Ok(result)) = rx.recv().await {
            done(result);
        }
    });
}

/// A modal dialog with Prime95's layout: content on the left, OK/Cancel stacked on the right.
fn dialog(parent: &gtk::Window, title: &str, content: &impl IsA<gtk::Widget>) -> (gtk::Window, gtk::Button) {
    let ok = gtk::Button::with_label("OK");
    ok.add_css_class("suggested-action");
    let cancel = gtk::Button::with_label("Cancel");
    let buttons = gtk::Box::new(Orientation::Vertical, 6);
    buttons.set_valign(Align::Start);
    buttons.append(&ok);
    buttons.append(&cancel);

    let body = gtk::Box::new(Orientation::Horizontal, 18);
    body.set_margin_top(14);
    body.set_margin_bottom(14);
    body.set_margin_start(14);
    body.set_margin_end(14);
    body.append(content);
    body.append(&buttons);

    let window = gtk::Window::builder()
        .transient_for(parent)
        .modal(true)
        .resizable(false)
        .title(title)
        .child(&body)
        .default_widget(&ok)
        .build();
    let w = window.downgrade();
    cancel.connect_clicked(move |_| {
        if let Some(w) = w.upgrade() {
            w.close();
        }
    });
    (window, ok)
}

fn label(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).use_underline(true).build()
}

fn spin(min: f64, max: f64, digits: u32) -> gtk::SpinButton {
    let step = if digits == 0 { 1.0 } else { 0.1 };
    let s = gtk::SpinButton::with_range(min, max, step);
    s.set_digits(digits);
    s
}

fn group(title: &str, child: &impl IsA<gtk::Widget>) -> gtk::Frame {
    child.set_margin_top(6);
    child.set_margin_bottom(8);
    child.set_margin_start(10);
    child.set_margin_end(10);
    gtk::Frame::builder().label(title).child(child).build()
}

fn grid() -> gtk::Grid {
    gtk::Grid::builder().row_spacing(6).column_spacing(10).build()
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Smallest,
    Small,
    Medium,
    Large,
    Blend,
    Custom,
}

impl Kind {
    /// The engine's own wording for each choice.
    fn engine_option(self) -> &'static str {
        match self {
            Kind::Smallest => "Smallest FFTs",
            Kind::Small => "Small FFTs",
            Kind::Medium => "Medium FFTs",
            Kind::Large => "Large FFTs",
            Kind::Blend | Kind::Custom => "Blend",
        }
    }
}

/// Options > Torture Test. `start` receives the finished script.
pub fn torture(parent: &gtk::Window, engine: PathBuf, start: impl Fn(Script) + 'static) {
    let cores = spin(1.0, 1.0, 0);
    let ht = gtk::CheckButton::with_mnemonic("Use hyperthreading (more stressful)");
    ht.set_visible(false);
    let top = gtk::Box::new(Orientation::Horizontal, 10);
    top.append(&label("Number of cores to _torture test:"));
    top.append(&cores);
    top.append(&ht);

    let kinds = [
        (Kind::Smallest, "Smallest FFTs (tests L1/L_2 caches, high power/heat/CPU stress)"),
        (Kind::Small, "Small FFTs (tests L1/L2/L_3 caches, maximum power/heat/CPU stress)"),
        (Kind::Medium, "Medium FFTs (tests L1/L2/L3/L_4 caches)"),
        (Kind::Large, "_Large FFTs (stresses memory controller and RAM)"),
        (Kind::Blend, "_Blend (all of the above)"),
        (Kind::Custom, "_Custom"),
    ];
    let radios: Vec<(Kind, gtk::CheckButton)> = kinds
        .iter()
        .map(|(kind, text)| (*kind, gtk::CheckButton::with_mnemonic(text)))
        .collect();
    for (_, radio) in &radios[1..] {
        radio.set_group(Some(&radios[0].1));
    }
    radios[2].1.set_visible(false); // only CPUs with an L4 cache get the Medium test
    radios[4].1.set_active(true);

    let min_fft = spin(4.0, 65536.0, 0);
    let max_fft = spin(4.0, 65536.0, 0);
    let in_place = gtk::CheckButton::with_mnemonic("Run FFTs _in-place");
    let memory = spin(0.0, 1024.0, 1);
    let minutes = spin(1.0, 60.0, 0);
    let settings = grid();
    settings.attach(&label("Mi_n FFT size (in K):"), 0, 0, 1, 1);
    settings.attach(&min_fft, 1, 0, 1, 1);
    settings.attach(&label("Ma_x FFT size (in K):"), 2, 0, 1, 1);
    settings.attach(&max_fft, 3, 0, 1, 1);
    settings.attach(&in_place, 0, 1, 1, 1);
    settings.attach(&label("_Memory to use (in GiB):"), 2, 1, 1, 1);
    settings.attach(&memory, 3, 1, 1, 1);
    settings.attach(&label("_Time to run each FFT size (in minutes):"), 0, 2, 3, 1);
    settings.attach(&minutes, 3, 2, 1, 1);
    let settings_group = group("Torture test settings", &settings);

    let no_avx512 = gtk::CheckButton::with_mnemonic("Disable AVX-_512");
    let no_fma3 = gtk::CheckButton::with_mnemonic("Disable AVX2 (_fused multiply-add)");
    let no_avx = gtk::CheckButton::with_mnemonic("Disable AV_X");
    no_avx512.set_visible(monitor::cpu_has("avx512f"));
    no_fma3.set_visible(monitor::cpu_has("fma"));
    no_avx.set_visible(monitor::cpu_has("avx"));
    let weak = gtk::Box::new(Orientation::Horizontal, 18);
    weak.append(&no_avx512);
    weak.append(&no_fma3);
    weak.append(&no_avx);
    let weak_group = group("Run a Weaker Torture Test (not recommended)", &weak);

    let content = gtk::Box::new(Orientation::Vertical, 6);
    content.append(&top);
    for (_, radio) in &radios {
        content.append(radio);
    }
    content.append(&settings_group);
    content.append(&weak_group);

    let (window, ok) = dialog(parent, "Run a Torture Test", &content);

    let kind = {
        let radios = radios.clone();
        move || radios.iter().find(|(_, r)| r.is_active()).map(|(k, _)| *k).unwrap_or(Kind::Blend)
    };

    // Settings are editable only for Custom, as on Windows; otherwise they show the engine's defaults.
    let sync_sensitivity = {
        let (kind, settings, in_place, memory) = (kind.clone(), settings.clone(), in_place.clone(), memory.clone());
        move || {
            settings.set_sensitive(kind() == Kind::Custom);
            memory.set_sensitive(!in_place.is_active());
        }
    };

    // The engine disables AVX-512, then AVX2, then AVX, so turning one off implies the ones above it.
    let sync_weak = {
        let (no_avx512, no_fma3, no_avx) = (no_avx512.clone(), no_fma3.clone(), no_avx.clone());
        move || {
            if no_avx.is_active() {
                no_fma3.set_active(true);
            }
            if no_fma3.is_active() {
                no_avx512.set_active(true);
            }
            no_fma3.set_sensitive(!no_avx.is_active());
            no_avx512.set_sensitive(!no_fma3.is_active());
        }
    };
    for check in [&no_avx512, &no_fma3, &no_avx] {
        let sync_weak = sync_weak.clone();
        check.connect_toggled(move |_| sync_weak());
    }

    let ready = Rc::new(Cell::new(false));
    let generation = Rc::new(Cell::new(0u32));
    let refresh_defaults = {
        let (engine, kind, cores, ht, ready, generation) =
            (engine.clone(), kind.clone(), cores.clone(), ht.clone(), ready.clone(), generation.clone());
        let (min_fft, max_fft, memory, minutes, in_place) =
            (min_fft.clone(), max_fft.clone(), memory.clone(), minutes.clone(), in_place.clone());
        move || {
            if !ready.get() || kind() == Kind::Custom {
                return;
            }
            let id = generation.get() + 1;
            generation.set(id);
            let script = Script::menu("Options/Torture Test")
                .text("Number of cores to torture test", cores.value_as_int())
                .yes_no("Use hyperthreading", ht.is_active())
                .pick("Type of torture test", kind().engine_option())
                .yes_no("Customize settings", true)
                .yes_no("Run a weaker torture test", false);
            let (generation, min_fft, max_fft, memory, minutes, in_place) =
                (generation.clone(), min_fft.clone(), max_fft.clone(), memory.clone(), minutes.clone(), in_place.clone());
            query_async(engine.clone(), script, move |q| {
                if generation.get() != id {
                    return; // a newer selection is already being looked up
                }
                q.number("Min FFT size").map(|v| min_fft.set_value(v));
                q.number("Max FFT size").map(|v| max_fft.set_value(v));
                let mem = q.number("Memory to use").unwrap_or(0.0);
                memory.set_value(mem);
                in_place.set_active(mem == 0.0);
                q.number("Time to run each FFT size").map(|v| minutes.set_value(v));
            });
        }
    };

    for (_, radio) in &radios {
        let (sync_sensitivity, refresh_defaults) = (sync_sensitivity.clone(), refresh_defaults.clone());
        radio.connect_toggled(move |r| {
            if r.is_active() {
                sync_sensitivity();
                refresh_defaults();
            }
        });
    }
    {
        let refresh_defaults = refresh_defaults.clone();
        cores.connect_value_changed(move |_| refresh_defaults());
    }
    {
        let refresh_defaults = refresh_defaults.clone();
        ht.connect_toggled(move |_| refresh_defaults());
    }
    {
        let (sync_sensitivity, memory) = (sync_sensitivity.clone(), memory.clone());
        in_place.connect_toggled(move |c| {
            if c.is_active() {
                memory.set_value(0.0);
            }
            sync_sensitivity();
        });
    }
    sync_sensitivity();

    // First look: the machine's core count, hyperthreading, and which test types exist.
    ok.set_sensitive(false);
    {
        let (cores, ht, radios, ready, ok, refresh_defaults) =
            (cores.clone(), ht.clone(), radios.clone(), ready.clone(), ok.clone(), refresh_defaults.clone());
        let script = Script::menu("Options/Torture Test").yes_no("Customize settings", false).yes_no("Run a weaker torture test", false);
        query_async(engine.clone(), script, move |q| {
            let hw_cores = q.number("Number of cores to torture test").unwrap_or(1.0);
            cores.set_range(1.0, hw_cores);
            cores.set_value(hw_cores);
            if q.asked("Use hyperthreading") {
                ht.set_visible(true);
                ht.set_active(q.yes("Use hyperthreading").unwrap_or(true));
            }
            let has_medium = q.lines.iter().any(|l| l.contains(" = Medium FFTs"));
            radios[2].1.set_visible(has_medium);
            ready.set(true);
            ok.set_sensitive(true);
            refresh_defaults();
        });
    }

    let w = window.downgrade();
    ok.connect_clicked(move |_| {
        let custom = kind() == Kind::Custom;
        let mut script = Script::menu("Options/Torture Test")
            .text("Number of cores to torture test", cores.value_as_int())
            .yes_no("Use hyperthreading", ht.is_active())
            .pick("Type of torture test", kind().engine_option())
            .yes_no("Customize settings", custom);
        if custom {
            let mem = if in_place.is_active() { 0.0 } else { memory.value() };
            script = script
                .text("Min FFT size", min_fft.value_as_int())
                .text("Max FFT size", max_fft.value_as_int().max(min_fft.value_as_int()))
                .text("Memory to use", format!("{mem:.1}"))
                .text("Time to run each FFT size", minutes.value_as_int());
        }
        let (a, f, x) = (
            no_avx512.is_visible() && no_avx512.is_active(),
            no_fma3.is_visible() && no_fma3.is_active(),
            no_avx.is_visible() && no_avx.is_active(),
        );
        script = script
            .yes_no("Run a weaker torture test", a || f || x)
            .yes_no("Disable AVX-512", a)
            .yes_no("Disable AVX2", f)
            .yes_no("Disable AVX", x);
        start(script);
        if let Some(w) = w.upgrade() {
            w.close();
        }
    });

    window.present();
}

/// Options > Benchmark.
pub fn benchmark(parent: &gtk::Window, engine: PathBuf, start: impl Fn(Script) + 'static) {
    let types = ["_Throughput", "_FFT timings", "T_rial factoring"].map(gtk::CheckButton::with_mnemonic);
    types[1].set_group(Some(&types[0]));
    types[2].set_group(Some(&types[0]));
    types[0].set_active(true);
    let type_box = gtk::Box::new(Orientation::Horizontal, 12);
    type_box.append(&label("Benchmark type:"));
    for t in &types {
        type_box.append(t);
    }

    let min_fft = spin(0.0, 65536.0, 0);
    let max_fft = spin(0.0, 65536.0, 0);
    let errchk = gtk::CheckButton::with_mnemonic("Benchmark with _round-off checking enabled");
    let negacyclic = gtk::CheckButton::with_mnemonic("Benchmark _negacyclic FFTs (for LLR, PFGW users)");
    let limit = gtk::CheckButton::with_mnemonic("_Limit FFT sizes (mimic older benchmarking code)");
    let ffts = grid();
    ffts.attach(&label("_Minimum FFT size (in K):"), 0, 0, 1, 1);
    ffts.attach(&min_fft, 1, 0, 1, 1);
    ffts.attach(&label("Ma_ximum FFT size (in K):"), 2, 0, 1, 1);
    ffts.attach(&max_fft, 3, 0, 1, 1);
    ffts.attach(&errchk, 0, 1, 4, 1);
    ffts.attach(&negacyclic, 0, 2, 4, 1);
    ffts.attach(&limit, 0, 3, 4, 1);
    let ffts_group = group("FFTs to benchmark", &ffts);

    let cores = gtk::Entry::new();
    let ht = gtk::CheckButton::with_mnemonic("Benchmark _hyperthreading");
    let cpu = grid();
    cpu.attach(&label("Number of _CPU cores (comma separated list of ranges):"), 0, 0, 1, 1);
    cpu.attach(&cores, 1, 0, 1, 1);
    cpu.attach(&ht, 0, 1, 2, 1);
    let cpu_group = group("CPU cores to benchmark", &cpu);

    let all_impl = gtk::CheckButton::with_mnemonic("Benchmark _all FFT implementations to find best one for your machine");
    let workers = gtk::Entry::new();
    let seconds = spin(5.0, 60.0, 0);
    let throughput = grid();
    throughput.attach(&all_impl, 0, 0, 2, 1);
    throughput.attach(&label("Number of _workers (comma separated list of ranges):"), 0, 1, 1, 1);
    throughput.attach(&workers, 1, 1, 1, 1);
    throughput.attach(&label("_Time to run each benchmark (in seconds):"), 0, 2, 1, 1);
    throughput.attach(&seconds, 1, 2, 1, 1);
    let throughput_group = group("Throughput benchmark options", &throughput);

    let content = gtk::Box::new(Orientation::Vertical, 8);
    content.append(&type_box);
    content.append(&ffts_group);
    content.append(&cpu_group);
    content.append(&throughput_group);

    let (window, ok) = dialog(parent, "Benchmark", &content);

    let bench_type = {
        let types = types.clone();
        move || types.iter().position(|t| t.is_active()).unwrap_or(0) as i32
    };

    let generation = Rc::new(Cell::new(0u32));
    let refresh = {
        let (engine, bench_type, generation, ok) = (engine.clone(), bench_type.clone(), generation.clone(), ok.clone());
        let (ffts_group, throughput_group) = (ffts_group.clone(), throughput_group.clone());
        let (min_fft, max_fft, errchk, negacyclic, limit) =
            (min_fft.clone(), max_fft.clone(), errchk.clone(), negacyclic.clone(), limit.clone());
        let (cores, ht, all_impl, workers, seconds, cpu_group) =
            (cores.clone(), ht.clone(), all_impl.clone(), workers.clone(), seconds.clone(), cpu_group.clone());
        move || {
            let t = bench_type();
            ffts_group.set_sensitive(t != 2);
            throughput_group.set_sensitive(t == 0);
            let id = generation.get() + 1;
            generation.set(id);
            ok.set_sensitive(false);
            let script = Script::menu("Options/Benchmark").text("Benchmark type", t);
            let (generation, ok) = (generation.clone(), ok.clone());
            let (min_fft, max_fft, errchk, negacyclic, limit) =
                (min_fft.clone(), max_fft.clone(), errchk.clone(), negacyclic.clone(), limit.clone());
            let (cores, ht, all_impl, workers, seconds, cpu_group) =
                (cores.clone(), ht.clone(), all_impl.clone(), workers.clone(), seconds.clone(), cpu_group.clone());
            query_async(engine.clone(), script, move |q| {
                if generation.get() != id {
                    return;
                }
                q.number("Minimum FFT size").map(|v| min_fft.set_value(v));
                q.number("Maximum FFT size").map(|v| max_fft.set_value(v));
                q.yes("round-off checking").map(|v| errchk.set_active(v));
                q.yes("negacyclic").map(|v| negacyclic.set_active(v));
                q.yes("Limit FFT sizes").map(|v| limit.set_active(v));
                q.default_of("Number of CPU cores").map(|v| cores.set_text(v));
                ht.set_visible(q.asked("Benchmark hyperthreading"));
                q.yes("Benchmark hyperthreading").map(|v| ht.set_active(v));
                cpu_group.set_visible(q.asked("Number of CPU cores") || q.asked("Benchmark hyperthreading"));
                q.yes("all FFT implementations").map(|v| all_impl.set_active(v));
                q.default_of("Number of workers").map(|v| workers.set_text(v));
                q.number("Time to run each benchmark").map(|v| seconds.set_value(v));
                ok.set_sensitive(true);
            });
        }
    };
    for t in &types {
        let refresh = refresh.clone();
        t.connect_toggled(move |t| {
            if t.is_active() {
                refresh();
            }
        });
    }
    refresh();

    let w = window.downgrade();
    ok.connect_clicked(move |_| {
        let script = Script::menu("Options/Benchmark")
            .text("Benchmark type", bench_type())
            .text("Minimum FFT size", min_fft.value_as_int())
            .text("Maximum FFT size", max_fft.value_as_int().max(min_fft.value_as_int()))
            .yes_no("round-off checking", errchk.is_active())
            .yes_no("negacyclic", negacyclic.is_active())
            .yes_no("Limit FFT sizes", limit.is_active())
            .text("Number of CPU cores", cores.text())
            .yes_no("Benchmark hyperthreading", ht.is_active())
            .yes_no("all FFT implementations", all_impl.is_active())
            .text("Number of workers", workers.text())
            .text("Time to run each benchmark", seconds.value_as_int());
        start(script);
        if let Some(w) = w.upgrade() {
            w.close();
        }
    });

    window.present();
}

/// A read-only text window (CPU information, results.txt, stress.txt).
pub fn text_window(parent: &gtk::Window, title: &str, text: &str) -> gtk::TextBuffer {
    let view = gtk::TextView::builder()
        .editable(false)
        .monospace(true)
        .wrap_mode(gtk::WrapMode::WordChar)
        .left_margin(10)
        .right_margin(10)
        .top_margin(8)
        .bottom_margin(8)
        .build();
    view.buffer().set_text(text);
    let scroller = gtk::ScrolledWindow::builder().child(&view).build();
    gtk::Window::builder()
        .transient_for(parent)
        .title(title)
        .default_width(720)
        .default_height(480)
        .child(&scroller)
        .build()
        .present();
    view.buffer()
}

/// Options > CPU: the engine's own description of the processor.
pub fn cpu_info(parent: &gtk::Window, engine: PathBuf) {
    let buffer = text_window(parent, "CPU Information", "Reading CPU information…");
    query_async(engine, Script::menu("Options/CPU"), move |q| {
        let mut text = String::new();
        if let Some(line) = q.lines.iter().find(|l| l.contains("Optimizing for CPU architecture")) {
            text.push_str(line.split("] ").nth(1).unwrap_or(line));
            text.push_str("\n\n");
        }
        let info = q.lines.iter().skip_while(|l| !l.starts_with("CPU Information")).skip(1);
        for line in info {
            text.push_str(line);
            text.push('\n');
        }
        buffer.set_text(text.trim());
    });
}

/// Shown when no engine is installed yet; offers the official download.
pub fn engine_setup(parent: &gtk::Window, done: impl Fn(PathBuf) + 'static) {
    let text = gtk::Label::builder()
        .label(
            "Prime95 GTK runs the official Prime95 engine (mprime) from GIMPS.\n\n\
             It is not installed yet. Download it now from mersenne.org?\n\
             (about 6 MB, stored in ~/.local/share/prime95-gtk/engine)",
        )
        .xalign(0.0)
        .wrap(true)
        .max_width_chars(60)
        .build();
    let spinner = gtk::Spinner::new();
    let content = gtk::Box::new(Orientation::Vertical, 12);
    content.append(&text);
    content.append(&spinner);
    let (window, ok) = dialog(parent, "Download Prime95 engine", &content);
    ok.set_label("Download");

    let w = window.downgrade();
    let done = Rc::new(done);
    ok.connect_clicked(move |ok| {
        ok.set_sensitive(false);
        spinner.start();
        text.set_label("Downloading the official Prime95 engine…");
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(engine::download_engine());
        });
        let (w, text, spinner, ok, done) = (w.clone(), text.clone(), spinner.clone(), ok.clone(), done.clone());
        glib::spawn_future_local(async move {
            let result = rx.recv().await;
            spinner.stop();
            match result {
                Ok(Ok(path)) => {
                    done(path);
                    if let Some(w) = w.upgrade() {
                        w.close();
                    }
                }
                Ok(Err(err)) => {
                    text.set_label(&err);
                    ok.set_sensitive(true);
                }
                Err(_) => {}
            }
        });
    });
    window.present();
}
