//! The automatic merge of one item that was changed on two devices (`ai-docs/STORAGE_AND_SYNC.md` §6).
//!
//! A pull that finds the item changed here **and** on another device used to keep this device's state and put the
//! other one in the history for the person to restore. Now it merges the two the way a notes app does, field by field,
//! against the state they both grew out of (the *base*):
//!
//! * a field only one side changed takes that side's value;
//! * a field both sides changed to the same thing stays;
//! * tags, web addresses, custom fields and attachments are sets: what either side added is kept, what one side
//!   removed and the other did not touch is removed;
//! * the notes of an item are merged line by line (a three-way diff): changes in different places both go in, and
//!   when both sides changed the very same lines both versions are written out one after the other, marked, so no
//!   line is ever dropped;
//! * any other field both sides changed (a password, a user name) takes the newer side's value. The older value is
//!   not lost: both earlier states stay in the history.
//!
//! Nothing here reads or writes the vault. It only decides what the merged content is, so it is easy to test
//! exhaustively. Without a base (an old state that records no parent) the merge is the same, except that a field
//! that differs on the two sides is treated as changed by both.

use super::model::{
    AttachmentRef, CustomField, FieldKind, ItemPayload, MAX_CUSTOM_FIELDS, MAX_FIELD_BYTES,
    MAX_TAGS, MAX_URLS,
};
use std::collections::{BTreeMap, BTreeSet};

/// Written above the newer side's lines when both sides changed the same lines of a note.
pub const MARK_TOP: &str = "[Changed on two devices. Keep what you want and delete the rest.]";
/// Between the two versions.
pub const MARK_MIDDLE: &str = "[The version from the other device]";
/// After the second version.
pub const MARK_END: &str = "[End of the two versions]";

/// The most cells the line diff fills in for one note (`lines x lines`). A bigger note is not diffed line by line.
const MAX_DIFF_CELLS: usize = 4_000_000;

/// What the merge made of two states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merged {
    /// The merged content, ready to be sealed as the next state. Its `event`, `parent`, `changed_fields` and
    /// `updated_at` are for the caller to set.
    pub payload: ItemPayload,
    /// The places where both sides had changed the same thing differently (by field name, never a value).
    pub conflicts: Vec<String>,
}

/// Which side's value is taken when both changed the same thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Newer {
    Local,
    Remote,
    /// Same time to the millisecond: the greater value wins, so two devices merging the same pair agree.
    Tie,
}

/// One slot of a three-way merge: `base` is `None` when there is no base, `Some(None)` when the base lacked the value.
/// With `keep_data`, a value one side removed and the other changed is kept.
fn three<V: Copy + Ord>(
    base: Option<Option<V>>,
    local: Option<V>,
    remote: Option<V>,
    newer: Newer,
    keep_data: bool,
) -> (Option<V>, bool) {
    if local == remote {
        return (local, false);
    }
    match base {
        Some(b) if b == local => return (remote, false),
        Some(b) if b == remote => return (local, false),
        None if local.is_none() => return (remote, false),
        None if remote.is_none() => return (local, false),
        _ => {}
    }
    // Both changed it, to different things.
    let picked = match (local, remote) {
        (None, Some(_)) if keep_data => remote,
        (Some(_), None) if keep_data => local,
        _ => match newer {
            Newer::Local => local,
            Newer::Remote => remote,
            Newer::Tie => local.max(remote),
        },
    };
    (picked, true)
}

/// The value of one field of a state.
fn field_of<'a>(p: &'a ItemPayload, key: &str) -> Option<&'a str> {
    p.fields.get(key).map(String::as_str)
}

