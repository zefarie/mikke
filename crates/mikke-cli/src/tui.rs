//! Recherche interactive plein écran, dans l'esprit de fzf : on tape, la
//! liste se met à jour à chaque frappe (la recherche coûte ~1 ms), Entrée
//! ouvre le fichier sélectionné avec xdg-open et rend la main.

use std::path::Path;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use mikke_core::{Embedder, SearchHit, Searcher};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};

/// Le vermillon du logo (256 couleurs : orange 208).
const ACCENT: Color = Color::Indexed(208);
const TOP: usize = 20;

pub fn run(index_dir: &Path, embedder: Option<Embedder>) -> Result<()> {
    // tout est ouvert UNE fois : la boucle ne fait plus que des recherches
    let searcher = Searcher::open(index_dir)?;
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &searcher, embedder.as_ref());
    ratatui::restore();
    if let Ok(Some(path)) = &result {
        open_file(path);
        println!("{path}");
    }
    result.map(|_| ())
}

fn search(searcher: &Searcher, embedder: Option<&Embedder>, query: &str) -> Vec<SearchHit> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    searcher.search(query, TOP, embedder).unwrap_or_default()
}

/// Retourne le chemin à ouvrir (Entrée) ou None (Échap).
fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    searcher: &Searcher,
    embedder: Option<&Embedder>,
) -> Result<Option<String>> {
    let mut query = String::new();
    let mut hits: Vec<SearchHit> = Vec::new();
    let mut selected: usize = 0;
    let mut list_state = ListState::default();

    loop {
        list_state.select((!hits.is_empty()).then_some(selected));
        terminal.draw(|f| draw(f, &query, &hits, &mut list_state))?;

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        let mut refresh = false;
        match key.code {
            KeyCode::Esc => return Ok(None),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(None);
            }
            KeyCode::Enter => {
                if let Some(hit) = hits.get(selected) {
                    return Ok(Some(hit.path.clone()));
                }
            }
            KeyCode::Up => selected = selected.saturating_sub(1),
            KeyCode::Down => selected = (selected + 1).min(hits.len().saturating_sub(1)),
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                query.clear();
                refresh = true;
            }
            KeyCode::Backspace => {
                query.pop();
                refresh = true;
            }
            KeyCode::Char(c) => {
                query.push(c);
                refresh = true;
            }
            _ => {}
        }
        if refresh {
            hits = search(searcher, embedder, &query);
            selected = 0;
        }
    }
}

fn draw(frame: &mut Frame, query: &str, hits: &[SearchHit], state: &mut ListState) {
    let [input_area, list_area, preview_area, help_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(7),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    let prompt = Line::from(vec![
        Span::styled(
            "mikke ❯ ",
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::raw(query),
        Span::styled("▏", Style::new().add_modifier(Modifier::DIM)),
    ]);
    frame.render_widget(Paragraph::new(prompt), input_area);

    let items: Vec<ListItem> = hits
        .iter()
        .map(|hit| {
            ListItem::new(Line::from(vec![
                Span::raw(shorten_home(&hit.path)),
                Span::styled(
                    format!("  {:.3}", hit.score),
                    Style::new().add_modifier(Modifier::DIM),
                ),
            ]))
        })
        .collect();
    let list = List::new(items)
        .highlight_style(Style::new().fg(ACCENT).add_modifier(Modifier::BOLD))
        .highlight_symbol("▌ ");
    frame.render_stateful_widget(list, list_area, state);

    let preview = state
        .selected()
        .and_then(|i| hits.get(i))
        .map(snippet_line)
        .unwrap_or_else(|| {
            Line::from(Span::styled(
                if query.trim().is_empty() {
                    "describe the file you're looking for…"
                } else {
                    "nothing found"
                },
                Style::new().add_modifier(Modifier::DIM),
            ))
        });
    frame.render_widget(
        Paragraph::new(preview)
            .wrap(Wrap { trim: true })
            .block(Block::new().borders(Borders::TOP)),
        preview_area,
    );

    let help = Line::from(Span::styled(
        format!(
            "{} result(s) — ↑↓ navigate · Enter open · Esc quit",
            hits.len()
        ),
        Style::new().add_modifier(Modifier::DIM),
    ));
    frame.render_widget(Paragraph::new(help), help_area);
}

/// L'extrait du chunk, termes de la requête en vermillon.
fn snippet_line(hit: &SearchHit) -> Line<'_> {
    let text = &hit.snippet;
    let mut ranges: Vec<_> = hit.highlights.clone();
    ranges.sort_by_key(|r| r.start);
    let mut spans = Vec::new();
    let mut cursor = 0;
    let clean = |s: &str| s.replace(['\n', '\t'], " ");
    for r in ranges {
        if r.start < cursor || r.end > text.len() {
            continue;
        }
        if r.start > cursor {
            spans.push(Span::raw(clean(&text[cursor..r.start])));
        }
        spans.push(Span::styled(
            clean(&text[r.start..r.end]),
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        ));
        cursor = r.end;
    }
    spans.push(Span::raw(clean(&text[cursor..])));
    Line::from(spans)
}

fn open_file(path: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(opener)
        .arg(path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn shorten_home(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if path.starts_with(&home) => format!("~{}", &path[home.len()..]),
        _ => path.to_string(),
    }
}
