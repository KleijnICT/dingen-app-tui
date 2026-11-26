//! Search (conformance "search"): areas, open projects and open tasks whose
//! title (or a task's notes) holds every word (repeat templates count as
//! open tasks, as the conformance set has it), after Unicode NFD, removing
//! diacritics and lowercasing. Areas, then projects, then tasks matching on
//! the title, then tasks matching on the notes; each in the input order.

use crate::model::Account;
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, PartialEq)]
pub enum Hit {
    Area(String),
    Project(String),
    Task(String),
}

impl Hit {
    pub fn kind(&self) -> &'static str {
        match self {
            Hit::Area(_) => "area",
            Hit::Project(_) => "project",
            Hit::Task(_) => "task",
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Hit::Area(id) | Hit::Project(id) | Hit::Task(id) => id,
        }
    }
}

/// Lower case without diacritics: "Café" is "cafe".
pub fn fold(text: &str) -> String {
    text.nfd()
        .filter(|c| !is_combining_mark(*c))
        .collect::<String>()
        .to_lowercase()
}

pub fn search(account: &Account, query: &str, limit: usize) -> Vec<Hit> {
    let words: Vec<String> = fold(query).split_whitespace().map(str::to_string).collect();
    if words.is_empty() {
        return vec![];
    }
    let has = |text: &str| {
        let folded = fold(text);
        words.iter().all(|w| folded.contains(w.as_str()))
    };
    let mut hits: Vec<Hit> = vec![];
    hits.extend(
        account
            .areas
            .iter()
            .filter(|a| !a.deleted && has(&a.title))
            .map(|a| Hit::Area(a.id.clone())),
    );
    hits.extend(
        account
            .projects
            .iter()
            .filter(|p| !p.deleted && p.is_open() && has(&p.title))
            .map(|p| Hit::Project(p.id.clone())),
    );
    let open: Vec<_> = account
        .tasks
        .iter()
        .filter(|t| !t.deleted && t.is_open())
        .collect();
    hits.extend(
        open.iter()
            .filter(|t| has(&t.title))
            .map(|t| Hit::Task(t.id.clone())),
    );
    hits.extend(
        open.iter()
            .filter(|t| !has(&t.title) && has(&format!("{} {}", t.title, t.notes)))
            .map(|t| Hit::Task(t.id.clone())),
    );
    hits.truncate(limit);
    hits
}