/// Merge two states of one item. `None` when they cannot be merged (they are not the same kind of item).
pub fn merge(
    base: Option<&ItemPayload>,
    local: &ItemPayload,
    remote: &ItemPayload,
) -> Option<Merged> {
    if local.item_type != remote.item_type || base.is_some_and(|b| b.item_type != local.item_type) {
        return None;
    }
    let newer = match local.updated_at.cmp(&remote.updated_at) {
        std::cmp::Ordering::Greater => Newer::Local,
        std::cmp::Ordering::Less => Newer::Remote,
        std::cmp::Ordering::Equal => Newer::Tie,
    };
    let mut conflicts: Vec<String> = Vec::new();
    let mut note = |name: &str, conflict: bool| {
        if conflict {
            conflicts.push(name.to_string());
        }
    };
    let mut out = local.clone();

    // ---- the plain slots
    let (v, c) = three(
        base.map(|b| Some(b.title.as_str())),
        Some(local.title.as_str()),
        Some(remote.title.as_str()),
        newer,
        false,
    );
    out.title = v.unwrap_or_default().to_string();
    note("title", c);

    let (v, c) = three(
        base.map(|b| Some(&b.favorite)),
        Some(&local.favorite),
        Some(&remote.favorite),
        newer,
        false,
    );
    out.favorite = v.copied().unwrap_or(local.favorite);
    note("favorite", c);

    // Asking for the password again is the safer setting, so with nothing to go by it stays on.
    let (v, c) = three(
        base.map(|b| Some(&b.reprompt)),
        Some(&local.reprompt),
        Some(&remote.reprompt),
        newer,
        false,
    );
    out.reprompt = if base.is_none() && local.reprompt != remote.reprompt {
        true
    } else {
        v.copied().unwrap_or(local.reprompt)
    };
    note("reprompt", c);

    let (v, c) = three(
        base.map(|b| Some(&b.archived)),
        Some(&local.archived),
        Some(&remote.archived),
        newer,
        false,
    );
    out.archived = v.copied().unwrap_or(local.archived);
    note("archived", c);

    let (v, c) = three(
        base.map(|b| b.folder.as_deref()),
        local.folder.as_deref(),
        remote.folder.as_deref(),
        newer,
        false,
    );
    out.folder = v.map(str::to_string);
    note("folder", c);

    // ---- in the trash or not: the newer decision stands, and the content is merged either way
    let (v, c) = three(
        base.map(|b| Some(&b.deleted)),
        Some(&local.deleted),
        Some(&remote.deleted),
        newer,
        false,
    );
    out.deleted = v.copied().unwrap_or(local.deleted);
    out.deleted_at = if out.deleted {
        [local, remote]
            .into_iter()
            .filter(|p| p.deleted)
            .filter_map(|p| p.deleted_at)
            .max()
            .or(Some(local.updated_at.max(remote.updated_at)))
    } else {
        None
    };
    note("trash", c);

    // ---- tags
    out.tags = merge_set(
        base.map(|b| b.tags.as_slice()),
        &local.tags,
        &remote.tags,
        String::clone,
    );
    out.tags.truncate(MAX_TAGS);

    // ---- fields
    let mut keys: BTreeSet<&str> = BTreeSet::new();
    for p in [Some(local), Some(remote), base].into_iter().flatten() {
        keys.extend(p.fields.keys().map(String::as_str));
    }
    let mut fields: BTreeMap<String, String> = BTreeMap::new();
    for key in keys {
        let kind = out.item_type.spec(key).map(|s| s.kind);
        let (l, r) = (field_of(local, key), field_of(remote, key));
        let b = base.map(|p| field_of(p, key));
        let merged: Option<String> = match kind {
            Some(FieldKind::Url) => {
                let to_lines = |s: Option<&str>| -> Vec<String> {
                    s.map(super::model::clean_urls).unwrap_or_default()
                };
                let base_lines: Option<Vec<String>> = b.map(to_lines);
                let lines = merge_set(
                    base_lines.as_deref(),
                    &to_lines(l),
                    &to_lines(r),
                    String::clone,
                );
                let lines: Vec<String> = lines.into_iter().take(MAX_URLS).collect();
                (!lines.is_empty()).then(|| lines.join("\n"))
            }
            Some(FieldKind::Note | FieldKind::Paragraph) => {
                let (text, c) = merge_text(b, l, r, newer);
                note(key, c);
                text
            }
            _ => {
                let (v, c) = three(b, l, r, newer, true);
                note(key, c);
                v.map(str::to_string)
            }
        };
        if let Some(v) = merged.filter(|v| !v.is_empty()) {
            fields.insert(key.to_string(), v);
        }
    }
    out.fields = fields;

    // ---- custom fields, by name
    let (custom, custom_conflicts) = merge_custom(
        base.map(|b| b.custom_fields.as_slice()),
        &local.custom_fields,
        &remote.custom_fields,
        newer,
    );
    out.custom_fields = custom;
    out.custom_fields.truncate(MAX_CUSTOM_FIELDS);
    for name in custom_conflicts {
        note(&name, true);
    }

    // ---- attachments, by id
    out.attachments = merge_set(
        base.map(|b| b.attachments.as_slice()),
        &local.attachments,
        &remote.attachments,
        |a: &AttachmentRef| a.id.clone(),
    );

    out.created_at = local.created_at.min(remote.created_at);
    out.updated_at = local.updated_at.max(remote.updated_at);
    Some(Merged {
        payload: out,
        conflicts,
    })
}

