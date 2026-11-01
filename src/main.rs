//! `aa-macro` CLI - Phase 1 tools for checking the multi-instance plumbing.

use aa_macro::capture::Frame;
use aa_macro::config::Settings;
use aa_macro::control::Control;
use aa_macro::geom::Rect;
use aa_macro::input::{Arbiter, ArbiterOptions, Batch};
use aa_macro::layout;
use aa_macro::platform::{self, Platform};
use aa_macro::vision::{Library, Pattern};
use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

#[derive(Parser)]
#[command(
    name = "aa-macro",
    version,
    about = "Anime Adventures multi-instance macro (Phase 1 tools)"
)]
struct Cli {
    /// Settings file (default: aa-macro.json next to the exe).
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List running Roblox clients.
    List,
    /// Check Windows scaling, client sizes and capture for every client.
    Doctor,
    /// Resize every client to 800x599 and tile them on the main screen.
    Tile,
    /// Save a PNG of every client's viewport to ./captures (works when covered).
    Capture,
    /// Hover the centre of each client in turn through the input arbiter
    /// (no clicks, no keys) and report focus timing.
    Ping {
        #[arg(long, default_value_t = 2)]
        rounds: u32,
    },
    /// Save / test your own button and text patterns (see README).
    Pattern {
        #[command(subcommand)]
        cmd: PatternCmd,
    },
    /// Hold Roblox's single-instance lock so more clients can open.
    /// Start this BEFORE opening the first Roblox and keep it running.
    Unlock,
}

#[derive(Subcommand)]
enum PatternCmd {
    /// Cut a pattern out of a live client or a saved capture.
    ///   aa-macro pattern add "Vote Start" --rect 330,52,140,32
    Add {
        name: String,
        /// x,y,w,h of the crop in client pixels (read them off a capture PNG in Paint).
        #[arg(long)]
        rect: String,
        /// Live client number (from `list`).
        #[arg(long, default_value_t = 1)]
        instance: usize,
        /// Use a saved PNG instead of a live client.
        #[arg(long)]
        from: Option<PathBuf>,
        /// Colour template instead of a black/white text pattern.
        #[arg(long)]
        image: bool,
        /// Brightness cut for text patterns (0-255). Default: automatic.
        #[arg(long)]
        threshold: Option<u8>,
        /// How far around the crop to search later (pixels).
        #[arg(long, default_value_t = 40)]
        margin: i32,
    },
    /// Test patterns on a live client (or PNG). No names = all.
    Test {
        names: Vec<String>,
        #[arg(long, default_value_t = 1)]
        instance: usize,
        #[arg(long)]
        from: Option<PathBuf>,
        /// Search the whole client instead of each pattern's own box.
        #[arg(long)]
        everywhere: bool,
    },
    /// List saved patterns.
    List,
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    #[cfg(windows)]
    platform::win32::make_dpi_aware();

    let cli = Cli::parse();
    let path = cli.config.unwrap_or_else(Settings::default_path);
    let settings = Settings::load_or_create(&path)?;
    let p: Arc<dyn Platform> = Arc::from(platform::native());

    match cli.cmd {
        Cmd::List => list(p.as_ref(), &settings),
        Cmd::Doctor => doctor(p.as_ref(), &settings),
        Cmd::Tile => tile(p.as_ref(), &settings),
        Cmd::Capture => capture(p.as_ref(), &settings),
        Cmd::Ping { rounds } => ping(p, &settings, rounds),
        Cmd::Pattern { cmd } => pattern(p.as_ref(), &settings, cmd),
        Cmd::Unlock => unlock(&settings),
    }
}

fn clients(p: &dyn Platform, s: &Settings) -> Result<Vec<aa_macro::platform::WinInfo>> {
    let mut w = p.list_windows(&s.process_name);
    if w.is_empty() {
        bail!("no {} windows found - open Roblox first", s.process_name);
    }
    if w.len() > s.max_instances {
        println!(
            "found {} clients, using the first {} (max_instances)",
            w.len(),
            s.max_instances
        );
        w.truncate(s.max_instances);
    }
    Ok(w)
}

fn list(p: &dyn Platform, s: &Settings) -> Result<()> {
    let w = p.list_windows(&s.process_name);
    println!("{} Roblox client(s)", w.len());
    for (i, info) in w.iter().enumerate() {
        match p.geometry(info.hwnd) {
            Some(g) => println!(
                "  #{} hwnd {:#x} pid {:>6}  client {}x{} at ({}, {})  title {:?}",
                i + 1,
                info.hwnd.0,
                info.pid,
                g.client.w,
                g.client.h,
                g.client.x,
                g.client.y,
                info.title
            ),
            None => println!(
                "  #{} hwnd {:#x} pid {} (geometry unavailable)",
                i + 1,
                info.hwnd.0,
                info.pid
            ),
        }
    }
    Ok(())
}

