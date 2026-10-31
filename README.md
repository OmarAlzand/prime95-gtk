# Prime95 GTK

A native Linux interface for **Prime95**, the classic CPU stress test, laid out like the
Windows version: a Main thread window plus one window per worker, the same menus, and the
same Torture Test and Benchmark dialogs. Written in Rust with GTK4.

The calculations are done by the **official Prime95 engine (`mprime`)** from
[GIMPS](https://www.mersenne.org/download/), so the test is exactly as strict as Prime95 on
Windows. Prime95 GTK walks the engine's own menus for you and shows its output, so every
default (FFT sizes for your caches, memory for Blend, time per FFT) comes straight from the engine.

> Unofficial. Not affiliated with GIMPS or Mersenne Research, Inc. Prime95 and mprime are
> by George Woltman and are distributed by GIMPS under their own license; this project does
> not include or redistribute them.

## Features

- **Options > Torture Test…** with the Windows options: cores, hyperthreading, Smallest /
  Small / Medium (on CPUs with L4) / Large FFTs / Blend / Custom, FFT range, memory, time per
  FFT, and the "weaker torture test" switches (disable AVX-512 / AVX2 / AVX)
- **Options > Benchmark…**: throughput, FFT timings and trial factoring benchmarks
- **Options > CPU…**: the engine's description of your processor
- One tiled window per worker, with a green (running) or red (error) light
- Live status bar: elapsed time, CPU clock speed, temperature and maximum temperature, error count
- Errors are detected as they happen and reported when the test ends
- **Test > Continue** repeats the last test; **Test > Stop** stops it cleanly
- **Help > Results File** and **Stress Testing Guide** (the engine's `results.txt` and `stress.txt`)
- Stress testing only: never asks for a PrimeNet account and never goes online
  (except the one-time engine download, if you choose it)

## Building

Dependencies: Rust (1.88+), GTK 4.12+.

```sh
# Arch / Manjaro
sudo pacman -S --needed rust gtk4
# Debian / Ubuntu
sudo apt install cargo libgtk-4-dev
# Fedora
sudo dnf install cargo gtk4-devel
```

```sh
make install        # builds and installs to ~/.local
make uninstall
```

## The engine

On first use, Prime95 GTK offers to download the official Linux engine from mersenne.org into
`~/.local/share/prime95-gtk/engine`. It also finds an engine you already have in:

- `$PRIME95_GTK_ENGINE` (full path to `mprime`)
- `~/.local/opt/prime95/mprime`
- `/opt/mprime/mprime` (the AUR `mprime` package)

Settings and results are kept in `~/.local/share/prime95-gtk/work`.

## Reading the results

A stable system finishes a torture test with **0 errors, 0 warnings** on every worker. Any
error means the CPU or memory miscalculated, usually from an overclock or undervolt, memory
settings, or overheating. Run a test for at least a few hours to be confident; see the
Stress Testing Guide in the Help menu.

## Tests

```sh
cargo test                           # unit tests
cargo test -- --ignored --nocapture  # drives a real engine (needs mprime installed)
```

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