/// A set that grows with what either side added and shrinks by what one side removed (and the other left alone).
/// Order: this side's, then what only the other side has.
fn merge_set<T: Clone>(
    base: Option<&[T]>,
    local: &[T],
    remote: &[T],
    key: impl Fn(&T) -> String,
) -> Vec<T> {
    let keys = |xs: &[T]| -> BTreeSet<String> { xs.iter().map(&key).collect() };
    let (lk, rk) = (keys(local), keys(remote));
    let bk = base.map(keys);
    // Only one side has it: it was added there, or the other side removed it (when the base had it).
    let keep = |k: &str| match (lk.contains(k), rk.contains(k)) {
        (true, true) => true,
        (false, false) => false,
        _ => !bk.as_ref().is_some_and(|b| b.contains(k)),
    };
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut out = Vec::new();
    for x in local.iter().chain(remote.iter()) {
        let k = key(x);
        if keep(&k) && seen.insert(k) {
            out.push(x.clone());
        }
    }
    out
}

/// Custom fields are matched by name (the n-th field of one name with the n-th of the same name).
fn merge_custom(
    base: Option<&[CustomField]>,
    local: &[CustomField],
    remote: &[CustomField],
    newer: Newer,
) -> (Vec<CustomField>, Vec<String>) {
    fn keyed(xs: &[CustomField]) -> Vec<(String, &CustomField)> {
        let mut count: BTreeMap<&str, usize> = BTreeMap::new();
        xs.iter()
            .map(|c| {
                let n = count.entry(c.name.as_str()).or_insert(0);
                *n += 1;
                (format!("{}\u{0}{}", c.name, n), c)
            })
            .collect()
    }
    let (lk, rk) = (keyed(local), keyed(remote));
    let bk = base.map(keyed);
    fn find<'a>(xs: &[(String, &'a CustomField)], k: &str) -> Option<(&'a str, bool)> {
        xs.iter()
            .find(|(key, _)| key == k)
            .map(|(_, c)| (c.value.as_str(), c.hidden))
    }
    let mut order: Vec<&str> = Vec::new();
    for (k, _) in lk.iter().chain(rk.iter()) {
        if !order.contains(&k.as_str()) {
            order.push(k);
        }
    }
    let mut out = Vec::new();
    let mut conflicts = Vec::new();
    for k in order {
        let (l, r) = (find(&lk, k), find(&rk, k));
        let b = bk.as_ref().map(|b| find(b, k));
        let (v, c) = three(b, l, r, newer, true);
        if let Some((value, hidden)) = v {
            let name = k.split('\u{0}').next().unwrap_or_default().to_string();
            if c {
                conflicts.push(name.clone());
            }
            out.push(CustomField {
                name,
                value: value.to_string(),
                hidden,
            });
        }
    }
    (out, conflicts)
}

// ------------------------------------------------------------------ notes: a three-way diff by lines

/// Merge the text of a note. `base` is `None` when there is no base, `Some(None)` when the base had no such text.
fn merge_text(
    base: Option<Option<&str>>,
    local: Option<&str>,
    remote: Option<&str>,
    newer: Newer,
) -> (Option<String>, bool) {
    let (l, r) = (local.unwrap_or(""), remote.unwrap_or(""));
    if l == r {
        return (local.map(str::to_string), false);
    }
    // One side did not touch it, or one side has nothing: the plain rule decides.
    let plain = || three(base, local, remote, newer, true);
    let Some(Some(b)) = base else {
        // No base: with text on both sides, keep both (nothing says which lines are new).
        if local.is_none() || remote.is_none() || l.is_empty() || r.is_empty() {
            let (v, c) = plain();
            return (v.map(str::to_string), c);
        }
        return (Some(both(l, r, newer)), true);
    };
    if b == l {
        return (remote.map(str::to_string), false);
    }
    if b == r {
        return (local.map(str::to_string), false);
    }
    match diff3(b, l, r, newer) {
        Some((text, conflict)) if text.len() <= MAX_FIELD_BYTES => (Some(text), conflict),
        // Too big to diff, or the two together would not fit: the newer text stays, the other is in the history.
        _ => {
            let (v, _) = plain();
            (v.map(str::to_string), true)
        }
    }
}

