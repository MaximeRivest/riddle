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
mod memory;
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
use oracle::Event;
use surface::{Surface, BLACK, FADED, WHITE};

const FONT_TTF: &[u8] = include_bytes!("../fonts/DancingScript.ttf");
const PNG_PATH: &str = "/tmp/riddle-page.png";

const IDLE_COMMIT: Duration = Duration::from_millis(2800);
/// How long the diary waits on a silent oracle before giving up on the turn.
/// Generous: thinking models can lead with a long silence.
const ORACLE_PATIENCE: Duration = Duration::from_secs(120);
const REPLY_PX: f32 = 96.0;
const MARGIN_X: i32 = 120;

const USAGE: &str = "\
riddle — the diary of Tom Riddle

usage:
  riddle                      open the diary (windowed when AppLoad sets
                              QTFB_KEY, otherwise takeover via libquill)
  riddle --oracle-test [PNG]  run one oracle turn against PNG (default
                              /tmp/riddle-page.png) and print the streamed
                              reply; verifies key + endpoint + model
  riddle --version            print the version

configuration lives in oracle.env next to the binary — see
oracle.env.example for every RIDDLE_* variable.
";

type OracleRx = mpsc::Receiver<Result<Event, String>>;

enum State {
    Listening { last_pen: Option<Instant> },
    Drinking { stage: u32, next: Instant, region: BBox, rx: OracleRx },
    Thinking { rx: OracleRx, pulse: Instant, blot_on: bool, since: Instant },
    Replying { plan: WritePlan, next: Instant, rx: Option<OracleRx> },
    Lingering { until: Instant, region: BBox },
    FadingReply { stage: u32, next: Instant, region: BBox },
    /// The guide panel. `panel: None` = dismissed, waiting for pen-up so the
    /// dismissing touch doesn't leave a mark on the page.
    Help { panel: Option<help::Help>, until: Instant },
    /// A remembered page rising through the paper: date, the writer's own
    /// past ink, Tom's old reply — all in faded ink. `saved` is today's page.
    Conjuring { plan: ConjurePlan, next: Instant, saved: Vec<u8> },
    /// The conjured memory rests on the page. Pen contact (or time) dissolves
    /// it and today's page returns. `saved: None` = dismissed, waiting pen-up.
    MemoryShown { saved: Option<Vec<u8>>, until: Instant, region: BBox },
}

/// A memory being rewritten onto the page: pre-positioned strokes with their
/// original radii, drawn in faded ink.
struct ConjurePlan {
    strokes: Vec<Vec<(i32, i32, i32)>>,
    stroke_i: usize,
    point_i: usize,
    region: BBox,
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
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        // Diagnostic: run one oracle turn and print the streamed chunks.
        // Lets you verify your endpoint + key + model before ever launching
        // the diary. No display needed.
        Some("--oracle-test") => {
            let png = args.get(2).map(String::as_str).unwrap_or(PNG_PATH);
            std::process::exit(oracle_test(png));
        }
        Some("--version" | "-V") => {
            println!("riddle {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        Some("--help" | "-h") => {
            print!("{USAGE}");
            return;
        }
        Some(flag) if flag.starts_with('-') => {
            eprintln!("riddle: unknown flag {flag}\n");
            eprint!("{USAGE}");
            std::process::exit(2);
        }
        _ => {}
    }
    if let Err(e) = run() {
        eprintln!("riddle: fatal: {e}");
        std::process::exit(1);
    }
}

