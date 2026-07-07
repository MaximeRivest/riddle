//! riddle — the diary of Tom Riddle, for the reMarkable Paper Pro.
//!
//! Write on the page with the pen. After a pause the diary drinks your ink,
//! and an answer writes itself onto the page in a flowing hand, then fades.
//!
//! Two display backends (picked at runtime): windowed via qtfb/AppLoad when
//! QTFB_KEY is set, or full takeover via the vendor engine (quill) when
//! built with --features takeover and launched with xochitl stopped.

mod display;
mod fb;
mod help;
mod ink;
mod oracle;
mod pen;
mod power;
mod qtfb;
mod script;
mod surface;
mod touch;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ab_glyph::FontRef;

use fb::{BBox, SCREEN_H, SCREEN_W};
use surface::{Surface, BLACK, WHITE};

const FONT_TTF: &[u8] = include_bytes!("../fonts/DancingScript.ttf");
const PNG_PATH: &str = "/tmp/riddle-page.png";

const IDLE_COMMIT: Duration = Duration::from_millis(2800);
const REPLY_PX: f32 = 96.0;
const MARGIN_X: i32 = 120;

enum State {
    Listening { last_pen: Option<Instant> },
    /// `reply`: a lingering reply the writer wrote underneath — drunk
    /// together with the new ink (empty when there was none).
    Drinking { stage: u32, next: Instant, region: BBox, reply: BBox, rx: mpsc::Receiver<Result<String, String>> },
    Thinking { rx: mpsc::Receiver<Result<String, String>>, pulse: Instant, blot_on: bool },
    Replying { plan: WritePlan, next: Instant, rx: Option<mpsc::Receiver<Result<String, String>>> },
    Lingering { until: Instant, region: BBox },
    FadingReply { stage: u32, next: Instant, region: BBox },
    /// The guide panel. `panel: None` = dismissed, waiting for pen-up so the
    /// dismissing touch doesn't leave a mark on the page.
    Help { panel: Option<help::Help>, until: Instant },
}

struct WritePlan {
    strokes: Vec<Vec<(i32, i32)>>,
    stroke_i: usize,
    point_i: usize,
    region: BBox,
    /// Where the next streamed chunk's first line starts.
    next_y: i32,
}

fn main() {
    // Hidden diagnostic: `riddle --oracle-test <image.png>` runs one oracle turn
    // and prints the streamed chunks, then exits. Lets you verify your endpoint
    // + key + model before ever launching the diary. No display needed.
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--oracle-test") {
        let png = args.get(2).map(String::as_str).unwrap_or("/tmp/riddle-page.png");
        std::process::exit(oracle_test(png));
    }
    if let Err(e) = run() {
        eprintln!("riddle: fatal: {e}");
        std::process::exit(1);
    }
}

fn oracle_test(png: &str) -> i32 {
    let o = match oracle::Oracle::spawn() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("oracle spawn failed: {e}");
            return 1;
        }
    };
    let (tx, rx) = mpsc::channel();
    let t0 = Instant::now();
    o.ask(png, tx);
    let mut got = String::new();
    loop {
        match rx.recv() {
            Ok(Ok(chunk)) => {
                if got.is_empty() {
                    eprintln!("first chunk +{}ms", t0.elapsed().as_millis());
                }
                print!("{chunk} ");
                use std::io::Write as _;
                let _ = std::io::stdout().flush();
                got.push_str(&chunk);
            }
            Ok(Err(e)) => {
                eprintln!("\noracle error: {e}");
                return 1;
            }
            Err(_) => break, // disconnected = reply complete
        }
    }
    println!("\n--- reply complete ({}ms, {} chars) ---", t0.elapsed().as_millis(), got.len());
    if got.trim().is_empty() { 1 } else { 0 }
}

