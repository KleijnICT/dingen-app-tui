//! Which tasks and projects are in which list, and the content of every area
//! and project (entities.md section 10, conformance "views").

use crate::model::{Account, Project, Task, When};
use crate::order::compare;
use std::collections::{BTreeMap, HashMap};

/// An entry in Upcoming, with the date it is shown on.
#[derive(Debug, Clone, PartialEq)]
pub struct Dated {
    pub id: String,
    pub date: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TaskLists {
    pub inbox: Vec<String>,
    pub today: Vec<String>,
    pub upcoming: Vec<Dated>,
    pub anytime: Vec<String>,
    pub someday: Vec<String>,
    pub logbook: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProjectLists {
    pub today: Vec<String>,
    pub upcoming: Vec<Dated>,
    pub someday: Vec<String>,
    pub logbook: Vec<String>,
}

fn due(when: &When, deadline: Option<&str>, today: &str) -> bool {
    when.date().is_some_and(|d| d <= today) || deadline.is_some_and(|d| d <= today)
}

/// The date an item is shown on in Upcoming: its date when that is in the
/// future, else its deadline when that is.
fn upcoming_date(when: &When, deadline: Option<&str>, today: &str) -> Option<String> {
    when.date()
        .filter(|d| *d > today)
        .or(deadline.filter(|d| *d > today))
        .map(str::to_string)
}

fn ids<T>(items: Vec<&T>, id: impl Fn(&T) -> &str) -> Vec<String> {
    items.into_iter().map(|t| id(t).to_string()).collect()
}

pub fn task_lists(account: &Account, today: &str) -> TaskLists {
    let projects: HashMap<&str, &Project> = account.projects.iter().map(|p| (p.id.as_str(), p)).collect();
    // A task whose project is completed, canceled or deleted is only in the Logbook.
    let visible = |t: &Task| match t.project_id.as_deref().and_then(|id| projects.get(id)) {
        Some(p) => !p.deleted && p.is_open(),
        None => true,
    };
    let in_someday_project = |t: &Task| {
        t.project_id
            .as_deref()
            .and_then(|id| projects.get(id))
            .is_some_and(|p| p.when == When::Someday)
    };
    let live: Vec<&Task> = account
        .tasks
        .iter()
        .filter(|t| !t.deleted && !t.is_template())
        .collect();
    let open: Vec<&Task> = live
        .iter()
        .copied()
        .filter(|t| t.is_open() && visible(t))
        .collect();
    let is_today = |t: &Task| due(&t.when, t.deadline.as_deref(), today);
    let by_order = |a: &&Task, b: &&Task| compare((&a.order, &a.id), (&b.order, &b.id));

    let mut inbox: Vec<&Task> = open
        .iter()
        .copied()
        .filter(|t| t.project_id.is_none() && t.area_id.is_none())
        .collect();
    inbox.sort_by(by_order);

    let mut todays: Vec<&Task> = open.iter().copied().filter(|t| is_today(t)).collect();
    todays.sort_by(|a, b| compare((a.today_key(), &a.id), (b.today_key(), &b.id)));

    let mut upcoming: Vec<(String, &Task)> = open
        .iter()
        .filter(|t| !is_today(t))
        .filter_map(|t| Some((upcoming_date(&t.when, t.deadline.as_deref(), today)?, *t)))
        .collect();
    upcoming.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| by_order(&a.1, &b.1)));

    let mut anytime: Vec<&Task> = open
        .iter()
        .copied()
        .filter(|t| t.when == When::Anytime && !is_today(t) && !in_someday_project(t))
        .collect();
    anytime.sort_by(by_order);

    let mut someday: Vec<&Task> = open.iter().copied().filter(|t| t.when == When::Someday).collect();
    someday.sort_by(by_order);

    let mut logbook: Vec<&Task> = live.iter().copied().filter(|t| !t.is_open()).collect();
    logbook.sort_by(|a, b| b.completed_at.cmp(&a.completed_at).then_with(|| by_order(a, b)));

    TaskLists {
        inbox: ids(inbox, |t| &t.id),
        today: ids(todays, |t| &t.id),
        upcoming: upcoming
            .into_iter()
            .map(|(date, t)| Dated {
                id: t.id.clone(),
                date,
            })
            .collect(),
        anytime: ids(anytime, |t| &t.id),
        someday: ids(someday, |t| &t.id),
        logbook: ids(logbook, |t| &t.id),
    }
}

