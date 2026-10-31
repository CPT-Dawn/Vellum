//! `vellum status`: the Daemon, its memory use, and what each Monitor shows.

use vellum_ipc::{Status, Transition, TransitionKind};

use crate::client;
use crate::config::Named;

pub fn run() -> Result<(), String> {
    let status = client::status(&client::daemon()?)?;
    print!("{}", report(&status));
    Ok(())
}

fn report(status: &Status) -> String {
    let mut out = format!(
        "vellumd {} (pid {}), {} resident\ntransition: {}\n\n",
        status.version,
        status.pid,
        memory(status.rss_bytes),
        transition(&status.settings.transition)
    );
    let rows: Vec<[&str; 3]> = status
        .monitors
        .iter()
        .map(|m| match &m.wallpaper {
            Some(w) => [m.name.as_str(), w.fit.name(), w.path.as_str()],
            None => [m.name.as_str(), "-", "-"],
        })
        .collect();
    let header = ["MONITOR", "FIT", "WALLPAPER"];
    let mut widths = header.map(str::len);
    for row in &rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.len());
        }
    }
    for row in std::iter::once(&header).chain(&rows) {
        let line: Vec<String> = row
            .iter()
            .zip(widths)
            .map(|(cell, width)| format!("{cell:<width$}"))
            .collect();
        out += line.join("  ").trim_end();
        out.push('\n');
    }
    out
}

/// The default Transition in Config words: "wipe up, 250 ms, linear".
fn transition(t: &Transition) -> String {
    match t.kind {
        TransitionKind::None => "none".into(),
        TransitionKind::Fade => format!("fade, {} ms, {}", t.duration_ms, t.easing.name()),
        TransitionKind::Wipe | TransitionKind::Slide => format!(
            "{} {}, {} ms, {}",
            t.kind.name(),
            t.direction.name(),
            t.duration_ms,
            t.easing.name()
        ),
    }
}

/// Bytes as a short human size: "812 KB", "2.4 MB".
pub fn memory(bytes: u64) -> String {
    const MB: u64 = 1024 * 1024;
    if bytes < MB {
        format!("{} KB", bytes.div_ceil(1024))
    } else {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vellum_ipc::{
        Direction, Easing, FitMode, MonitorStatus, Settings, Transition, TransitionKind, Wallpaper,
    };

    #[test]
    fn the_report_names_the_daemon_and_each_monitor() {
        let status = Status {
            version: "2.0.0".into(),
            pid: 42,
            rss_bytes: 2_516_582,
            monitors: vec![
                MonitorStatus {
                    name: "DP-1".into(),
                    wallpaper: Some(Wallpaper {
                        path: "/w/dunes.jpg".into(),
                        fit: FitMode::Center,
                    }),
                },
                MonitorStatus {
                    name: "HDMI-A-1".into(),
                    wallpaper: None,
                },
            ],
            settings: Settings::default(),
        };
        assert_eq!(
            report(&status),
            "vellumd 2.0.0 (pid 42), 2.4 MB resident\n\
             transition: fade, 600 ms, ease-out\n\n\
             MONITOR   FIT     WALLPAPER\n\
             DP-1      center  /w/dunes.jpg\n\
             HDMI-A-1  -       -\n"
        );
    }

    #[test]
    fn the_transition_line_names_a_direction_only_where_it_matters() {
        let slide = Transition {
            kind: TransitionKind::Slide,
            direction: Direction::Up,
            duration_ms: 250,
            easing: Easing::Linear,
        };
        assert_eq!(transition(&slide), "slide up, 250 ms, linear");
        let none = Transition {
            kind: TransitionKind::None,
            ..slide
        };
        assert_eq!(transition(&none), "none");
    }

    #[test]
    fn small_memory_prints_in_kilobytes() {
        assert_eq!(memory(812 * 1024), "812 KB");
        assert_eq!(memory(1024 * 1024), "1.0 MB");
    }
}