fn run() -> std::io::Result<()> {
    let font = FontRef::try_from_slice(FONT_TTF).map_err(std::io::Error::other)?;

    let (disp, mut surf) = display::Display::open()?;
    let takeover = matches!(disp, display::Display::Quill);
    eprintln!(
        "riddle: display {} ({}x{} stride {})",
        if takeover { "quill/takeover" } else { "qtfb" },
        surf.w,
        surf.h,
        surf.stride
    );

    let mut pen_dev = match pen::PenDevice::open() {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("riddle: raw pen unavailable ({e}), falling back to qtfb pen events");
            None
        }
    };
    // Takeover mode: touch is ours too; 5-finger tap = quit.
    let mut touch_dev = if takeover { touch::TouchDevice::open().ok() } else { None };
    // Takeover mode: the power button is ours too (sleep page + suspend).
    let mut power_dev = if takeover {
        power::PowerButton::open().map_err(|e| eprintln!("riddle: no power button ({e})")).ok()
    } else {
        None
    };
    // Ignore power presses briefly after a wake: the waking press itself (and
    // key bounce) arrives on our grabbed fd and must not re-suspend.
    let mut power_grace = Instant::now();

    let sigterm = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&sigterm))?;
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&sigterm))?;

    // Blank page.
    surf.fill_rect(0, 0, SCREEN_W, SCREEN_H, WHITE);
    disp.update_all(surf.w, surf.h);

    // Warm the oracle now: pi loads Node + extensions + codex auth ONCE here,
    // while you're still picking up the pen, so replies pay only model latency.
    let oracle = match oracle::Oracle::spawn() {
        Ok(o) => {
            eprintln!("riddle: oracle warming (pi rpc)");
            Some(o)
        }
        Err(e) => {
            eprintln!("riddle: oracle spawn failed: {e}");
            None
        }
    };

    let mut user_ink = ink::Ink::new();
    let mut state = State::Listening { last_pen: None };
    let mut pen_down = false;
    // Raw stylus contact, tracked in every state (the guide dismisses on it).
    // `stylus_on` is the level; `stylus_tapped` latches any contact seen this
    // loop iteration, so a tap that starts AND ends within one drain still
    // registers.
    let mut stylus_on = false;
    let mut stylus_tapped = false;
    let mut ink_dirty = BBox::empty();
    // Experiment: while drawing, stamp a tiny faded footprint beside the ink.
    // This tests mixing precomposed pixel art with live pen updates. Off by
    // default — the stray prints read as ink blots over handwriting — and
    // opt-in via RIDDLE_FOOTPRINTS=on (or 1/true/yes).
    let footprints = std::env::var("RIDDLE_FOOTPRINTS")
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "on" | "true" | "yes"))
        .unwrap_or(false);
    let mut last_footstep: Option<(i32, i32)> = None;
    let mut footstep_i: u32 = 0;
    // A reply the writer started answering while it still lingered on the
    // page: it stays visible while they write and is drunk with their ink.
    let mut pending_reply = BBox::empty();
    let mut last_flush = Instant::now();
    // Takeover swaps are cheap and synchronous; qtfb needs coalescing.
    let flush_every = if takeover { Duration::from_millis(8) } else { Duration::from_millis(35) };

    eprintln!("riddle: the diary is open");

    loop {
        if sigterm.load(Ordering::Relaxed) {
            break;
        }
        if let Some(ref mut t) = touch_dev {
            if t.drain_check_quit() {
                eprintln!("riddle: 5-finger quit");
                break;
            }
        }

        // ---- power button: sleep page, suspend, restore on wake ----
        if let Some(ref mut p) = power_dev {
            let pressed = p.drain_pressed();
            if pressed && Instant::now() >= power_grace {
                eprintln!("riddle: sleeping (power button)");
                let saved = help::show_sleep(&mut surf, &font);
                disp.full_refresh(surf.w, surf.h);
                // Let the flashing refresh finish before the panel loses power.
                std::thread::sleep(Duration::from_millis(800));
                // Suspend, and confirm via the kernel's success counter. The
                // EPD regulator refuses to sleep while its post-update vpdd
                // timer (≤30s) runs — the whole suspend aborts with "Some
                // devices failed to suspend" — so retry until it sticks.
                let count0 = power::suspend_count();
                let mut attempts = 0;
                'sleeping: loop {
                    if p.grabbed {
                        let _ = std::process::Command::new("systemctl").arg("suspend").status();
                    }
                    attempts += 1;
                    let t0 = Instant::now();
                    while t0.elapsed() < Duration::from_secs(6) {
                        std::thread::sleep(Duration::from_millis(400));
                        if power::suspend_count() > count0 {
                            break 'sleeping;
                        }
                    }
                    if attempts >= 8 {
                        eprintln!("riddle: suspend never happened ({attempts} tries); waking the page");
                        break;
                    }
                    eprintln!("riddle: suspend aborted (EPD discharge timer), retrying");
                }
                eprintln!("riddle: waking");
                help::restore_sleep(&mut surf, &saved);
                disp.full_refresh(surf.w, surf.h);
                power::wifi_heal();
                // Discard input that queued while asleep — stale pen events
                // would otherwise replay as phantom ink on the restored page.
                if let Some(ref mut pd) = pen_dev {
                    let _ = pd.drain();
                }
                if let Some(ref mut td) = touch_dev {
                    let _ = td.drain_check_quit();
                }
                p.drain_pressed();
                power_grace = Instant::now() + Duration::from_secs(3);
            }
        }

        // ---- raw pen (preferred path) ----
        if let Some(ref mut pdev) = pen_dev {
            for s in pdev.drain() {
                let writing = s.touching && s.pressure > 40;
                stylus_on = writing;
                stylus_tapped |= writing;
                if !writing {
                    if pen_down {
                        pen_down = false;
                        user_ink.pen_up();
                        last_footstep = None;
                        if let State::Listening { ref mut last_pen } = state {
                            *last_pen = Some(Instant::now());
                        }
                    }
                    continue;
                }
                // Pen contact while a reply lingers: keep Tom's words on the
                // page and start inking with this same contact — the reply is
                // remembered and drunk together with the new ink at commit.
                if let State::Lingering { region, .. } = state {
                    if !region.is_empty() {
                        pending_reply.add(region.x0, region.y0, 0);
                        pending_reply.add(region.x1, region.y1, 0);
                    }
                    state = State::Listening { last_pen: None };
                }
                match state {
                    State::Listening { ref mut last_pen } => {
                        pen_down = true;
                        let d = match s.tool {
                            pen::Tool::Pen => {
                                let r = 2 + s.pressure * 3 / pen::MAX_PRESSURE;
                                let mut d = user_ink.pen_point(&mut surf, s.x, s.y, r);
                                if footprints && should_stamp_footstep(last_footstep, s.x, s.y) {
                                    let f = draw_faded_footstep(&mut surf, s.x + 52, s.y - 38, footstep_i);
                                    d.add(f.x0, f.y0, 0);
                                    d.add(f.x1, f.y1, 0);
                                    last_footstep = Some((s.x, s.y));
                                    footstep_i = footstep_i.wrapping_add(1);
                                }
                                d
                            }
                            pen::Tool::Eraser => user_ink.erase_point(&mut surf, s.x, s.y, 22),
                        };
                        if !d.is_empty() {
                            ink_dirty.add(d.x0, d.y0, 0);
                            ink_dirty.add(d.x1, d.y1, 0);
                        }
                        *last_pen = Some(Instant::now());
                    }
                    _ => {}
                }
            }
        }

        // ---- window-system events (qtfb close detection + pen fallback) ----
        let events = match disp.pump() {
            Ok(v) => v,
            Err(_) => break, // qtfb window closed
        };
        for ev in events {
            if pen_dev.is_some() {
                continue;
            }
            match ev.input_type {
                qtfb::INPUT_PEN_PRESS | qtfb::INPUT_PEN_UPDATE => {
                    stylus_on = true;
                    stylus_tapped = true;
                    // Same as the raw-pen path: writing over a lingering
                    // reply keeps it on the page.
                    if let State::Lingering { region, .. } = state {
                        if !region.is_empty() {
                            pending_reply.add(region.x0, region.y0, 0);
                            pending_reply.add(region.x1, region.y1, 0);
                        }
                        state = State::Listening { last_pen: None };
                    }
                    if let State::Listening { ref mut last_pen } = state {
                        pen_down = true;
                        let r = 2 + ev.d.clamp(0, 100) / 45;
                        let mut d = user_ink.pen_point(&mut surf, ev.x, ev.y, r);
                        if footprints && should_stamp_footstep(last_footstep, ev.x, ev.y) {
                            let f = draw_faded_footstep(&mut surf, ev.x + 52, ev.y - 38, footstep_i);
                            d.add(f.x0, f.y0, 0);
                            d.add(f.x1, f.y1, 0);
                            last_footstep = Some((ev.x, ev.y));
                            footstep_i = footstep_i.wrapping_add(1);
                        }
                        if !d.is_empty() {
                            ink_dirty.add(d.x0, d.y0, 0);
                            ink_dirty.add(d.x1, d.y1, 0);
                        }
                        *last_pen = Some(Instant::now());
                    }
                }
                qtfb::INPUT_PEN_RELEASE => {
                    stylus_on = false;
                    if pen_down {
                        pen_down = false;
                        user_ink.pen_up();
                        last_footstep = None;
                        if let State::Listening { ref mut last_pen } = state {
                            *last_pen = Some(Instant::now());
                        }
                    }
                }
                _ => {}
            }
        }

        // ---- coalesced ink flush ----
        if !ink_dirty.is_empty() && last_flush.elapsed() >= flush_every {
            let (x, y, w, h) = ink_dirty.rect();
            disp.update(x, y, w, h, true);
            ink_dirty = BBox::empty();
            last_flush = Instant::now();
        }

        // ---- state machine ----
        state = match state {
            State::Listening { last_pen } => match last_pen {
                Some(t) if !pen_down && t.elapsed() >= IDLE_COMMIT && !user_ink.is_empty() => {
                    if help::looks_like_question_mark(user_ink.stroke_list()) {
                        // Absorb the "?" and open the guide instead of asking.
                        let (qx, qy, qw, qh) = user_ink.bbox.rect();
                        surf.fill_rect(qx as usize, qy as usize, qw as usize, qh as usize, WHITE);
                        disp.update(qx, qy, qw, qh, false);
                        user_ink.clear();
                        let panel = help::show(&mut surf, &font);
                        let (px, py, pw, ph) = panel.region.rect();
                        disp.update(px, py, pw, ph, false);
                        eprintln!("riddle: guide shown");
                        State::Help { panel: Some(panel), until: Instant::now() + Duration::from_secs(45) }
                    } else {
                        if let Err(e) = user_ink.to_png(&surf, PNG_PATH) {
                            eprintln!("riddle: rasterize failed: {e}");
                        }
                        // Ask NOW: the model streams while the diary drinks the
                        // ink, hiding most of the reply latency in the animation.
                        let (tx, rx) = mpsc::channel();
                        if let Some(ref o) = oracle {
                            o.ask(PNG_PATH, tx);
                        } else {
                            let _ = tx.send(Err("no oracle".into()));
                        }
                        let region = user_ink.bbox;
                        let reply = pending_reply;
                        pending_reply = BBox::empty();
                        State::Drinking { stage: 0, next: Instant::now(), region, reply, rx }
                    }
                }
                _ => State::Listening { last_pen },
            },

            State::Drinking { stage, next, region, reply, rx } => {
                const STAGES: u32 = 14;
                if Instant::now() >= next {
                    ink::dissolve_pass(&mut surf, region, stage, STAGES);
                    let (x, y, w, h) = region.rect();
                    disp.update(x, y, w, h, true);
                    // A reply the writer answered underneath dissolves along
                    // with their ink.
                    if !reply.is_empty() {
                        ink::dissolve_pass(&mut surf, reply, stage, STAGES);
                        let (x, y, w, h) = reply.rect();
                        disp.update(x, y, w, h, true);
                    }
                    if stage + 1 >= STAGES {
                        user_ink.clear();
                        State::Thinking { rx, pulse: Instant::now(), blot_on: false }
                    } else {
                        State::Drinking { stage: stage + 1, next: Instant::now() + Duration::from_millis(70), region, reply, rx }
                    }
                } else {
                    State::Drinking { stage, next, region, reply, rx }
                }
            }

            State::Thinking { rx, pulse, blot_on } => match rx.try_recv() {
                Ok(result) => {
                    surf.fill_rect(SCREEN_W / 2 - 14, SCREEN_H / 2 - 14, 28, 28, WHITE);
                    disp.update(SCREEN_W as i32 / 2 - 14, SCREEN_H as i32 / 2 - 14, 28, 28, true);
                    // First streamed chunk: start writing now; keep the
                    // receiver so the rest of the reply can append itself.
                    let (text, rx) = match result {
                        Ok(t) => (t, Some(rx)),
                        Err(e) => {
                            eprintln!("riddle: oracle failed: {e}");
                            ("…".to_string(), None)
                        }
                    };
                    let plan = plan_reply(&font, &text, None);
                    State::Replying { plan, next: Instant::now(), rx }
                }
                Err(mpsc::TryRecvError::Empty) => {
                    if pulse.elapsed() >= Duration::from_millis(600) {
                        let (cx, cy) = (SCREEN_W as i32 / 2, SCREEN_H as i32 / 2);
                        if blot_on {
                            surf.fill_rect(cx as usize - 14, cy as usize - 14, 28, 28, WHITE);
                        } else {
                            surf.stamp(cx, cy, 9, BLACK);
                        }
                        disp.update(cx - 14, cy - 14, 28, 28, true);
                        State::Thinking { rx, pulse: Instant::now(), blot_on: !blot_on }
                    } else {
                        State::Thinking { rx, pulse, blot_on }
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => State::Listening { last_pen: None },
            },

            State::Replying { mut plan, next, mut rx } => {
                // More of the reply may still be streaming in: append each
                // new chunk below what is already planned, mid-animation.
                if let Some(ref r) = rx {
                    let drop_rx = match r.try_recv() {
                        Ok(Ok(more)) => {
                            append_reply(&font, &mut plan, &more);
                            false
                        }
                        Ok(Err(e)) => {
                            eprintln!("riddle: oracle failed mid-reply: {e}");
                            true
                        }
                        Err(mpsc::TryRecvError::Disconnected) => true,
                        Err(mpsc::TryRecvError::Empty) => false,
                    };
                    if drop_rx {
                        rx = None;
                    }
                }
                if Instant::now() >= next {
                    let mut dirty = BBox::empty();
                    let mut budget = 26;
                    while budget > 0 && plan.stroke_i < plan.strokes.len() {
                        let stroke = &plan.strokes[plan.stroke_i];
                        if plan.point_i >= stroke.len() {
                            plan.stroke_i += 1;
                            plan.point_i = 0;
                            continue;
                        }
                        let (x, y) = stroke[plan.point_i];
                        if plan.point_i > 0 {
                            let (px, py) = stroke[plan.point_i - 1];
                            surf.brush_line(px, py, x, y, 2, BLACK);
                        } else {
                            surf.stamp(x, y, 2, BLACK);
                        }
                        dirty.add(x, y, 4);
                        plan.point_i += 1;
                        budget -= 1;
                    }
                    if !dirty.is_empty() {
                        let (x, y, w, h) = dirty.rect();
                        disp.update(x, y, w, h, true);
                    }
                    if plan.stroke_i >= plan.strokes.len() && rx.is_none() {
                        let chars: usize = plan.strokes.iter().map(|s| s.len()).sum();
                        let linger = Duration::from_millis(4000 + (chars as u64) * 2);
                        let region = plan.region;
                        State::Lingering { until: Instant::now() + linger.min(Duration::from_secs(20)), region }
                    } else {
                        State::Replying { plan, next: Instant::now() + Duration::from_millis(14), rx }
                    }
                } else {
                    State::Replying { plan, next, rx }
                }
            }

            State::Lingering { until, region } => {
                if Instant::now() >= until {
                    State::FadingReply { stage: 0, next: Instant::now(), region }
                } else {
                    State::Lingering { until, region }
                }
            }

            State::Help { panel, until } => match panel {
                Some(p) => {
                    if stylus_tapped || Instant::now() >= until {
                        let region = p.dismiss(&mut surf);
                        let (x, y, w, h) = region.rect();
                        disp.update(x, y, w, h, false);
                        eprintln!("riddle: guide dismissed");
                        State::Help { panel: None, until }
                    } else {
                        State::Help { panel: Some(p), until }
                    }
                }
                // Dismissed: swallow the closing touch, listen again on pen-up.
                None if stylus_on => State::Help { panel: None, until },
                None => State::Listening { last_pen: None },
            },

            State::FadingReply { stage, next, region } => {
                const STAGES: u32 = 10;
                if Instant::now() >= next {
                    ink::dissolve_pass(&mut surf, region, stage, STAGES);
                    let (x, y, w, h) = region.rect();
                    disp.update(x, y, w, h, true);
                    if stage + 1 >= STAGES {
                        disp.full_refresh(surf.w, surf.h);
                        State::Listening { last_pen: None }
                    } else {
                        State::FadingReply { stage: stage + 1, next: Instant::now() + Duration::from_millis(80), region }
                    }
                } else {
                    State::FadingReply { stage, next, region }
                }
            }
        };

        stylus_tapped = false;
        std::thread::sleep(Duration::from_millis(2));
    }

    eprintln!("riddle: the diary closes");
    disp.terminate();
    Ok(())
}

