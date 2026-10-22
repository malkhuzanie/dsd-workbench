//! Comparing the tags of a file on disk with what a plan says they should be. Used by
//! verification and by tag-only mode, which is why it lives in neither.

use std::path::Path;

use crate::plan::{SplitPlan, TrackPlan};
use crate::snapshot::{self, Loaded};

/// One tag that differs between a file and the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagChange {
    /// The field's name in lower case (`comment`, `track`), or `cover`.
    pub field: String,
    /// What the file has now (empty for nothing).
    pub have: String,
    /// What the plan says (empty for nothing).
    pub wanted: String,
}

impl TagChange {
    /// `comment: nothing → 'text'`, for a tooltip or a log line.
    pub fn describe(&self) -> String {
        let show = |s: &str| {
            if s.is_empty() {
                "nothing".to_owned()
            } else {
                format!("'{s}'")
            }
        };
        format!(
            "{}: {} → {}",
            self.field,
            show(&self.have),
            show(&self.wanted)
        )
    }
}

/// The tags that would change if `plan`'s tags were written to the file at `target`. A file
/// with no tag yet counts as having every field empty; an unreadable or missing file has no
/// changes to report.
pub fn tag_changes(plan: &SplitPlan, track: &TrackPlan, target: &Path) -> Vec<TagChange> {
    let empty = snapshot::TagSnapshot::default();
    let found = match snapshot::load(target) {
        Loaded::Tags(found) => found,
        Loaded::Untagged => empty,
        Loaded::Failed(_) | Loaded::Missing => return Vec::new(),
    };
    let mut changes: Vec<TagChange> = plan
        .tag_keys(track)
        .into_iter()
        .filter_map(|key| {
            let wanted = plan.album.resolve(&track.meta, &key);
            let wanted = if wanted.trim().is_empty() { "" } else { wanted };
            let have = found.value(&key);
            (have != wanted).then(|| TagChange {
                field: key.label().to_lowercase(),
                have: have.to_owned(),
                wanted: wanted.to_owned(),
            })
        })
        .collect();
    let wanted = plan.track_tag(track).to_string();
    if found.track != wanted {
        changes.push(TagChange {
            field: "track".to_owned(),
            have: found.track.clone(),
            wanted,
        });
    }
    if plan.cover.as_ref().is_some_and(|c| c.embed) && !found.has_cover {
        changes.push(TagChange {
            field: "cover".to_owned(),
            have: String::new(),
            wanted: "embedded picture".to_owned(),
        });
    }
    changes
}

/// How the tags of the file at `target` differ from what `plan` says `track` should have.
/// An empty list means the file's tags match.
pub(super) fn tag_issues(plan: &SplitPlan, track: &TrackPlan, target: &Path) -> Vec<String> {
    let mut issues = Vec::new();
    let quote = |s: &str| {
        if s.is_empty() {
            "nothing".to_owned()
        } else {
            format!("'{s}'")
        }
    };
    match snapshot::load(target) {
        Loaded::Tags(found) => {
            let mut rows: Vec<(String, String, String)> = plan
                .tag_keys(track)
                .into_iter()
                .map(|key| {
                    let wanted = plan.album.resolve(&track.meta, &key);
                    let wanted = if wanted.trim().is_empty() { "" } else { wanted };
                    let name = key.label().to_lowercase();
                    (name, found.value(&key).to_owned(), wanted.to_owned())
                })
                .collect();
            rows.push((
                "track".to_owned(),
                found.track.clone(),
                plan.track_tag(track).to_string(),
            ));
            for (name, have, wanted) in rows {
                if have != wanted {
                    issues.push(format!(
                        "Tag {name}: the file has {}, the plan says {}.",
                        quote(&have),
                        quote(&wanted)
                    ));
                }
            }
            let embeds = plan.cover.as_ref().is_some_and(|c| c.embed);
            if embeds && !found.has_cover {
                issues.push("The plan embeds cover art, but the file has none.".to_owned());
            }
        }
        Loaded::Untagged => issues.push("The file has no ID3 tag.".to_owned()),
        Loaded::Failed(err) => issues.push(format!("Tags could not be read: {err}")),
        Loaded::Missing => {}
    }
    issues
}