fn doctor(p: &dyn Platform, s: &Settings) -> Result<()> {
    let w = clients(p, s)?;
    let mut problems = 0;
    for (i, info) in w.iter().enumerate() {
        let n = i + 1;
        let dpi = p.dpi(info.hwnd);
        if dpi != 96 {
            problems += 1;
            println!(
                "#{n} BAD  Windows scaling is {}% - set Display scale to 100%",
                dpi * 100 / 96
            );
        } else {
            println!("#{n} ok   scaling 100%");
        }
        if p.is_minimized(info.hwnd) {
            problems += 1;
            println!("#{n} BAD  minimised - run `aa-macro tile`");
            continue;
        }
        match p.geometry(info.hwnd) {
            Some(g) if g.client.w == s.client_w && g.client.h == s.client_h => {
                println!("#{n} ok   client {}x{}", g.client.w, g.client.h)
            }
            Some(g) => {
                problems += 1;
                println!(
                    "#{n} BAD  client {}x{}, patterns expect {}x{} - run `aa-macro tile` (F11 out of fullscreen first)",
                    g.client.w, g.client.h, s.client_w, s.client_h
                );
            }
            None => {
                problems += 1;
                println!("#{n} BAD  no geometry");
            }
        }
        let t = Instant::now();
        match p.capture_client(info.hwnd) {
            Ok(f) if f.is_blank() => {
                problems += 1;
                println!(
                    "#{n} BAD  capture is black (GPU/overlay blocks PrintWindow) - tell Claude"
                );
            }
            Ok(f) => println!(
                "#{n} ok   capture {}x{} in {} ms",
                f.width,
                f.height,
                t.elapsed().as_millis()
            ),
            Err(e) => {
                problems += 1;
                println!("#{n} BAD  capture failed: {e}");
            }
        }
    }
    println!(
        "{}",
        if problems == 0 {
            "all good".to_string()
        } else {
            format!("{problems} problem(s)")
        }
    );
    Ok(())
}

fn tile(p: &dyn Platform, s: &Settings) -> Result<()> {
    let w = clients(p, s)?;
    let first = p.geometry(w[0].hwnd);
    // Outer size that gives an exact 800x599 client with this window's borders.
    let (ow, oh) = first
        .map(|g| g.outer_for_client(s.client_w, s.client_h))
        .unwrap_or((816, 638));
    let rects = layout::tile(w.len(), p.work_area(), ow, oh);
    for (info, r) in w.iter().zip(rects) {
        p.move_resize(info.hwnd, r)?;
    }
    // Borders can differ per window; correct each one to the exact client size.
    for info in &w {
        if let Some(g) = p.geometry(info.hwnd) {
            if g.client.w != s.client_w || g.client.h != s.client_h {
                let (ow, oh) = g.outer_for_client(s.client_w, s.client_h);
                p.move_resize(info.hwnd, Rect::new(g.outer.x, g.outer.y, ow, oh))?;
            }
        }
    }
    println!("tiled {} client(s)", w.len());
    list(p, s)
}

fn capture(p: &dyn Platform, s: &Settings) -> Result<()> {
    let w = clients(p, s)?;
    for (i, info) in w.iter().enumerate() {
        let t = Instant::now();
        let f = p.capture_client(info.hwnd)?;
        let path = PathBuf::from("captures").join(format!("instance{}.png", i + 1));
        f.save_png(&path)?;
        println!(
            "#{} {}x{} in {} ms -> {}{}",
            i + 1,
            f.width,
            f.height,
            t.elapsed().as_millis(),
            path.display(),
            if f.is_blank() {
                "  (BLACK - capture blocked)"
            } else {
                ""
            }
        );
    }
    Ok(())
}