fn oracle_test(png: &str) -> i32 {
    let store = memory::MemoryStore::open();
    let o = match oracle::Oracle::spawn(store.is_some()) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("oracle spawn failed: {e}");
            return 1;
        }
    };
    let ctx = build_ctx(&store);
    let (tx, rx) = mpsc::channel();
    let t0 = Instant::now();
    o.ask(png, &ctx, tx);
    let mut got = String::new();
    loop {
        match rx.recv() {
            Ok(Ok(Event::Ink(chunk))) => {
                if got.is_empty() {
                    eprintln!("first chunk +{}ms", t0.elapsed().as_millis());
                }
                print!("{chunk} ");
                use std::io::Write as _;
                let _ = std::io::stdout().flush();
                got.push_str(&chunk);
            }
            Ok(Ok(Event::Show(id))) => {
                println!("[would conjure memory {id} — {}]", memory::spoken_date(id));
                got.push_str("(show)");
            }
            Ok(Ok(Event::Transcript(t))) => eprintln!("\n[transcript] {t}"),
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

/// What the diary sends alongside the page: its memory of recent turns and
/// the catalog the oracle picks conjured pages from. Empty when memory is off.
fn build_ctx(store: &Option<memory::MemoryStore>) -> oracle::TurnContext {
    let Some(s) = store else { return oracle::TurnContext::default() };
    let turns: usize = std::env::var("RIDDLE_MEMORY_TURNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6);
    let (catalog_lines, catalog_ids) = s.catalog(40);
    oracle::TurnContext { history: s.recent_dialogue(turns), catalog_lines, catalog_ids }
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

    // The diary's memory (None = RIDDLE_MEMORY=off or the dir is unusable).
    let mut store = memory::MemoryStore::open();
    if let Some(ref s) = store {
        eprintln!("riddle: memory holds {} pages", s.entries.len());
    }

    // Warm the oracle now: pi loads Node + extensions + codex auth ONCE here,
    // while you're still picking up the pen, so replies pay only model latency.
    let oracle = match oracle::Oracle::spawn(store.is_some()) {
        Ok(o) => {
            eprintln!("riddle: oracle ready");
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
    // The turn being remembered: strokes captured at commit, transcript and
    // reply accumulated as they stream, stored when the turn completes.
    let mut turn_id: u64 = 0;
    let mut turn_strokes: memory::Strokes = Vec::new();
    let mut turn_reply = String::new();
    let mut turn_transcript: Option<String> = None;
    let mut turn_failed = false;
    // Raw stylus contact, tracked in every state (the guide dismisses on it).
    // `stylus_on` is the level; `stylus_tapped` latches any contact seen this
    // loop iteration, so a tap that starts AND ends within one drain still
    // registers.
    let mut stylus_on = false;
    let mut stylus_tapped = false;
    let mut ink_dirty = BBox::empty();
    // Basilisk-fang erase: hold the eraser still on one spot for 3s and the
    // diary's memory dies. (origin x, origin y, press start), the nominal
    // splat radius already inked, and the bbox of everything the splat drew.
    let mut stab: Option<(i32, i32, Instant)> = None;
    let mut bleed: Option<Bleed> = None;
    // Latest eraser contact this loop; None once the pen lifts or flips.
    let mut eraser_at: Option<(i32, i32)> = None;
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
                        eraser_at = None;
                        if let State::Listening { ref mut last_pen } = state {
                            *last_pen = Some(Instant::now());
                        }
                    }
                    continue;
                }
                match state {
                    State::Listening { ref mut last_pen } => {
                        pen_down = true;
                        let d = match s.tool {
                            pen::Tool::Pen => {
                                eraser_at = None;
                                let r = 2 + s.pressure * 3 / pen::MAX_PRESSURE;
                                user_ink.pen_point(&mut surf, s.x, s.y, r)
                            }
                            pen::Tool::Eraser => {
                                eraser_at = Some((s.x, s.y));
                                // While the stab's warning ink pools, the tip
                                // is a fang, not an eraser: don't erase (and
                                // don't drop stroke points under the splat).
                                if bleed.is_some() {
                                    BBox::empty()
                                } else {
                                    user_ink.erase_point(&mut surf, s.x, s.y, 22)
                                }
                            }
                        };
                        if !d.is_empty() {
                            ink_dirty.add(d.x0, d.y0, 0);
                            ink_dirty.add(d.x1, d.y1, 0);
                        }
                        *last_pen = Some(Instant::now());
                    }
                    State::Lingering { region, .. } => {
                        state = State::FadingReply { stage: 0, next: Instant::now(), region };
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
                    if let State::Listening { ref mut last_pen } = state {
                        pen_down = true;
                        let r = 2 + ev.d.clamp(0, 100) / 45;
                        let d = user_ink.pen_point(&mut surf, ev.x, ev.y, r);
                        if !d.is_empty() {
                            ink_dirty.add(d.x0, d.y0, 0);
                            ink_dirty.add(d.x1, d.y1, 0);
                        }
                        *last_pen = Some(Instant::now());
                    } else if let State::Lingering { region, .. } = state {
                        state = State::FadingReply { stage: 0, next: Instant::now(), region };
                    }
                }
                qtfb::INPUT_PEN_RELEASE => {
                    stylus_on = false;
                    if pen_down {
                        pen_down = false;
                        user_ink.pen_up();
                        if let State::Listening { ref mut last_pen } = state {
                            *last_pen = Some(Instant::now());
                        }
                    }
                }
                _ => {}
            }
        }

        // ---- basilisk-fang stab (hold the eraser still on one spot) ----
        // One continuous process from first pooling to page-drown: the bleed
        // (a noise-displaced distance field anchored to the paper) creeps
        // while the fang is held, and the SAME bleed accelerates into the
        // death past 3s. Lift before then and the page reabsorbs it.
        let stab_live = pen_down && matches!(state, State::Listening { .. });
        match (stab, eraser_at, stab_live) {
            (None, Some((x, y)), true) => stab = Some((x, y, Instant::now())),
            (Some((sx, sy, t0)), Some((x, y)), true) => {
                let (dx, dy) = (x - sx, y - sy);
                if dx * dx + dy * dy > 20 * 20 {
                    // Drifted: this is erasing, not stabbing.
                    if let Some(bl) = bleed.take() {
                        if !bl.bbox.is_empty() {
                            absorb_region(&mut surf, &disp, bl.bbox);
                        }
                    }
                    stab = Some((x, y, Instant::now()));
                } else {
                    let held = t0.elapsed();
                    if held >= Duration::from_secs(3) {
                        // The fang stays in the wound: the diary bleeds out.
                        // Same bleed, same field — R just stops being gentle.
                        eprintln!("riddle: basilisk fang — the diary's memory is erased");
                        let mut bl = bleed
                            .take()
                            .unwrap_or_else(|| bleed_new(&surf, sx, sy, splat_seed(sx, sy)));
                        let (w, h) = (surf.w as i32, surf.h as i32);
                        let corners = [(0i32, 0i32), (w - 1, 0), (0, h - 1), (w - 1, h - 1)];
                        // Ease into the rush: the creep leans forward over
                        // the first second or so instead of snapping to full
                        // speed, and the full speed itself stays unhurried —
                        // a drowning, not an explosion.
                        let mut frame = 0i32;
                        loop {
                            let mult = 1.0 + (0.025 * frame as f32).min(0.22);
                            let add = (2 + frame).min(14) as f32;
                            let nr = (bl.r * mult + add).min(6000.0);
                            frame += 1;
                            bleed_grow(&mut surf, &mut bl, nr);
                            disp.update(0, 0, w, h, true);
                            let drowned = corners.iter().all(|&(qx, qy)| {
                                let (cdx, cdy) = ((qx - bl.ox) as i64, (qy - bl.oy) as i64);
                                let d2 = (cdx * cdx + cdy * cdy) as u64;
                                let g = bl.field
                                    [((qy / BLEED_CELL) * bl.fw + qx / BLEED_CELL) as usize]
                                    as u64;
                                d2 * 4096 <= (bl.r * bl.r) as u64 * g * g
                            });
                            if drowned {
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(90));
                        }
                        surf.fill_rect(0, 0, surf.w, surf.h, BLACK);
                        disp.update(0, 0, w, h, true);
                        std::thread::sleep(Duration::from_millis(500));
                        if let Some(ref mut s) = store {
                            s.forget_all();
                        }
                        surf.fill_rect(0, 0, surf.w, surf.h, WHITE);
                        disp.full_refresh(surf.w, surf.h);
                        user_ink.clear();
                        ink_dirty = BBox::empty();
                        stab = None;
                        eraser_at = None;
                        state = State::Listening { last_pen: None };
                    } else if held >= Duration::from_millis(800) {
                        // The bleed pools under the held fang — the warning,
                        // and the writer's window to abort by lifting.
                        if bleed.is_none() {
                            bleed = Some(bleed_new(&surf, sx, sy, splat_seed(sx, sy)));
                        }
                        let bl = bleed.as_mut().unwrap();
                        let target = 8.0 + (held.as_millis() as f32 - 800.0) / 2200.0 * 110.0;
                        if target >= bl.r + 2.0 {
                            bleed_grow(&mut surf, bl, target);
                            let (bx, by, bw, bh) = bl.bbox.rect();
                            disp.update(bx, by, bw, bh, true);
                        }
                    }
                }
            }
            (Some(_), _, _) => {
                // Lifted (or the state moved on): the page reabsorbs the
                // bleed. Anything under it goes too — an eraser was pressed
                // there, after all.
                if let Some(bl) = bleed.take() {
                    if !bl.bbox.is_empty() {
                        absorb_region(&mut surf, &disp, bl.bbox);
                    }
                }
                stab = None;
            }
            _ => {}
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
                    if region_all_white(&surf, user_ink.bbox) {
                        // Everything was erased before the pause: nothing to
                        // commit (and no phantom "?" from erased strokes).
                        user_ink.clear();
                        State::Listening { last_pen: None }
                    } else if help::looks_like_question_mark(user_ink.stroke_list()) {
                        // Absorb the "?" and open the guide instead of asking.
                        let (qx, qy, qw, qh) = user_ink.bbox.rect();
                        surf.fill_rect(qx as usize, qy as usize, qw as usize, qh as usize, WHITE);
                        disp.update(qx, qy, qw, qh, false);
                        user_ink.clear();
                        let panel = help::show(&mut surf, &font, takeover);
                        let (px, py, pw, ph) = panel.region.rect();
                        disp.update(px, py, pw, ph, false);
                        eprintln!("riddle: guide shown");
                        State::Help { panel: Some(panel), until: Instant::now() + Duration::from_secs(45) }
                    } else if oracle.is_none() {
                        // No spirit at all: don't eat ink that nothing will
                        // answer — leave the writing and put the reason below.
                        let y = (user_ink.bbox.y1 + 90).min(SCREEN_H as i32 - 400);
                        let plan = plan_reply(&font, &oracle_excuse("no oracle"), Some(y));
                        State::Replying { plan, next: Instant::now(), rx: None }
                    } else {
                        if let Err(e) = user_ink.to_png(&surf, PNG_PATH) {
                            eprintln!("riddle: rasterize failed: {e}");
                        }
                        // Remember this page: strokes now (they're cleared
                        // after the drink), transcript/reply as they stream.
                        turn_id = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        turn_strokes = user_ink.stroke_list().to_vec();
                        turn_reply.clear();
                        turn_transcript = None;
                        turn_failed = false;
                        // Ask NOW: the model streams while the diary drinks the
                        // ink, hiding most of the reply latency in the animation.
                        let (tx, rx) = mpsc::channel();
                        if let Some(ref o) = oracle {
                            o.ask(PNG_PATH, &build_ctx(&store), tx);
                        }
                        // Both backends read the page before ask() returns; the
                        // writer's words don't need to sit on disk afterwards.
                        if std::env::var_os("RIDDLE_KEEP_PAGE").is_none() {
                            let _ = std::fs::remove_file(PNG_PATH);
                        }
                        let region = user_ink.bbox;
                        State::Drinking { stage: 0, next: Instant::now(), region, rx }
                    }
                }
                _ => State::Listening { last_pen },
            },

            State::Drinking { stage, next, region, rx } => {
                const STAGES: u32 = 14;
                if Instant::now() >= next {
                    ink::dissolve_pass(&mut surf, region, stage, STAGES);
                    let (x, y, w, h) = region.rect();
                    disp.update(x, y, w, h, true);
                    if stage + 1 >= STAGES {
                        user_ink.clear();
                        State::Thinking { rx, pulse: Instant::now(), blot_on: false, since: Instant::now() }
                    } else {
                        State::Drinking { stage: stage + 1, next: Instant::now() + Duration::from_millis(70), region, rx }
                    }
                } else {
                    State::Drinking { stage, next, region, rx }
                }
            }

            State::Thinking { rx, pulse, blot_on, since } => match rx.try_recv() {
                Ok(result) => {
                    surf.fill_rect(SCREEN_W / 2 - 14, SCREEN_H / 2 - 14, 28, 28, WHITE);
                    disp.update(SCREEN_W as i32 / 2 - 14, SCREEN_H as i32 / 2 - 14, 28, 28, true);
                    // First streamed event: start writing now; keep the
                    // receiver so the rest of the reply can append itself.
                    match result {
                        Ok(Event::Show(id)) => {
                            // An incantation: the rest of this turn is the
                            // conjured memory, not a reply. (rx drops here.)
                            match conjure(&font, &store, id, &mut surf, &disp) {
                                Some(st) => st,
                                None => {
                                    eprintln!("riddle: memory {id} is missing");
                                    let plan = plan_reply(&font, &oracle_excuse("lost page"), None);
                                    turn_failed = true;
                                    State::Replying { plan, next: Instant::now(), rx: None }
                                }
                            }
                        }
                        Ok(Event::Ink(text)) => {
                            turn_reply.push_str(&text);
                            let plan = plan_reply(&font, &text, None);
                            State::Replying { plan, next: Instant::now(), rx: Some(rx) }
                        }
                        Ok(Event::Transcript(t)) => {
                            // Transcript with no prose (model skipped the
                            // reply): remember the words, keep waiting.
                            turn_transcript = Some(t);
                            State::Thinking { rx, pulse, blot_on, since }
                        }
                        Err(e) => {
                            eprintln!("riddle: oracle failed: {e}");
                            turn_failed = true;
                            let plan = plan_reply(&font, &oracle_excuse(&e), None);
                            State::Replying { plan, next: Instant::now(), rx: None }
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {
                    if since.elapsed() >= ORACLE_PATIENCE {
                        // The oracle never answered (stalled stream, dead pi):
                        // stop pulsing and say so instead of thinking forever.
                        eprintln!("riddle: oracle timed out after {}s", ORACLE_PATIENCE.as_secs());
                        surf.fill_rect(SCREEN_W / 2 - 14, SCREEN_H / 2 - 14, 28, 28, WHITE);
                        disp.update(SCREEN_W as i32 / 2 - 14, SCREEN_H as i32 / 2 - 14, 28, 28, true);
                        let plan = plan_reply(&font, &oracle_excuse("timed out"), None);
                        State::Replying { plan, next: Instant::now(), rx: None }
                    } else if pulse.elapsed() >= Duration::from_millis(600) {
                        let (cx, cy) = (SCREEN_W as i32 / 2, SCREEN_H as i32 / 2);
                        if blot_on {
                            surf.fill_rect(cx as usize - 14, cy as usize - 14, 28, 28, WHITE);
                        } else {
                            surf.stamp(cx, cy, 9, BLACK);
                        }
                        disp.update(cx - 14, cy - 14, 28, 28, true);
                        State::Thinking { rx, pulse: Instant::now(), blot_on: !blot_on, since }
                    } else {
                        State::Thinking { rx, pulse, blot_on, since }
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => State::Listening { last_pen: None },
            },

            State::Replying { mut plan, next, mut rx } => {
                // More of the reply may still be streaming in: append each
                // new chunk below what is already planned, mid-animation.
                if let Some(ref r) = rx {
                    let drop_rx = match r.try_recv() {
                        Ok(Ok(Event::Ink(more))) => {
                            if plan.next_y > SCREEN_H as i32 - 200 {
                                // The page is full: let the rest go unwritten
                                // rather than inking below the visible page.
                                eprintln!("riddle: reply reached the page bottom; trailing text dropped");
                                true
                            } else {
                                turn_reply.push_str(" ");
                                turn_reply.push_str(&more);
                                append_reply(&font, &mut plan, &more);
                                false
                            }
                        }
                        Ok(Ok(Event::Transcript(t))) => {
                            turn_transcript = Some(t);
                            false // the disconnect is still coming
                        }
                        Ok(Ok(Event::Show(_))) => {
                            eprintln!("riddle: conjuring directive mid-reply ignored");
                            false
                        }
                        Ok(Err(e)) => {
                            eprintln!("riddle: oracle failed mid-reply: {e}");
                            turn_failed = true;
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
                        // The turn is complete: the diary remembers it.
                        if !turn_failed && !turn_reply.is_empty() {
                            if let Some(ref mut s) = store {
                                s.append(
                                    turn_id,
                                    turn_transcript.as_deref().unwrap_or(""),
                                    turn_reply.trim(),
                                    &turn_strokes,
                                );
                            }
                        }
                        turn_strokes = Vec::new();
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

            State::Conjuring { mut plan, next, saved } => {
                if stylus_tapped {
                    // The writer interrupts: today's page returns at once.
                    surf.paste_rect(0, 0, SCREEN_W, SCREEN_H, &saved);
                    disp.full_refresh(surf.w, surf.h);
                    State::MemoryShown { saved: None, until: Instant::now(), region: plan.region }
                } else if Instant::now() >= next {
                    // The memory pours back faster than Tom writes: it is
                    // remembered, not composed.
                    let mut dirty = BBox::empty();
                    let mut budget = 48;
                    while budget > 0 && plan.stroke_i < plan.strokes.len() {
                        let stroke = &plan.strokes[plan.stroke_i];
                        if plan.point_i >= stroke.len() {
                            plan.stroke_i += 1;
                            plan.point_i = 0;
                            continue;
                        }
                        let (x, y, r) = stroke[plan.point_i];
                        if plan.point_i > 0 {
                            let (px, py, pr) = stroke[plan.point_i - 1];
                            surf.brush_line(px, py, x, y, r.min(pr + 1), FADED);
                        } else {
                            surf.stamp(x, y, r, FADED);
                        }
                        dirty.add(x, y, r + 2);
                        plan.point_i += 1;
                        budget -= 1;
                    }
                    if !dirty.is_empty() {
                        let (x, y, w, h) = dirty.rect();
                        disp.update(x, y, w, h, true);
                    }
                    if plan.stroke_i >= plan.strokes.len() {
                        let region = plan.region;
                        State::MemoryShown {
                            saved: Some(saved),
                            until: Instant::now() + Duration::from_secs(120),
                            region,
                        }
                    } else {
                        State::Conjuring { plan, next: Instant::now() + Duration::from_millis(10), saved }
                    }
                } else {
                    State::Conjuring { plan, next, saved }
                }
            }

            State::MemoryShown { saved, until, region } => match saved {
                Some(s) => {
                    if stylus_tapped || Instant::now() >= until {
                        // The paper swallows its memory; today's page returns.
                        surf.paste_rect(0, 0, SCREEN_W, SCREEN_H, &s);
                        disp.full_refresh(surf.w, surf.h);
                        eprintln!("riddle: memory dismissed");
                        State::MemoryShown { saved: None, until, region }
                    } else {
                        State::MemoryShown { saved: Some(s), until, region }
                    }
                }
                // Dismissed: swallow the closing touch, listen again on pen-up.
                None if stylus_on => State::MemoryShown { saved: None, until, region },
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

/// True if the region no longer holds any dark pixels (fully erased).
fn region_all_white(surf: &Surface, region: BBox) -> bool {
    if region.is_empty() {
        return true;
    }
    for y in region.y0..=region.y1 {
        for x in region.x0..=region.x1 {
            if surf.luma(x, y) < 200 {
                return false;
            }
        }
    }
    true
}

/// What Tom writes when the spirit cannot answer: short, in a diary's voice,
/// but specific enough to act on. The raw error still goes to stderr.
/// Deterministic per-stab randomness: the splat keeps its shape as it grows.
fn splat_seed(x: i32, y: i32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B1) ^ (y as u32).wrapping_mul(0x85EB_CA6B);
    h ^= h >> 13;
    h = h.wrapping_mul(0xC2B2_AE35);
    h ^ (h >> 16)
}

/// Per-droplet hash stream (stable across redraws).
fn splat_hash(seed: u32, i: u32) -> u32 {
    let mut h = seed.wrapping_add(i.wrapping_mul(0x9E37_79B1));
    h ^= h >> 15;
    h = h.wrapping_mul(0x85EB_CA6B);
    h ^ (h >> 13)
}

/// A single continuous ink bleed, from first pooling to page-drown: a pixel
/// is inked once dist(px, tip) ≤ R · f(px), where f is a multi-octave value-
/// noise field ANCHORED TO THE PAPER. One process, one look: as R grows the
/// front keeps meeting new paper — fingers run where the page drinks fast,
/// bays lag where it doesn't — so growth reads as spreading ink, never as a
/// shape being resized (the field never moves with the stain).
struct Bleed {
    ox: i32,
    oy: i32,
    /// Paper absorbency ≈ f×64 (f ~ 0.55..1.55), sampled every 4px.
    field: Vec<u8>,
    fw: i32,
    r: f32,
    bbox: BBox,
    seed: u32,
}

const BLEED_CELL: i32 = 4;

fn bleed_new(surf: &Surface, ox: i32, oy: i32, seed: u32) -> Bleed {
    let fw = surf.w as i32 / BLEED_CELL + 2;
    let fh = surf.h as i32 / BLEED_CELL + 2;
    // Three octaves of bilinear value noise (~44/100/220px wavelengths):
    // fine feathering on the small pool, long fibres for the page-scale run.
    let mut field = vec![0u8; (fw * fh) as usize];
    let octaves: [(i32, f32); 3] = [(11, 0.35), (25, 0.30), (55, 0.35)];
    let lat = |s: u32, lx: i32, ly: i32| {
        (splat_hash(seed ^ s, (ly * 977 + lx) as u32) % 1000) as f32 / 1000.0
    };
    for y in 0..fh {
        for x in 0..fw {
            let mut n = 0.0f32;
            for (oi, &(wl, amp)) in octaves.iter().enumerate() {
                let (fx, fy) = (x as f32 / wl as f32, y as f32 / wl as f32);
                let (lx, ly) = (fx.floor() as i32, fy.floor() as i32);
                let (tx, ty) = (fx - fx.floor(), fy - fy.floor());
                let s = 0x1000 * (oi as u32 + 1);
                let v = lat(s, lx, ly) * (1.0 - tx) * (1.0 - ty)
                    + lat(s, lx + 1, ly) * tx * (1.0 - ty)
                    + lat(s, lx, ly + 1) * (1.0 - tx) * ty
                    + lat(s, lx + 1, ly + 1) * tx * ty;
                n += v * amp;
            }
            field[(y * fw + x) as usize] = ((0.55 + n) * 64.0) as u8;
        }
    }
    Bleed { ox, oy, field, fw, r: 0.0, bbox: BBox::empty(), seed }
}

/// Grow the bleed to nominal radius `new_r`: ink every newly claimed pixel
/// (d² · 4096 ≤ R² · g², integer-only per pixel), plus the spatter droplets
/// whose appearance thresholds R just crossed — flung ink landing ahead of
/// the front, swallowed by it later.
fn bleed_grow(surf: &mut Surface, b: &mut Bleed, new_r: f32) {
    let old_r = b.r;
    if new_r <= old_r {
        return;
    }
    b.r = new_r;
    let (w, h) = (surf.w as i32, surf.h as i32);
    let reach = (new_r * 1.6) as i32 + 2;
    let y0 = (b.oy - reach).max(0);
    let y1 = (b.oy + reach).min(h - 1);
    let x0 = (b.ox - reach).max(0);
    let x1 = (b.ox + reach).min(w - 1);
    let r2 = (new_r * new_r) as u64;
    let old2 = (old_r * old_r) as u64;
    for y in y0..=y1 {
        let frow = (y / BLEED_CELL) * b.fw;
        for x in x0..=x1 {
            let (dx, dy) = ((x - b.ox) as i64, (y - b.oy) as i64);
            let d2 = (dx * dx + dy * dy) as u64;
            let g = b.field[(frow + x / BLEED_CELL) as usize] as u64;
            let g2 = g * g;
            let lhs = d2 * 4096;
            if lhs <= r2 * g2 && lhs > old2 * g2 {
                surf.put_px(x, y, BLACK);
                b.bbox.add(x, y, 1);
            }
        }
    }
    for i in 0..40u32 {
        let hh = splat_hash(b.seed ^ 0xD09, i);
        let thr = 12.0 + (hh % 300) as f32;
        if !(old_r < thr && thr <= new_r) {
            continue;
        }
        let ang = ((hh >> 9) % 6283) as f32 / 1000.0;
        let dist = thr * (1.25 + ((hh >> 20) % 80) as f32 / 100.0);
        let rr = 2 + ((hh >> 27) % 6) as i32;
        let (px, py) = (b.ox + (dist * ang.cos()) as i32, b.oy + (dist * ang.sin()) as i32);
        if px > -20 && py > -20 && px < w + 20 && py < h + 20 {
            surf.stamp(px, py, rr, BLACK);
            b.bbox.add(px, py, rr + 1);
        }
    }
}

/// Reabsorb warning ink after an aborted stab: dissolve the region in place.
fn absorb_region(surf: &mut Surface, disp: &display::Display, region: BBox) {
    for stage in 0..8 {
        ink::dissolve_pass(surf, region, stage, 8);
        let (x, y, w, h) = region.rect();
        disp.update(x, y, w, h, true);
        std::thread::sleep(Duration::from_millis(45));
    }
}

fn oracle_excuse(e: &str) -> String {
    if e.contains("no oracle") {
        "The diary lies dormant: it found no oracle. \
         Put an API key in oracle.env, then open me again."
            .into()
    } else if e.starts_with("http 401") || e.starts_with("http 403") {
        "The oracle refused the diary's key. Check RIDDLE_OPENAI_KEY in oracle.env.".into()
    } else if e.starts_with("http ") {
        let code = e.split(':').next().unwrap_or("an error");
        format!("The oracle rejected the diary's plea ({code}). Check the model and endpoint in oracle.env.")
    } else if e.contains("request failed") || e.contains("timed out") {
        "The diary cannot reach its oracle. Is the tablet connected to Wi-Fi?".into()
    } else if e.contains("empty reply") {
        "The spirit read your words but said nothing. Write again.".into()
    } else {
        "The ink blurred before it could answer. Write again.".into()
    }
}

/// Summon a remembered page: snapshot today's page, clear the paper, and plan
/// the memory's rewriting — the date in a small hand, the writer's own strokes
/// exactly as they were penned, Tom's old reply beneath — all in faded ink.
fn conjure(
    font: &FontRef,
    store: &Option<memory::MemoryStore>,
    id: u64,
    surf: &mut Surface,
    disp: &display::Display,
) -> Option<State> {
    let s = store.as_ref()?;
    let entry = s.get(id)?.clone();
    let strokes = s.strokes(id).unwrap_or_default();
    eprintln!("riddle: conjuring memory {id} ({})", memory::spoken_date(id));

    let saved = surf.copy_rect(0, 0, SCREEN_W, SCREEN_H);
    surf.fill_rect(0, 0, SCREEN_W, SCREEN_H, WHITE);
    disp.update_all(surf.w, surf.h);

    let mut all: Vec<Vec<(i32, i32, i32)>> = Vec::new();
    let mut region = BBox::empty();

    // The date, small and centered near the top, like a diary heading.
    let date = memory::spoken_date(entry.id);
    let mut raster = script::rasterize_line(font, &date, 54.0);
    script::thin(&mut raster);
    let x0 = (SCREEN_W as i32 - raster.width as i32) / 2;
    let mut ink_bottom = 64;
    for stroke in script::trace(&raster) {
        let mapped: Vec<(i32, i32, i32)> =
            stroke.iter().map(|&(sx, sy)| (x0 + sx, 64 + sy, 1)).collect();
        for &(x, y, r) in &mapped {
            region.add(x, y, r + 2);
            ink_bottom = ink_bottom.max(y);
        }
        all.push(mapped);
    }

    // The writer's own hand, exactly as it was penned.
    for stroke in &strokes {
        for &(x, y, r) in stroke {
            region.add(x, y, r + 2);
            ink_bottom = ink_bottom.max(y);
        }
        all.push(stroke.clone());
    }

    // Tom's old reply, below.
    if !entry.reply.is_empty() {
        let y = (ink_bottom + 130).min(SCREEN_H as i32 - 400);
        let reply = plan_reply(font, &entry.reply, Some(y));
        for stroke in reply.strokes {
            let mapped: Vec<(i32, i32, i32)> = stroke.iter().map(|&(x, y)| (x, y, 2)).collect();
            for &(x, y, r) in &mapped {
                region.add(x, y, r + 2);
            }
            all.push(mapped);
        }
    }

    Some(State::Conjuring {
        plan: ConjurePlan { strokes: all, stroke_i: 0, point_i: 0, region },
        next: Instant::now(),
        saved,
    })
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
