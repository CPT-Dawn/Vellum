//! `vellum monitors`: the connected Monitors, their sizes and scale.

use vellum_ipc::MonitorInfo;

use crate::client;

pub fn run() -> Result<(), String> {
    let monitors = client::monitors(&client::socket()?)?;
    print!("{}", table(&monitors));
    Ok(())
}

/// Monitors as left-aligned columns under a header.
fn table(monitors: &[MonitorInfo]) -> String {
    let header = ["MONITOR", "LOGICAL", "PHYSICAL", "SCALE"].map(String::from);
    let rows: Vec<[String; 4]> = monitors
        .iter()
        .map(|m| {
            [
                m.name.clone(),
                format!("{}x{}", m.logical_width, m.logical_height),
                format!("{}x{}", m.physical_width, m.physical_height),
                scale(m.scale_120),
            ]
        })
        .collect();
    let mut widths = header.clone().map(|h| h.len());
    for row in &rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.len());
        }
    }
    let mut out = String::new();
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

/// A scale in 120ths as a short decimal: 120 is "1", 150 is "1.25".
fn scale(scale_120: u32) -> String {
    let text = format!("{:.3}", f64::from(scale_120) / 120.0);
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scales_print_as_short_decimals() {
        assert_eq!(scale(120), "1");
        assert_eq!(scale(150), "1.25");
        assert_eq!(scale(180), "1.5");
        assert_eq!(scale(240), "2");
        assert_eq!(scale(140), "1.167");
    }

    #[test]
    fn the_table_aligns_columns_under_the_header() {
        let monitor = |name: &str, scale_120| MonitorInfo {
            name: name.into(),
            logical_width: 200,
            logical_height: 160,
            physical_width: 300,
            physical_height: 240,
            scale_120,
        };
        let table = table(&[monitor("DP-1", 180), monitor("HDMI-A-1", 120)]);
        assert_eq!(
            table,
            "MONITOR   LOGICAL  PHYSICAL  SCALE\n\
             DP-1      200x160  300x240   1.5\n\
             HDMI-A-1  200x160  300x240   1\n"
        );
    }
}