pub fn project_lists(account: &Account, today: &str) -> ProjectLists {
    let by_order = |a: &&Project, b: &&Project| compare((&a.order, &a.id), (&b.order, &b.id));
    let mut live: Vec<&Project> = account.projects.iter().filter(|p| !p.deleted).collect();
    live.sort_by(by_order);
    let open: Vec<&Project> = live.iter().copied().filter(|p| p.is_open()).collect();
    let is_today = |p: &Project| due(&p.when, p.deadline.as_deref(), today);

    let mut logbook: Vec<&Project> = live.iter().copied().filter(|p| !p.is_open()).collect();
    logbook.sort_by(|a, b| b.completed_at.cmp(&a.completed_at).then_with(|| by_order(a, b)));

    ProjectLists {
        today: ids(open.iter().copied().filter(|p| is_today(p)).collect(), |p| &p.id),
        upcoming: open
            .iter()
            .filter(|p| !is_today(p))
            .filter_map(|p| {
                Some(Dated {
                    id: p.id.clone(),
                    date: upcoming_date(&p.when, p.deadline.as_deref(), today)?,
                })
            })
            .collect(),
        someday: ids(
            open.iter().copied().filter(|p| p.when == When::Someday).collect(),
            |p| &p.id,
        ),
        logbook: ids(logbook, |p| &p.id),
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Checklist {
    pub items: Vec<String>,
    pub done: usize,
    pub total: usize,
}

/// The sidebar and the content of each container.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Containers {
    /// Live areas, in order.
    pub areas: Vec<String>,
    /// Open projects without an area.
    pub loose_projects: Vec<String>,
    /// Open projects per area.
    pub projects_by_area: BTreeMap<String, Vec<String>>,
    /// Tasks directly in an area: completed ones too, templates not.
    pub tasks_by_area: BTreeMap<String, Vec<String>>,
    pub tasks_by_project: BTreeMap<String, Vec<String>>,
    pub headings_by_project: BTreeMap<String, Vec<String>>,
    /// completed / (open + completed); 0 without tasks.
    pub progress_by_project: BTreeMap<String, f64>,
    pub checklist_by_task: BTreeMap<String, Checklist>,
}

pub fn containers(account: &Account) -> Containers {
    let mut c = Containers::default();

    let mut areas: Vec<_> = account.areas.iter().filter(|a| !a.deleted).collect();
    areas.sort_by(|a, b| compare((&a.order, &a.id), (&b.order, &b.id)));
    c.areas = areas.iter().map(|a| a.id.clone()).collect();

    let mut projects: Vec<&Project> = account
        .projects
        .iter()
        .filter(|p| !p.deleted && p.is_open())
        .collect();
    projects.sort_by(|a, b| compare((&a.order, &a.id), (&b.order, &b.id)));
    let mut tasks: Vec<&Task> = account
        .tasks
        .iter()
        .filter(|t| !t.deleted && !t.is_template())
        .collect();
    tasks.sort_by(|a, b| compare((&a.order, &a.id), (&b.order, &b.id)));
    let mut headings: Vec<_> = account.headings.iter().filter(|h| !h.deleted).collect();
    headings.sort_by(|a, b| compare((&a.order, &a.id), (&b.order, &b.id)));

    for area in &account.areas {
        c.projects_by_area.insert(area.id.clone(), vec![]);
        c.tasks_by_area.insert(area.id.clone(), vec![]);
    }
    for project in &account.projects {
        c.tasks_by_project.insert(project.id.clone(), vec![]);
        c.headings_by_project.insert(project.id.clone(), vec![]);
        c.progress_by_project.insert(project.id.clone(), 0.0);
    }
    for project in projects {
        match project.area_id.as_deref() {
            None => c.loose_projects.push(project.id.clone()),
            Some(area) => {
                if let Some(list) = c.projects_by_area.get_mut(area) {
                    list.push(project.id.clone());
                }
            }
        }
    }
    let mut counts: HashMap<&str, (usize, usize)> = HashMap::new();
    for task in tasks {
        if let Some(project) = task.project_id.as_deref() {
            if let Some(list) = c.tasks_by_project.get_mut(project) {
                list.push(task.id.clone());
            }
            let count = counts.entry(project).or_default();
            match task.status.as_str() {
                "open" => count.0 += 1,
                "completed" => count.1 += 1,
                _ => {}
            }
        } else if let Some(list) = task.area_id.as_deref().and_then(|a| c.tasks_by_area.get_mut(a)) {
            list.push(task.id.clone());
        }
    }
    for (project, (open, completed)) in counts {
        if let Some(progress) = c
            .progress_by_project
            .get_mut(project)
            .filter(|_| open + completed > 0)
        {
            *progress = completed as f64 / (open + completed) as f64;
        }
    }
    for heading in headings {
        if let Some(list) = c.headings_by_project.get_mut(&heading.project_id) {
            list.push(heading.id.clone());
        }
    }
    for task in &account.tasks {
        let items = account.checklist_of(&task.id);
        c.checklist_by_task.insert(
            task.id.clone(),
            Checklist {
                done: items.iter().filter(|i| i.done).count(),
                total: items.len(),
                items: items.iter().map(|i| i.id.clone()).collect(),
            },
        );
    }
    c
}
