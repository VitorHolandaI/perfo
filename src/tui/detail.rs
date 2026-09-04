use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::data::cpu::ProcessInfo;
use crate::theme::Theme;

use super::cpu::{self, Pane, Ui};

const MEMORY_WARN_PCT: f64 = 5.0;
const MEMORY_HOT_PCT: f64 = 10.0;

pub(super) fn draw_mem(frame: &mut Frame, area: Rect, ui: &Ui) {
    let outer = cpu::block("4:MEM", ui.pane == Pane::Mem, &ui.theme);
    frame.render_widget(outer.clone(), area);
    let inner = outer.inner(area);
    let [summary, processes] =
        Layout::vertical([Constraint::Length(9), Constraint::Min(0)]).areas(inner);
    let [ram, pressure] =
        Layout::horizontal([Constraint::Percentage(52), Constraint::Percentage(48)]).areas(summary);
    draw_mem_summary(frame, ram, ui);
    draw_mem_pressure(frame, pressure, ui);
    draw_mem_processes(frame, processes, ui);
}

fn draw_mem_summary(frame: &mut Frame, area: Rect, ui: &Ui) {
    let m = &ui.snap.mem;
    let total = m.total.max(1);
    let used_pct = m.used as f64 / total as f64 * 100.0;
    let bar_width = area.width.saturating_sub(14) as usize;
    let lines = vec![
        Line::from(vec![
            Span::styled("RAM ", Style::default().fg(ui.theme.muted)),
            Span::styled(
                cpu::bar(used_pct as f32, bar_width),
                Style::default().fg(memory_color(used_pct, &ui.theme)),
            ),
            Span::styled(
                format!(" {:>5.1}%", used_pct),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(format!(
            "used {:>8} / {:<8}",
            cpu::human_bytes(m.used),
            cpu::human_bytes(m.total)
        )),
        Line::from(format!("available {:>8}", cpu::human_bytes(m.available))),
        Line::from(format!(
            "cache {:>12}  buffers {:>8}",
            cpu::human_bytes(m.cache),
            cpu::human_bytes(m.buffers)
        )),
        Line::from(format!("free  {:>12}", cpu::human_bytes(m.free))),
    ];
    render_panel(frame, area, "RAM", lines, &ui.theme);
}

fn draw_mem_pressure(frame: &mut Frame, area: Rect, ui: &Ui) {
    let m = &ui.snap.mem;
    let swap_pct = if m.swap_total == 0 {
        0.0
    } else {
        m.swap_used as f64 / m.swap_total as f64 * 100.0
    };
    let psi = format!(
        "{:.1} / {:.1} / {:.1}%",
        m.psi_some_10, m.psi_some_60, m.psi_some_300
    );
    let swap = if m.swap_total == 0 {
        "off".to_string()
    } else {
        format!(
            "{:.1}% {} / {}",
            swap_pct,
            cpu::short_bytes(m.swap_used),
            cpu::short_bytes(m.swap_total)
        )
    };
    let lines = vec![
        Line::from(vec![
            Span::styled("PSI some ", Style::default().fg(ui.theme.muted)),
            Span::styled(
                psi,
                Style::default().fg(memory_color(m.psi_some_10, &ui.theme)),
            ),
        ]),
        Line::from("10s / 60s / 300s"),
        Line::from(format!("swap {:>24}", swap)),
        Line::from(""),
        Line::from(Span::styled(
            "higher PSI means processes are waiting for RAM",
            Style::default().fg(ui.theme.muted),
        )),
    ];
    render_panel(frame, area, "PRESSURE / SWAP", lines, &ui.theme);
}

fn draw_mem_processes(frame: &mut Frame, area: Rect, ui: &Ui) {
    let panel = cpu::block("TOP MEMORY PROCESSES", false, &ui.theme);
    frame.render_widget(panel.clone(), area);
    let inner = panel.inner(area);
    let mut processes: Vec<&ProcessInfo> = ui
        .snap
        .processes
        .iter()
        .filter(|p| p.owner.is_none() && !p.is_kernel)
        .collect();
    processes.sort_by_key(|p| std::cmp::Reverse(p.mem_bytes));
    let mut lines = vec![Line::from(Span::styled(
        "     PID USER             MEM       % COMMAND",
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    for p in processes
        .iter()
        .take(inner.height.saturating_sub(2) as usize)
    {
        let pct = p.mem_bytes as f64 / ui.snap.mem.total.max(1) as f64 * 100.0;
        lines.push(Line::from(format!(
            "{:>8} {:<12} {:>8} {:>6.1}% {}",
            p.pid,
            cpu::truncate(&p.user, 12),
            cpu::human_bytes(p.mem_bytes),
            pct,
            cpu::truncate(&p.cmd, inner.width.saturating_sub(43) as usize)
        )));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

pub(super) fn draw_disks(frame: &mut Frame, area: Rect, ui: &Ui) {
    let outer = cpu::block("5:DISKS", ui.pane == Pane::Disks, &ui.theme);
    frame.render_widget(outer.clone(), area);
    let inner = outer.inner(area);
    let [usage, devices] =
        Layout::vertical([Constraint::Length(10), Constraint::Min(0)]).areas(inner);
    draw_disk_usage(frame, usage, ui);
    draw_disk_devices(frame, devices, ui);
}

fn draw_disk_usage(frame: &mut Frame, area: Rect, ui: &Ui) {
    let mut lines = vec![Line::from(Span::styled(
        "DEVICE     MOUNT        FS       USED/TOTAL       FREE     USE",
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    for d in ui
        .snap
        .disks
        .iter()
        .take(area.height.saturating_sub(2) as usize)
    {
        let name = cpu::truncate(d.name.rsplit('/').next().unwrap_or(&d.name), 10);
        let mount = cpu::truncate(&d.mount, 12);
        let fs = cpu::truncate(&d.fs, 8);
        let color = disk_color(d.percent, &ui.theme);
        lines.push(Line::from(vec![
            Span::styled(
                format!(
                    "{name:<10} {mount:<12} {fs:<8} {:>6}/{:<6} {:>8} ",
                    cpu::short_bytes(d.used_bytes),
                    cpu::short_bytes(d.total_bytes),
                    cpu::short_bytes(d.available_bytes)
                ),
                Style::default().fg(ui.theme.fg),
            ),
            Span::styled(format!("{:>5.1}%", d.percent), Style::default().fg(color)),
        ]));
    }
    render_panel(frame, area, "SPACE BY MOUNT", lines, &ui.theme);
}

fn draw_disk_devices(frame: &mut Frame, area: Rect, ui: &Ui) {
    let panel = cpu::block("DEVICE DETAILS", false, &ui.theme);
    frame.render_widget(panel.clone(), area);
    let inner = panel.inner(area);
    let mut lines = vec![Line::from(Span::styled(
        "DEVICE       TEMP       READ/s      WRITE/s       READ TOTAL      WRITE TOTAL",
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    for d in cpu::unique_disks(&ui.snap.disks)
        .into_iter()
        .take(inner.height.saturating_sub(2) as usize)
    {
        let name = cpu::truncate(d.name.rsplit('/').next().unwrap_or(&d.name), 12);
        let temp = d
            .temp_c
            .map(|t| format!("{t:>4.0}C"))
            .unwrap_or_else(|| "   -".to_string());
        lines.push(Line::from(vec![
            Span::styled(format!("{name:<12} "), Style::default().fg(ui.theme.fg)),
            Span::styled(
                format!("{temp:>6}  "),
                Style::default().fg(cpu::temp_color(d.temp_c, &ui.theme)),
            ),
            Span::styled(
                format!(
                    "{:>9}/s  {:>9}/s  ",
                    cpu::short_bytes(d.read_bps),
                    cpu::short_bytes(d.write_bps)
                ),
                Style::default().fg(ui.theme.accent),
            ),
            Span::styled(
                format!(
                    "{:>12}  {:>12}",
                    cpu::short_bytes(d.total_read_bytes),
                    cpu::short_bytes(d.total_written_bytes)
                ),
                Style::default().fg(ui.theme.muted),
            ),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_panel(frame: &mut Frame, area: Rect, title: &str, lines: Vec<Line>, theme: &Theme) {
    let panel = cpu::block(title, false, theme);
    frame.render_widget(panel.clone(), area);
    frame.render_widget(Paragraph::new(lines), panel.inner(area));
}

fn memory_color(value: f64, theme: &Theme) -> Color {
    if value >= MEMORY_HOT_PCT {
        theme.red
    } else if value >= MEMORY_WARN_PCT {
        theme.yellow
    } else {
        theme.green
    }
}

fn disk_color(value: f32, theme: &Theme) -> Color {
    if value >= 85.0 {
        theme.red
    } else if value >= 70.0 {
        theme.yellow
    } else {
        theme.green
    }
}