fn should_stamp_footstep(last: Option<(i32, i32)>, x: i32, y: i32) -> bool {
    match last {
        None => true,
        Some((lx, ly)) => {
            let dx = x - lx;
            let dy = y - ly;
            dx * dx + dy * dy >= 120 * 120
        }
    }
}

/// Stamp a tiny solid black footprint beside live ink.
fn draw_faded_footstep(surf: &mut Surface, x: i32, y: i32, i: u32) -> BBox {
    let side = if i % 2 == 0 { -1 } else { 1 };
    let tilt = side * 5;
    let mut bbox = BBox::empty();
    solid_ellipse(surf, x, y, 8, 12, &mut bbox);
    solid_ellipse(surf, x + side * 8, y - 15, 5, 7, &mut bbox);
    solid_ellipse(surf, x + side * 2 + tilt, y - 25, 3, 4, &mut bbox);
    solid_ellipse(surf, x + side * 9 + tilt, y - 27, 3, 4, &mut bbox);
    solid_ellipse(surf, x + side * 15 + tilt, y - 23, 2, 3, &mut bbox);
    bbox
}

fn solid_ellipse(
    surf: &mut Surface,
    cx: i32,
    cy: i32,
    rx: i32,
    ry: i32,
    bbox: &mut BBox,
) {
    for dy in -ry..=ry {
        for dx in -rx..=rx {
            if dx * dx * ry * ry + dy * dy * rx * rx <= rx * rx * ry * ry {
                surf.put_px(cx + dx, cy + dy, BLACK);
                bbox.add(cx + dx, cy + dy, 1);
            }
        }
    }
}