/// Both texts, the newer one first, with marks.
fn both(local: &str, remote: &str, newer: Newer) -> String {
    let (first, second) = match newer {
        Newer::Local => (local, remote),
        Newer::Remote => (remote, local),
        Newer::Tie if local >= remote => (local, remote),
        Newer::Tie => (remote, local),
    };
    format!("{MARK_TOP}\n{first}\n{MARK_MIDDLE}\n{second}\n{MARK_END}")
}

/// One region of the base that a side replaced: `base[start..end]` became `lines`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Hunk {
    start: usize,
    end: usize,
    lines: Vec<String>,
}

/// The regions where `other` differs from `base` (a longest-common-subsequence alignment).
fn hunks(base: &[&str], other: &[&str]) -> Vec<Hunk> {
    let (n, m) = (base.len(), other.len());
    // lcs[i][j] = length of the common subsequence of base[i..] and other[j..]
    let width = m + 1;
    let mut lcs = vec![0u32; (n + 1) * width];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i * width + j] = if base[i] == other[j] {
                lcs[(i + 1) * width + j + 1] + 1
            } else {
                lcs[(i + 1) * width + j].max(lcs[i * width + j + 1])
            };
        }
    }
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    let (mut hs, mut hj) = (None::<usize>, 0);
    let flush = |out: &mut Vec<Hunk>, hs: &mut Option<usize>, hj: usize, i: usize, j: usize| {
        if let Some(start) = hs.take() {
            out.push(Hunk {
                start,
                end: i,
                lines: other[hj..j].iter().map(|s| (*s).to_string()).collect(),
            });
        }
    };
    while i < n || j < m {
        if i < n && j < m && base[i] == other[j] {
            flush(&mut out, &mut hs, hj, i, j);
            i += 1;
            j += 1;
        } else {
            if hs.is_none() {
                hs = Some(i);
                hj = j;
            }
            if j < m && (i == n || lcs[i * width + j + 1] >= lcs[(i + 1) * width + j]) {
                j += 1;
            } else {
                i += 1;
            }
        }
    }
    flush(&mut out, &mut hs, hj, i, j);
    out
}

/// The text of `base[start..end]` with this side's hunks inside it applied.
fn apply(base: &[&str], start: usize, end: usize, side: &[&Hunk]) -> Vec<String> {
    let mut out = Vec::new();
    let mut pos = start;
    for h in side {
        out.extend(base[pos..h.start].iter().map(|s| (*s).to_string()));
        out.extend(h.lines.iter().cloned());
        pos = h.end;
    }
    out.extend(base[pos..end].iter().map(|s| (*s).to_string()));
    out
}

