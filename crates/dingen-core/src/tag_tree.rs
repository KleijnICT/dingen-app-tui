//! Tags as a tree (conformance "tag-tree"): the order of the tree, which
//! tags can be a parent, whether a name can be used, open tasks per tag, and
//! deleting a tag.

use crate::actions::Write;
use crate::misc;
use crate::model::{Project, Tag, Task};
use crate::order::compare;
use serde_json::json;
use std::collections::{HashMap, HashSet};

/// The space of each tag id; `None` (or absent) when not known yet, which
/// counts as every space.
pub type Spaces = HashMap<String, Option<String>>;

/// The live tags in tree order, with their depth: each tag followed by its
/// children, siblings by order then id. A tag whose parent is deleted or
/// missing is at the top; tags in a loop come after the rest, each at the top.
pub fn tree(tags: &[Tag]) -> Vec<(String, usize)> {
    let mut live: Vec<&Tag> = tags.iter().filter(|t| !t.deleted).collect();
    live.sort_by(|a, b| compare((&a.order, &a.id), (&b.order, &b.id)));
    let ids: HashSet<String> = live.iter().map(|t| t.id.clone()).collect();
    // Each live tag with its live parent; in sibling order.
    let nodes: Vec<(String, Option<String>)> = live
        .iter()
        .map(|t| (t.id.clone(), t.parent_id.clone().filter(|p| ids.contains(p))))
        .collect();
    fn walk(
        nodes: &[(String, Option<String>)],
        id: &str,
        depth: usize,
        placed: &mut HashSet<String>,
        out: &mut Vec<(String, usize)>,
    ) {
        if !placed.insert(id.to_string()) {
            return;
        }
        out.push((id.to_string(), depth));
        for (child, _) in nodes.iter().filter(|(_, p)| p.as_deref() == Some(id)) {
            walk(nodes, child, depth + 1, placed, out);
        }
    }
    let mut out = vec![];
    let mut placed = HashSet::new();
    for (id, _) in nodes.iter().filter(|(_, p)| p.is_none()) {
        walk(&nodes, id, 0, &mut placed, &mut out);
    }
    // What is left hangs in a loop of parents.
    for (id, _) in &nodes {
        walk(&nodes, id, 0, &mut placed, &mut out);
    }
    out
}

/// The tag and every tag below it.
pub fn descendants(tags: &[Tag], id: &str) -> HashSet<String> {
    let mut out: HashSet<String> = [id.to_string()].into();
    loop {
        let more: Vec<String> = tags
            .iter()
            .filter(|t| !t.deleted && !out.contains(&t.id))
            .filter(|t| t.parent_id.as_ref().is_some_and(|p| out.contains(p)))
            .map(|t| t.id.clone())
            .collect();
        if more.is_empty() {
            return out;
        }
        out.extend(more);
    }
}

fn same_space(spaces: &Spaces, a: &str, b: &str) -> bool {
    match (spaces.get(a).cloned().flatten(), spaces.get(b).cloned().flatten()) {
        (Some(x), Some(y)) => x == y,
        _ => true,
    }
}

/// The tree without the tag, its descendants and tags in another space.
pub fn valid_parents(tags: &[Tag], tag: &str, spaces: &Spaces) -> Vec<(String, usize)> {
    let blocked = descendants(tags, tag);
    let allowed: Vec<Tag> = tags
        .iter()
        .filter(|t| !blocked.contains(&t.id) && same_space(spaces, &t.id, tag))
        .cloned()
        .collect();
    tree(&allowed)
}

/// Why a name cannot be used: `empty`, or `taken` by another live tag (for a
/// renamed tag in the same space, for a new tag anywhere).
pub fn name_issue(tags: &[Tag], tag: Option<&str>, typed: &str, spaces: &Spaces) -> Option<&'static str> {
    let title = misc::normalize_tag(typed);
    if title.is_empty() {
        return Some("empty");
    }
    let clash = tags.iter().any(|other| {
        !other.deleted
            && Some(other.id.as_str()) != tag
            && misc::same_tag(&other.title, &title)
            && tag.is_none_or(|t| same_space(spaces, &other.id, t))
    });
    clash.then_some("taken")
}

/// Open tasks per tag id (a tag listed twice on a task counts once).
pub fn open_task_counts(tasks: &[Task]) -> HashMap<String, usize> {
    let mut out = HashMap::new();
    for task in tasks
        .iter()
        .filter(|t| !t.deleted && !t.is_template() && t.is_open())
    {
        for id in task.tag_ids.iter().collect::<HashSet<_>>() {
            *out.entry(id.clone()).or_default() += 1;
        }
    }
    out
}

/// Deleting a tag: it comes off every task and project that has it, its
/// live children move to the top, then the tag is deleted.
pub fn delete_tag(tasks: &[Task], projects: &[Project], tags: &[Tag], tag: &str) -> Vec<Write> {
    let change = |entity_type: &str, id: &str, fields: serde_json::Value| Write::Change {
        entity_type: entity_type.into(),
        entity_id: id.into(),
        fields: fields.as_object().cloned().unwrap_or_default(),
    };
    let mut out = vec![];
    for task in tasks.iter().filter(|t| t.tag_ids.iter().any(|x| x == tag)) {
        let ids: Vec<&String> = task.tag_ids.iter().filter(|x| *x != tag).collect();
        out.push(change("task", &task.id, json!({ "tagIds": ids })));
    }
    for project in projects.iter().filter(|p| p.tag_ids.iter().any(|x| x == tag)) {
        let ids: Vec<&String> = project.tag_ids.iter().filter(|x| *x != tag).collect();
        out.push(change("project", &project.id, json!({ "tagIds": ids })));
    }
    for child in tags
        .iter()
        .filter(|t| !t.deleted && t.parent_id.as_deref() == Some(tag))
    {
        out.push(change("tag", &child.id, json!({ "parentId": null })));
    }
    out.push(change("tag", tag, json!({ "deleted": true })));
    out
}
