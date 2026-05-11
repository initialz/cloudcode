//! Splash screen shown once on `cloudcode` startup, right before the menu.

use crate::input::KeyRx;
use anyhow::Result;
use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Terminal;
use std::io::stdout;

pub async fn show(keys: &mut KeyRx, account: &str) -> Result<()> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(out);
    let mut term = Terminal::new(backend)?;
    let res = render(&mut term, keys, account).await;
    disable_raw_mode().ok();
    execute!(term.backend_mut(), LeaveAlternateScreen).ok();
    term.show_cursor().ok();
    res
}

async fn render<B: Backend>(term: &mut Terminal<B>, keys: &mut KeyRx, account: &str) -> Result<()> {
    let account = account.to_string();
    term.draw(|f| {
        let area = f.area();
        let w = 54u16.min(area.width.saturating_sub(4));
        let h = 16u16.min(area.height);
        let rect = centered_rect(area, w, h);
        f.render_widget(Clear, rect);

        let cyan_bold = Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD);
        let cyan_soft = Style::default().fg(Color::LightCyan);
        let pink = Style::default().fg(Color::LightMagenta);
        let dim = Style::default().fg(Color::DarkGray);
        let star = Style::default().fg(Color::Yellow);

        let lines: Vec<Line> = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("·  ", dim),
                Span::styled("☁", cyan_soft),
                Span::raw("    "),
                Span::styled("✦", star),
                Span::raw("    "),
                Span::styled("☁", cyan_soft),
                Span::styled("  ·", dim),
            ])
            .alignment(Alignment::Center),
            Line::from(""),
            // Wide-spaced bold "C L O U D C O D E"
            Line::from(Span::styled("C  L  O  U  D  C  O  D  E", cyan_bold))
                .alignment(Alignment::Center),
            Line::from(""),
            Line::from(Span::styled("──────────────────────", cyan_soft))
                .alignment(Alignment::Center),
            Line::from(""),
            Line::from(Span::styled("remote claude · your terminal", dim))
                .alignment(Alignment::Center),
            Line::from(""),
            Line::from(""),
            Line::from(vec![
                Span::styled("Hi ", pink),
                Span::styled(&account, pink.add_modifier(Modifier::BOLD)),
                Span::styled("  ✨", star),
            ])
            .alignment(Alignment::Center),
            Line::from(""),
            Line::from(Span::styled(
                "press any key to start",
                dim.add_modifier(Modifier::DIM),
            ))
            .alignment(Alignment::Center),
        ];

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(" ☁ cloudcode ", cyan_bold))
            .title_alignment(Alignment::Center);

        let para = Paragraph::new(lines).block(block);
        f.render_widget(para, rect);
    })?;
    let _ = keys.recv().await;
    Ok(())
}

fn centered_rect(area: Rect, w: u16, h: u16) -> Rect {
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}