/// Three-way merge of text by lines. `None` when it is too big to diff.
fn diff3(base: &str, local: &str, remote: &str, newer: Newer) -> Option<(String, bool)> {
    let (b, l, r): (Vec<&str>, Vec<&str>, Vec<&str>) = (
        base.split('\n').collect(),
        local.split('\n').collect(),
        remote.split('\n').collect(),
    );
    if b.len().saturating_mul(l.len().max(r.len())) > MAX_DIFF_CELLS {
        return None;
    }
    let (hl, hr) = (hunks(&b, &l), hunks(&b, &r));
    // Every hunk with the side it came from, in the order of the base; touching or overlapping hunks form one cluster.
    let mut all: Vec<(bool, &Hunk)> = hl
        .iter()
        .map(|h| (true, h))
        .chain(hr.iter().map(|h| (false, h)))
        .collect();
    all.sort_by_key(|(side, h)| (h.start, h.end, !*side));

    let mut out: Vec<String> = Vec::new();
    let mut pos = 0;
    let mut conflict = false;
    let mut k = 0;
    while k < all.len() {
        let start = all[k].1.start;
        let mut end = all[k].1.end;
        let mut last = k + 1;
        while last < all.len() && all[last].1.start <= end {
            end = end.max(all[last].1.end);
            last += 1;
        }
        let local_side: Vec<&Hunk> = all[k..last]
            .iter()
            .filter(|(s, _)| *s)
            .map(|(_, h)| *h)
            .collect();
        let remote_side: Vec<&Hunk> = all[k..last]
            .iter()
            .filter(|(s, _)| !*s)
            .map(|(_, h)| *h)
            .collect();
        out.extend(b[pos..start].iter().map(|s| (*s).to_string()));
        let (lt, rt) = (
            apply(&b, start, end, &local_side),
            apply(&b, start, end, &remote_side),
        );
        if remote_side.is_empty() || lt == rt {
            out.extend(lt);
        } else if local_side.is_empty() {
            out.extend(rt);
        } else {
            conflict = true;
            let local_first = match newer {
                Newer::Local => true,
                Newer::Remote => false,
                Newer::Tie => lt >= rt,
            };
            let (first, second) = if local_first { (lt, rt) } else { (rt, lt) };
            out.push(MARK_TOP.to_string());
            out.extend(first);
            out.push(MARK_MIDDLE.to_string());
            out.extend(second);
            out.push(MARK_END.to_string());
        }
        pos = end;
        k = last;
    }
    out.extend(b[pos..].iter().map(|s| (*s).to_string()));
    Some((out.join("\n"), conflict))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::model::{ItemDraft, ItemType};

    fn payload(item_type: ItemType, title: &str, fields: &[(&str, &str)], at: u64) -> ItemPayload {
        let draft = ItemDraft {
            item_type,
            title: title.into(),
            favorite: false,
            reprompt: false,
            tags: Vec::new(),
            folder: None,
            fields: fields
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            custom_fields: Vec::new(),
        };
        let mut p = ItemPayload::new("11111111-1111-4111-8111-111111111111".into(), &draft, 1);
        p.updated_at = at;
        p
    }

    fn login(fields: &[(&str, &str)], at: u64) -> ItemPayload {
        payload(ItemType::Login, "Site", fields, at)
    }

    fn custom(name: &str, value: &str, hidden: bool) -> CustomField {
        CustomField {
            name: name.into(),
            value: value.into(),
            hidden,
        }
    }

    #[test]
    fn a_field_only_one_side_changed_takes_that_sides_value() {
        let base = login(
            &[
                ("username", "ana"),
                ("password", "old"),
                ("url", "https://a.example"),
            ],
            10,
        );
        let mut local = base.clone();
        local.fields.insert("password".into(), "new-local".into());
        local.updated_at = 20;
        let mut remote = base.clone();
        remote.fields.insert("username".into(), "ana2".into());
        remote.updated_at = 30;
        let m = merge(Some(&base), &local, &remote).unwrap();
        assert_eq!(m.payload.fields["password"], "new-local");
        assert_eq!(m.payload.fields["username"], "ana2");
        assert_eq!(m.payload.fields["url"], "https://a.example");
        assert!(m.conflicts.is_empty(), "{:?}", m.conflicts);
    }

    #[test]
    fn a_field_both_changed_differently_takes_the_newer_and_says_so() {
        let base = login(&[("password", "old")], 10);
        let mut local = base.clone();
        local.fields.insert("password".into(), "from-phone".into());
        local.updated_at = 50;
        let mut remote = base.clone();
        remote
            .fields
            .insert("password".into(), "from-laptop".into());
        remote.updated_at = 40;
        let m = merge(Some(&base), &local, &remote).unwrap();
        assert_eq!(m.payload.fields["password"], "from-phone");
        assert_eq!(m.conflicts, ["password"]);
        // the other way round: the laptop is newer
        let m = merge(Some(&base), &remote, &local).unwrap();
        assert_eq!(
            m.payload.fields["password"], "from-phone",
            "newer is newer on either side"
        );
        // the same change on both sides is no conflict
        let mut same = base.clone();
        same.fields.insert("password".into(), "from-phone".into());
        let m = merge(Some(&base), &local, &same).unwrap();
        assert!(m.conflicts.is_empty());
    }

    #[test]
    fn two_devices_merging_the_same_pair_at_the_same_second_agree() {
        let base = login(&[("password", "old")], 10);
        let mut a = base.clone();
        a.fields.insert("password".into(), "aaa".into());
        a.updated_at = 99;
        let mut b = base.clone();
        b.fields.insert("password".into(), "bbb".into());
        b.updated_at = 99;
        let one = merge(Some(&base), &a, &b).unwrap();
        let two = merge(Some(&base), &b, &a).unwrap();
        assert_eq!(one.payload.fields, two.payload.fields);
    }

    #[test]
    fn a_value_one_side_removed_and_the_other_edited_is_kept() {
        let base = login(&[("username", "ana"), ("totp", "JBSWY3DPEHPK3PXP")], 10);
        let mut local = base.clone();
        local.fields.remove("totp");
        local.updated_at = 90;
        let mut remote = base.clone();
        remote
            .fields
            .insert("totp".into(), "KRSXG5CTMVRXEZLU".into());
        remote.updated_at = 20;
        let m = merge(Some(&base), &local, &remote).unwrap();
        assert_eq!(m.payload.fields["totp"], "KRSXG5CTMVRXEZLU");
        // removed on one side and untouched on the other is gone
        let mut untouched = base.clone();
        untouched.updated_at = 20;
        let m = merge(Some(&base), &local, &untouched).unwrap();
        assert!(!m.payload.fields.contains_key("totp"));
    }

    #[test]
    fn tags_are_a_set_additions_join_and_removals_stick() {
        let mut base = login(&[], 10);
        base.tags = vec!["work".into(), "old".into()];
        let mut local = base.clone();
        local.tags = vec!["work".into(), "phone".into()];
        let mut remote = base.clone();
        remote.tags = vec!["work".into(), "old".into(), "laptop".into()];
        let m = merge(Some(&base), &local, &remote).unwrap();
        assert_eq!(m.payload.tags, ["work", "phone", "laptop"]);
        // no base: everything either side has is kept
        let m = merge(None, &local, &remote).unwrap();
        assert_eq!(m.payload.tags, ["work", "phone", "old", "laptop"]);
    }

    #[test]
    fn web_addresses_from_both_devices_are_all_kept() {
        let base = login(&[("url", "https://a.example")], 10);
        let local = login(&[("url", "https://a.example\nhttps://phone.example")], 20);
        let remote = login(&[("url", "https://a.example\nhttps://laptop.example")], 30);
        let m = merge(Some(&base), &local, &remote).unwrap();
        assert_eq!(
            m.payload.fields["url"],
            "https://a.example\nhttps://phone.example\nhttps://laptop.example"
        );
        // one side removing an address the other did not touch
        let removed = login(&[("url", "https://b.example")], 40);
        let base2 = login(&[("url", "https://a.example\nhttps://b.example")], 10);
        let m = merge(Some(&base2), &removed, &base2).unwrap();
        assert_eq!(m.payload.fields["url"], "https://b.example");
    }

    #[test]
    fn custom_fields_merge_by_name() {
        let mut base = login(&[], 10);
        base.custom_fields = vec![custom("PIN", "1111", true), custom("Plan", "Free", false)];
        let mut local = base.clone();
        local.custom_fields = vec![
            custom("PIN", "2222", true),
            custom("Plan", "Free", false),
            custom("Seat", "A1", false),
        ];
        local.updated_at = 20;
        let mut remote = base.clone();
        remote.custom_fields = vec![
            custom("PIN", "1111", true),
            custom("Plan", "Team", false),
            custom("Floor", "3", false),
        ];
        remote.updated_at = 30;
        let m = merge(Some(&base), &local, &remote).unwrap();
        let by_name = |n: &str| {
            m.payload
                .custom_fields
                .iter()
                .find(|c| c.name == n)
                .map(|c| c.value.clone())
        };
        assert_eq!(by_name("PIN").as_deref(), Some("2222"));
        assert_eq!(by_name("Plan").as_deref(), Some("Team"));
        assert_eq!(by_name("Seat").as_deref(), Some("A1"));
        assert_eq!(by_name("Floor").as_deref(), Some("3"));
        assert!(m.conflicts.is_empty());
        // a field one side deleted and the other left alone is gone
        let mut dropped = base.clone();
        dropped.custom_fields = vec![custom("Plan", "Free", false)];
        let m = merge(Some(&base), &dropped, &base).unwrap();
        assert!(m.payload.custom_fields.iter().all(|c| c.name != "PIN"));
    }

    #[test]
    fn notes_changed_in_different_places_both_go_in() {
        let base = payload(
            ItemType::SecureNote,
            "Codes",
            &[("body", "one\ntwo\nthree\nfour\nfive")],
            10,
        );
        let local = payload(
            ItemType::SecureNote,
            "Codes",
            &[("body", "one\nTWO\nthree\nfour\nfive")],
            20,
        );
        let remote = payload(
            ItemType::SecureNote,
            "Codes",
            &[("body", "one\ntwo\nthree\nfour\nfive\nsix")],
            30,
        );
        let m = merge(Some(&base), &local, &remote).unwrap();
        assert_eq!(m.payload.fields["body"], "one\nTWO\nthree\nfour\nfive\nsix");
        assert!(m.conflicts.is_empty());
    }

    #[test]
    fn notes_changed_on_the_same_lines_keep_both_versions_marked() {
        let base = payload(ItemType::SecureNote, "N", &[("body", "a\nb\nc")], 10);
        let local = payload(ItemType::SecureNote, "N", &[("body", "a\nB-phone\nc")], 50);
        let remote = payload(ItemType::SecureNote, "N", &[("body", "a\nB-laptop\nc")], 40);
        let m = merge(Some(&base), &local, &remote).unwrap();
        let body = &m.payload.fields["body"];
        assert_eq!(
            body,
            &format!("a\n{MARK_TOP}\nB-phone\n{MARK_MIDDLE}\nB-laptop\n{MARK_END}\nc")
        );
        assert_eq!(m.conflicts, ["body"]);
        // nothing of either side is missing
        for line in ["a", "c", "B-phone", "B-laptop"] {
            assert!(body.lines().any(|l| l == line), "{line}");
        }
    }

    #[test]
    fn notes_with_no_base_keep_both_texts() {
        let local = payload(ItemType::SecureNote, "N", &[("body", "phone text")], 50);
        let remote = payload(ItemType::SecureNote, "N", &[("body", "laptop text")], 40);
        let m = merge(None, &local, &remote).unwrap();
        let body = &m.payload.fields["body"];
        assert!(body.contains("phone text") && body.contains("laptop text"));
        assert!(body.starts_with(MARK_TOP));
        // a note only one side has is simply kept
        let empty = payload(ItemType::SecureNote, "N", &[], 40);
        let m = merge(None, &local, &empty).unwrap();
        assert_eq!(m.payload.fields["body"], "phone text");
        assert!(m.conflicts.is_empty());
    }

    #[test]
    fn a_login_note_is_a_note_too() {
        let base = login(&[("notes", "x\ny")], 10);
        let local = login(&[("notes", "x\ny\nz")], 20);
        let remote = login(&[("notes", "w\nx\ny")], 30);
        let m = merge(Some(&base), &local, &remote).unwrap();
        assert_eq!(m.payload.fields["notes"], "w\nx\ny\nz");
    }

    #[test]
    fn a_huge_note_falls_back_to_the_newer_text() {
        let big = "line\n".repeat(3000);
        let base = payload(ItemType::SecureNote, "N", &[("body", &big)], 10);
        let local = payload(
            ItemType::SecureNote,
            "N",
            &[("body", &format!("{big}phone"))],
            20,
        );
        let remote = payload(
            ItemType::SecureNote,
            "N",
            &[("body", &format!("laptop\n{big}"))],
            30,
        );
        let m = merge(Some(&base), &local, &remote).unwrap();
        // 3000 x 3000 lines is over the diff budget
        assert!(m.payload.fields["body"].starts_with("laptop"));
        assert!(!m.payload.fields["body"].contains("phone"));
        assert_eq!(m.conflicts, ["body"]);
    }

    #[test]
    fn the_trash_follows_the_newer_decision_and_the_content_is_merged_anyway() {
        let base = login(&[("username", "ana")], 10);
        let mut local = base.clone();
        local.deleted = true;
        local.deleted_at = Some(60);
        local.updated_at = 60;
        let mut remote = base.clone();
        remote.fields.insert("username".into(), "ana2".into());
        remote.updated_at = 50;
        let m = merge(Some(&base), &local, &remote).unwrap();
        assert!(m.payload.deleted);
        assert_eq!(m.payload.deleted_at, Some(60));
        assert_eq!(
            m.payload.fields["username"], "ana2",
            "an edit made elsewhere is not lost"
        );
        // restored on one side only
        let mut trashed = base.clone();
        trashed.deleted = true;
        trashed.deleted_at = Some(15);
        let m = merge(Some(&trashed), &trashed, &base).unwrap();
        assert!(!m.payload.deleted);
        assert_eq!(m.payload.deleted_at, None);
    }

    #[test]
    fn flags_follow_the_side_that_changed_them() {
        let base = login(&[], 10);
        let mut local = base.clone();
        local.favorite = true;
        let mut remote = base.clone();
        remote.reprompt = true;
        remote.folder = Some("Work".into());
        let m = merge(Some(&base), &local, &remote).unwrap();
        assert!(m.payload.favorite);
        assert!(m.payload.reprompt);
        assert_eq!(m.payload.folder.as_deref(), Some("Work"));
        // with no base, asking again stays on if either side asks
        let m = merge(None, &base, &remote).unwrap();
        assert!(m.payload.reprompt);
    }

    #[test]
    fn items_of_different_kinds_are_not_merged() {
        let a = login(&[], 1);
        let b = payload(ItemType::SecureNote, "N", &[], 2);
        assert!(merge(None, &a, &b).is_none());
        assert!(merge(Some(&b), &a, &a).is_none());
    }

    #[test]
    fn the_timestamps_never_go_back_and_the_id_stays() {
        let mut a = login(&[], 100);
        a.created_at = 5;
        let mut b = login(&[], 70);
        b.created_at = 3;
        let m = merge(None, &a, &b).unwrap();
        assert_eq!(m.payload.created_at, 3);
        assert_eq!(m.payload.updated_at, 100);
        assert_eq!(m.payload.id, a.id);
    }

    #[test]
    fn merging_a_state_with_itself_changes_nothing() {
        let mut a = login(&[("username", "ana"), ("notes", "n\nm")], 10);
        a.tags = vec!["x".into()];
        a.custom_fields = vec![custom("k", "v", false)];
        let m = merge(Some(&a), &a, &a).unwrap();
        assert_eq!(m.payload, a);
        assert!(m.conflicts.is_empty());
    }

    // ---- the line diff on its own

    fn text(base: &str, local: &str, remote: &str) -> (String, bool) {
        diff3(base, local, remote, Newer::Local).unwrap()
    }

    #[test]
    fn diff3_takes_non_overlapping_changes_from_both() {
        assert_eq!(
            text("a\nb\nc\nd", "a\nB\nc\nd", "a\nb\nc\nD"),
            ("a\nB\nc\nD".into(), false)
        );
        // insertions in different places, deletions in others
        assert_eq!(
            text("1\n2\n3\n4\n5", "0\n1\n2\n3\n4\n5", "1\n2\n3\n4"),
            ("0\n1\n2\n3\n4".into(), false)
        );
        // the same change on both sides is applied once
        assert_eq!(text("a\nb", "a\nX", "a\nX"), ("a\nX".into(), false));
        // an empty base
        assert_eq!(text("", "a", "a"), ("a".into(), false));
    }

    #[test]
    fn diff3_marks_overlapping_changes_and_keeps_the_text_around_them() {
        let (t, conflict) = text("a\nb\nc\nd\ne", "a\nB1\nc\nd\ne", "a\nB2\nc\nd\nE");
        assert!(conflict);
        assert_eq!(
            t,
            format!("a\n{MARK_TOP}\nB1\n{MARK_MIDDLE}\nB2\n{MARK_END}\nc\nd\nE")
        );
        // adjacent edits are treated as one region
        let (t, conflict) = text("a\nb\nc", "a\nX\nc", "a\nb\nY");
        assert!(conflict);
        assert!(t.contains("X") && t.contains("Y"));
    }

    #[test]
    fn diff3_never_drops_a_line_either_side_added() {
        let base = "k1\nk2\nk3";
        let local = "k1\nphone-a\nk2\nk3\nphone-b";
        let remote = "laptop-a\nk1\nk2\nk3";
        let (t, conflict) = text(base, local, remote);
        assert!(!conflict);
        for line in ["k1", "k2", "k3", "phone-a", "phone-b", "laptop-a"] {
            assert!(t.lines().any(|l| l == line), "{line} missing from {t:?}");
        }
    }
}