/// Lay out reply text and produce screen-space strokes. `y_start` continues a
/// streamed reply below its previous chunk; None places the first chunk.
fn plan_reply(font: &FontRef, text: &str, y_start: Option<i32>) -> WritePlan {
    let max_w = (SCREEN_W as i32 - 2 * MARGIN_X) as f32;
    let lines = script::wrap(font, text, REPLY_PX, max_w);
    let line_h = (REPLY_PX * 1.25) as i32;
    let total_h = line_h * lines.len() as i32;
    let mut y = y_start.unwrap_or(((SCREEN_H as i32 - total_h) / 3).max(60));
    let mut strokes = Vec::new();
    let mut region = BBox::empty();
    let mut seed = 0x1234u32;
    let mut jitter = move || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        ((seed >> 16) % 7) as i32 - 3
    };

    for line_text in &lines {
        let mut raster = script::rasterize_line(font, line_text, REPLY_PX);
        script::thin(&mut raster);
        let line_strokes = script::trace(&raster);
        let x0 = (SCREEN_W as i32 - raster.width as i32) / 2;
        let wobble = jitter();
        for s in line_strokes {
            let mapped: Vec<(i32, i32)> = s.iter().map(|&(sx, sy)| (x0 + sx, y + sy + wobble)).collect();
            for &(x, yy) in &mapped {
                region.add(x, yy, 5);
            }
            strokes.push(mapped);
        }
        y += line_h;
    }

    WritePlan { strokes, stroke_i: 0, point_i: 0, region, next_y: y }
}

/// Splice a streamed continuation chunk into a running write animation.
fn append_reply(font: &FontRef, plan: &mut WritePlan, more: &str) {
    let cont = plan_reply(font, more, Some(plan.next_y));
    if cont.strokes.is_empty() {
        return;
    }
    plan.region.add(cont.region.x0, cont.region.y0, 0);
    plan.region.add(cont.region.x1, cont.region.y1, 0);
    plan.strokes.extend(cont.strokes);
    plan.next_y = cont.next_y;
}