fn ping(p: Arc<dyn Platform>, s: &Settings, rounds: u32) -> Result<()> {
    let w = clients(p.as_ref(), s)?;
    let ctl = Control::new();
    let arb = Arbiter::start(
        p.clone(),
        ctl.clone(),
        ArbiterOptions {
            mouse_settle_ms: s.mouse_settle_ms,
            click_hold_ms: s.click_hold_ms,
            max_lease_ms: s.max_lease_ms,
        },
    );
    println!("hovering each client's centre, {rounds} round(s) - hands off the mouse");
    for r in 1..=rounds {
        for (i, info) in w.iter().enumerate() {
            let c = p
                .geometry(info.hwnd)
                .map(|g| (g.client.w / 2, g.client.h / 2))
                .unwrap_or((400, 300));
            let rep = arb.run(Batch::new(info.hwnd, "ping").move_to(c.0, c.1).wait(150))?;
            let fg = p.foreground() == Some(info.hwnd);
            println!(
                "round {r} #{}: focus {} | held {} ms",
                i + 1,
                if fg { "ok" } else { "LOST" },
                rep.held.as_millis()
            );
        }
    }
    Ok(())
}

fn unlock(s: &Settings) -> Result<()> {
    #[cfg(windows)]
    {
        let _held = platform::win32::hold_singleton(&s.singleton_names)?;
        println!(
            "holding {:?} - open your Roblox clients now. Keep this window open; Ctrl+C to quit.",
            s.singleton_names
        );
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }
    #[cfg(not(windows))]
    {
        let _ = s;
        bail!("unlock needs Windows")
    }
}

fn parse_rect(s: &str) -> Result<Rect> {
    let v: Vec<i32> = s
        .split(',')
        .map(|x| x.trim().parse())
        .collect::<Result<_, _>>()?;
    if v.len() != 4 || v[2] <= 0 || v[3] <= 0 {
        bail!("--rect must be x,y,w,h (e.g. 330,52,140,32)");
    }
    Ok(Rect::new(v[0], v[1], v[2], v[3]))
}

fn source(
    p: &dyn Platform,
    s: &Settings,
    instance: usize,
    from: &Option<PathBuf>,
) -> Result<Frame> {
    if let Some(f) = from {
        return Frame::load_png(f);
    }
    let w = clients(p, s)?;
    let info = w
        .get(instance.wrapping_sub(1))
        .ok_or_else(|| anyhow::anyhow!("no client #{instance} (have {})", w.len()))?;
    p.capture_client(info.hwnd)
}

fn pattern(p: &dyn Platform, s: &Settings, cmd: PatternCmd) -> Result<()> {
    let dir = Library::default_dir();
    let mut lib = Library::load(&dir)?;
    match cmd {
        PatternCmd::Add {
            name,
            rect,
            instance,
            from,
            image,
            threshold,
            margin,
        } => {
            let frame = source(p, s, instance, &from)?;
            let r = parse_rect(&rect)?;
            let pat = if image {
                Pattern::image_from(&frame, &name, r, margin)?
            } else {
                Pattern::text_from(&frame, &name, r, margin, threshold)?
            };
            let thr = pat.threshold;
            let found = pat.find_in(&frame, None);
            let path = lib.save(pat)?;
            println!("saved {}", path.display());
            if !image {
                println!(
                    "threshold {thr} - check {}.preview.png looks like the text (white on black)",
                    path.with_extension("").display()
                );
            }
            match found {
                Some(h) => println!(
                    "self-test: found at ({}, {}) score {:.2}",
                    h.at.x, h.at.y, h.score
                ),
                None => {
                    println!("self-test: NOT found - try another --threshold or a tighter --rect")
                }
            }
        }
        PatternCmd::Test {
            names,
            instance,
            from,
            everywhere,
        } => {
            if lib.is_empty() {
                bail!(
                    "no patterns in {} yet - add one with `aa-macro pattern add`",
                    dir.display()
                );
            }
            let frame = source(p, s, instance, &from)?;
            let names: Vec<String> = if names.is_empty() {
                lib.names().map(String::from).collect()
            } else {
                names
            };
            let full = Rect::new(0, 0, frame.width as i32, frame.height as i32);
            for n in names {
                let Some(pat) = lib.get(&n) else {
                    println!("{n:<24} ?  no such pattern");
                    continue;
                };
                let t = Instant::now();
                let hit = pat.find_in(&frame, everywhere.then_some(full));
                let us = t.elapsed().as_micros();
                match hit {
                    Some(h) => println!(
                        "{n:<24} FOUND at ({}, {}) score {:.2}  [{us} us]",
                        h.at.x, h.at.y, h.score
                    ),
                    None => println!("{n:<24} -      [{us} us]"),
                }
            }
        }
        PatternCmd::List => {
            println!("{} pattern(s) in {}", lib.len(), dir.display());
            for n in lib.names() {
                let pt = lib.get(n).unwrap();
                println!(
                    "  {n:<24} {:?} {}x{} search {:?}",
                    pt.kind,
                    pt.size().0,
                    pt.size().1,
                    pt.search
                );
            }
        }
    }
    Ok(())
}
