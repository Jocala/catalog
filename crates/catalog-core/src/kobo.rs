//! Port of `ReaderCatalogGUI/KoboPath.swift` + `KoboIndex.swift` +
//! the `KoboLauncher` enum in `ReaderCatalogApp.swift`.
//!
//! `KoboPath` replicates `calibre/devices/kobo/driver.py`
//! `create_upload_path` (MAX_PATH_LEN=185, FAT sanitize, title_sort).
//! Transport is russh throughout (`ssh::run_shell_blocks`: shell channel,
//! since Dropbear swallows EXEC); no external `ssh` binary is required.

pub mod handoff;
pub mod open;
pub mod ssh;
pub mod sync;

pub const MAX_PATH_LEN: usize = 185;
pub const PREFIX: &str = "/mnt/onboard";

// ---------------------------------------------------------------------------
// KoboPath (KoboPath.swift)
// ---------------------------------------------------------------------------

fn is_sanitize_char(c: char) -> bool {
    (c as u32) <= 0x1F || matches!(c, '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\\' | '+')
}

/// calibre title_sort: move leading A/An/The to the end with a comma.
pub fn title_sort(title: &str) -> String {
    let t = title.trim();
    for prefix in ["A ", "An ", "The "] {
        if t.len() > prefix.len() && t[..prefix.len()].eq_ignore_ascii_case(prefix) {
            let rest = t[prefix.len()..].trim();
            return format!("{}, {}", rest, prefix.trim());
        }
    }
    t.to_string()
}

/// calibre author_to_author_sort (comma method, simplified).
pub fn author_sort(natural: &str) -> String {
    let a = natural.trim();
    if a.is_empty() {
        return String::new();
    }
    if a.contains(',') {
        return a.to_string();
    }
    let tokens: Vec<&str> = a.split_whitespace().collect();
    if tokens.len() < 2 {
        return a.to_string();
    }
    format!("{}, {}", tokens.last().unwrap(), tokens[..tokens.len() - 1].join(" "))
}

pub fn natural_name(sort: &str) -> String {
    let mut parts = sort.splitn(2, ',');
    match (parts.next(), parts.next()) {
        (Some(last), Some(first)) => format!("{} {}", first.trim(), last.trim()),
        _ => sort.to_string(),
    }
}

fn fold_diacritics(s: &str) -> String {
    // Minimal fold covering observed library cases (Carré→Carre) plus
    // common Latin-1; full unicode-normalization would need `deunicode`.
    s.chars()
        .map(|c| match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' => 'A',
            'È' | 'É' | 'Ê' | 'Ë' => 'E',
            'Ì' | 'Í' | 'Î' | 'Ï' => 'I',
            'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' => 'O',
            'Ù' | 'Ú' | 'Û' | 'Ü' => 'U',
            'Ç' => 'C',
            'Ñ' => 'N',
            'ÿ' | 'ý' => 'y',
            'œ' => 'e',
            _ => c,
        })
        .collect()
}

/// calibre sanitize_file_name + Kobo ascii handling ('→_).
pub fn sanitize(name: &str) -> String {
    let ascii = fold_diacritics(name).replace('\'', "_");
    let mut one: String = ascii.chars().map(|c| if is_sanitize_char(c) { '_' } else { c }).collect();
    // collapse whitespace → single space, strip
    let collapsed = one.split_whitespace().collect::<Vec<_>>().join(" ");
    one = collapsed.trim().to_string();
    let (mut bname, ext) = match one.rfind('.') {
        Some(i) => (one[..i].to_string(), one[i + 1..].to_string()),
        None => (one.clone(), String::new()),
    };
    if !bname.is_empty() && bname.chars().all(|c| c == '.') {
        bname = "_".to_string();
    }
    let mut result = bname.replace("..", "_");
    if !ext.is_empty() {
        result.push('.');
        result.push_str(&ext);
    } else if one.is_empty() {
        result = one.clone();
    }
    if result.ends_with('.') || result.ends_with(' ') {
        result.pop();
        result.push('_');
    }
    if result.starts_with('.') {
        result = format!("_{}", &result[1..]);
    }
    result
}

fn encoded_len(s: &str) -> usize {
    // macOS utf-16 filename length rule: 2 bytes per code unit.
    s.encode_utf16().count() * 2
}

fn shorten_component(s: &str, by: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    let l = chars.len();
    if l <= by {
        return s.to_string();
    }
    let keep = (l - by) / 2;
    if keep == 0 {
        return chars[..l - by.min(l - 1)].iter().collect();
    }
    let head: String = chars[..keep].iter().collect();
    let tail: String = chars[l - keep..].iter().collect();
    format!("{head}{tail}")
}

/// Predicted Kobo path (mirrors `KoboPath.predictedPath`).
pub fn predicted_path(title: &str, author_sort_value: &str, authors_natural: Option<&str>) -> String {
    let a_sort = sanitize(author_sort_value);
    let t_sort = sanitize(&title_sort(title));
    let authors = sanitize(authors_natural.unwrap_or(&natural_name(author_sort_value)));
    let file = sanitize(&format!("{t_sort} - {authors}.kepub.epub"));
    let mut comps = [a_sort, file];
    let prefix_len = encoded_len(PREFIX);
    let max_comps = MAX_PATH_LEN * 2 - prefix_len - 2;
    if encoded_len(&comps.join("/")) > max_comps {
        // Simple proportional shorten (mirrors Swift fallback).
        let over = encoded_len(&format!("{PREFIX}/{}", comps.join("/"))) - MAX_PATH_LEN * 2;
        let by_chars = (over / 2).max(10);
        let short_title = shorten_component(&t_sort, by_chars);
        comps[1] = sanitize(&format!("{short_title} - {authors}.kepub.epub"));
    }
    format!("{PREFIX}/{}", comps.join("/"))
}

// ---------------------------------------------------------------------------
// KoboIndex (KoboIndex.swift) — kobo_index.txt in the platform data dir.
// ---------------------------------------------------------------------------

pub fn cached_paths() -> Option<Vec<String>> {
    let data = std::fs::read_to_string(crate::settings::kobo_index_file()).ok()?;
    let lines: Vec<String> =
        data.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
    if lines.is_empty() {
        None
    } else {
        Some(lines)
    }
}

pub fn save_paths(paths: &[String]) {
    let file = crate::settings::kobo_index_file();
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(file, paths.join("\n") + "\n");
}

pub fn contains_path(path: &str, cache: Option<&[String]>) -> bool {
    if let Some(c) = cache {
        return c.iter().any(|p| p == path);
    }
    cached_paths().map(|c| c.iter().any(|p| p == path)).unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Launcher (KoboLauncher — ssh heredoc, strict fail-closed matching)
// ---------------------------------------------------------------------------

fn norm(s: &str) -> String {
    fold_diacritics(s)
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

fn last_name(author: &str) -> String {
    let t = author.trim();
    if let Some((last, _)) = t.split_once(',') {
        last.trim().to_string()
    } else {
        t.split_whitespace().last().unwrap_or(t).to_string()
    }
}

// ---------------------------------------------------------------------------
// Match — identity is (title, author), never a substring sweep of the path
// ---------------------------------------------------------------------------
//
// The old matcher required every title token *plus* the author's last name to
// appear somewhere in the normalised path. With a one-word title ("The
// Hollow") the token set collapsed to ["hollow"], so every book with that word
// anywhere — including an author named Holloway — became a "match", and the
// shell could only fail closed with an 8-item dump. Here each device file is
// parsed into its title and author components and compared as identity.

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Tier {
    /// Same title, same author.
    Exact,
    /// Same author, near-identical title (subtitle/typo).
    Fuzzy,
    /// Same title, different author — a different book, offered, never opened.
    TitleOnly,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Exact => "exact",
            Tier::Fuzzy => "fuzzy",
            Tier::TitleOnly => "title_only",
        }
    }
}

/// One device file, parsed back into the identity that produced its name.
#[derive(Clone, Debug)]
pub struct Identity {
    pub path: String,
    /// Title component as written on the device (article form untouched).
    pub title: String,
    /// Natural author name (file component, else derived from the dir).
    pub author: String,
    /// Normalised last name — the identity key that must agree.
    pub author_key: String,
    /// Normalised title (article-insensitive; subtitle kept).
    pub title_key: String,
    /// Normalised head of a subtitled title, `None` when plain.
    pub title_head: Option<String>,
    /// `.kepub.epub` beats `.epub` for the same identity.
    pub kepub: bool,
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub identity: Identity,
    pub tier: Tier,
}

impl Candidate {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "path": self.identity.path,
            "title": self.identity.title,
            "author": self.identity.author,
            "tier": self.tier.as_str(),
        })
    }
}

pub struct Match {
    pub candidates: Vec<Candidate>,
    /// True when the best tier has the author agreeing (Exact/Fuzzy). The
    /// shell uses this to decide between a tie-breaker and a "different
    /// book" offer; it never opens on its own.
    pub author_matched: bool,
    /// The single unambiguous path, when one identity wins outright.
    pub auto: Option<String>,
    /// Device files considered (honest denominator for messages).
    pub total_paths: usize,
}

/// Fuzzy titles must be at least this similar (1 - edit/max_len).
pub const FUZZY_MIN: f64 = 0.85;

/// Strip a leading "The/A/An" or a trailing ", The/A/An" (calibre title_sort).
fn strip_article(mut t: String) -> String {
    let lower = t.to_ascii_lowercase();
    for suf in [", the", ", an", ", a"] {
        if lower.ends_with(suf) && t.len() > suf.len() {
            t.truncate(t.len() - suf.len());
            t = t.trim().to_string();
            break;
        }
    }
    let lower = t.to_ascii_lowercase();
    for pre in ["the ", "an ", "a "] {
        if lower.starts_with(pre) && t.len() > pre.len() {
            t = t[pre.len()..].trim().to_string();
            break;
        }
    }
    t
}

/// Compare key for a title: diacritics folded, article normalised, punctuation
/// removed. Subtitle kept — identity and dedupe must never collapse two
/// volumes of one series.
pub fn title_key(s: &str) -> String {
    norm(&strip_article(fold_diacritics(s)))
}

/// The head of a subtitled title ("The Hollow: A Novel" → Some("hollow")),
/// `None` for a plain title. Used only as a title-tolerance rule, never as an
/// identity key.
pub fn title_head(s: &str) -> Option<String> {
    fold_diacritics(s)
        .split_once(':')
        .map(|(h, _)| norm(&strip_article(h.to_string())))
}

/// Compare key for an author: the last name only ("Connelly, Michael" and
/// "Michael Connelly" both give "connelly").
pub fn author_key(sort_form: &str, natural: &str) -> String {
    let src = if !sort_form.trim().is_empty() { sort_form } else { natural };
    norm(&last_name(src))
}

/// Parse `/mnt/onboard/<AuthorDir>/<Title> - <Author>.kepub.epub`.
/// `None` for anything that is not a plain epub (AppleDouble, kepub-only
/// directories, partial downloads) — those never become candidates.
pub fn parse_candidate(path: &str) -> Option<Identity> {
    let file = path.rsplit('/').next()?.trim();
    if file.is_empty() || file.starts_with('.') {
        return None;
    }
    let (stem, kepub) = if let Some(s) = file.strip_suffix(".kepub.epub") {
        (s, true)
    } else {
        (file.strip_suffix(".epub")?, false)
    };
    // The author is the trailing component of "<title> - <author>"; titles
    // may contain " - " themselves, so split from the right.
    let (title, file_author) = match stem.rsplit_once(" - ") {
        Some((t, a)) => (t.trim().to_string(), a.trim().to_string()),
        None => (stem.trim().to_string(), String::new()),
    };
    if title.is_empty() {
        return None;
    }
    let mut comps: Vec<&str> = path.split('/').collect();
    comps.pop(); // filename
    let dir = comps.pop().unwrap_or("").trim();
    let author = if !file_author.is_empty() {
        file_author
    } else {
        natural_name(dir)
    };
    Some(Identity {
        path: path.to_string(),
        author_key: author_key(dir, &author),
        title_key: title_key(&title),
        title_head: title_head(&title),
        title,
        author,
        kepub,
    })
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

fn similarity(a: &str, b: &str) -> f64 {
    let m = a.chars().count().max(b.chars().count());
    if m == 0 {
        return 1.0;
    }
    1.0 - (levenshtein(a, b) as f64 / m as f64)
}

/// Classify device paths against the requested book. Files that match neither
/// title nor author are dropped entirely — they are not "duplicates", and
/// they never reach a dialog.
pub fn classify(paths: &[String], title: &str, author: &str) -> Match {
    let want_title = title_key(title);
    let want_head = title_head(title);
    let want_key = author_key(author, author);
    let author_known = !want_key.is_empty();

    let mut scored: Vec<Candidate> = Vec::new();
    for p in paths {
        let Some(id) = parse_candidate(p) else { continue };
        if id.title_key.is_empty() {
            continue;
        }
        // Subtitle tolerance only when exactly one side carries one: the
        // library's "The Hollow: A Novel" still meets the device's "Hollow,
        // The", while "Star Wars: Heir" never meets "Star Wars: Hand".
        let same_title = id.title_key == want_title
            || match (&want_head, &id.title_head) {
                (Some(head), None) => *head == id.title_key,
                (None, Some(head)) => want_title == *head,
                _ => false,
            };
        let tier = if same_title {
            if !author_known || id.author_key == want_key {
                Tier::Exact
            } else {
                Tier::TitleOnly
            }
        } else if author_known
            && id.author_key == want_key
            && similarity(&id.title_key, &want_title) >= FUZZY_MIN
        {
            Tier::Fuzzy
        } else {
            continue;
        };
        scored.push(Candidate { identity: id, tier });
    }

    // One book, two containers: keep the kepub, drop the epub twin. Two
    // kepubs that still claim the same identity survive — that is a real
    // tie (two volumes, two directories) and the shell asks.
    let mut groups: std::collections::BTreeMap<(String, String), Vec<Candidate>> =
        std::collections::BTreeMap::new();
    for c in scored {
        groups
            .entry((c.identity.title_key.clone(), c.identity.author_key.clone()))
            .or_default()
            .push(c);
    }
    let mut candidates: Vec<Candidate> = Vec::new();
    for (_, mut group) in groups {
        let has_kepub = group.iter().any(|c| c.identity.kepub);
        if has_kepub {
            group.retain(|c| c.identity.kepub);
        }
        candidates.extend(group);
    }
    candidates.sort_by(|a, b| {
        a.tier
            .cmp(&b.tier)
            .then_with(|| b.identity.kepub.cmp(&a.identity.kepub))
            .then_with(|| a.identity.path.cmp(&b.identity.path))
    });

    let best = candidates.first().map(|c| c.tier);
    let author_matched = matches!(best, Some(Tier::Exact) | Some(Tier::Fuzzy));
    let auto = match best {
        Some(Tier::Exact) | Some(Tier::Fuzzy) => {
            let at_best = candidates.iter().filter(|c| c.tier == candidates[0].tier).count();
            if at_best == 1 {
                Some(candidates[0].identity.path.clone())
            } else {
                None
            }
        }
        _ => None,
    };

    Match { candidates, author_matched, auto, total_paths: paths.len() }
}

pub struct SshResult {
    pub output: String,
    pub code: i32,
}

pub fn koreader_open_cmd(chosen: &str) -> String {
    let esc = chosen.replace('\'', "'\\''").replace('"', "\\\"");
    format!(
        "if [ -x /mnt/onboard/.adds/koreader/koreader.sh ]; then K=/mnt/onboard/.adds/koreader/koreader.sh; else K=/mnt/onboard/koreader/koreader.sh; fi; \
         if [ ! -f '{esc}' ]; then echo \"not found: {esc}\"; exit 1; fi; \
         nohup \"$K\" '{esc}' >/tmp/koreader-open.log 2>&1 & sleep 1; \
         ps | grep -E \"koreader|reader.lua\" | head -n 5; echo \"launched: {esc}\""
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_sort_moves_article() {
        assert_eq!(title_sort("The Emma"), "Emma, The");
        assert_eq!(title_sort("A Study"), "Study, A");
        assert_eq!(title_sort("Emma"), "Emma");
    }

    #[test]
    fn sanitize_kobo_fat() {
        assert_eq!(sanitize("Smiley's"), "Smiley_s");
        assert_eq!(sanitize("Carré, John le"), "Carre, John le");
    }

    #[test]
    fn predicted_path_shape() {
        let p = predicted_path("Emma", "Austen, Jane", None);
        assert!(p.starts_with("/mnt/onboard/Austen, Jane/"));
        assert!(p.ends_with("Emma - Jane Austen.kepub.epub"));
    }

    // The eight "Hollow" books from the 2026-11-19 screenshot: one shared
    // word, eight unrelated books. None of them is a candidate for a
    // Michael Connelly title.
    fn hollow_device() -> Vec<String> {
        vec![
            "/mnt/onboard/Bellairs, George/Crime in Lepers_ Hollow - George Bellairs.kepub.epub",
            "/mnt/onboard/Christie, Agatha/Hollow, The - Agatha Christie.kepub.epub",
            "/mnt/onboard/Graham, Caroline/Death Of A Hollow Man - Caroline Graham.kepub.epub",
            "/mnt/onboard/Krueger, William Kent/Blood Hollow - William Kent Krueger.kepub.epub",
            "/mnt/onboard/Landy, Derek/Haunted House on Hollow Hill, The - Derek Landy.kepub.epub",
            "/mnt/onboard/Riggs, Ransom/Hollow City - Ransom Riggs.kepub.epub",
            "/mnt/onboard/Stewart, Mary/Hollow Hills, The - Mary Stewart.kepub.epub",
            "/mnt/onboard/Stroud, Jonathan/Hollow Boy, The - Jonathan Stroud.kepub.epub",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    }

    #[test]
    fn shared_word_is_not_a_candidate() {
        // The screenshot case: "The Hollow" by Connelly is not on the
        // device. Exactly one of the eight shares the *title* (Christie's);
        // the other seven shared only a word and must never leak in.
        let m = classify(&hollow_device(), "The Hollow", "Michael Connelly");
        assert_eq!(m.candidates.len(), 1, "only the title-equal book may appear");
        assert_eq!(m.candidates[0].tier, Tier::TitleOnly);
        assert!(m.candidates[0].identity.path.contains("Christie"));
        assert!(m.auto.is_none());
        assert!(!m.author_matched);
        assert_eq!(m.total_paths, 8);
    }

    #[test]
    fn title_and_author_pick_the_one_book() {
        let m = classify(&hollow_device(), "The Hollow", "Christie, Agatha");
        assert_eq!(m.candidates.len(), 1);
        assert_eq!(m.candidates[0].tier, Tier::Exact);
        assert_eq!(
            m.auto.as_deref(),
            Some("/mnt/onboard/Christie, Agatha/Hollow, The - Agatha Christie.kepub.epub")
        );
    }

    #[test]
    fn author_substring_no_longer_matches() {
        // "hollow" used to hit the author name Holloway via `contains`.
        let paths = vec![
            "/mnt/onboard/Holloway, Jane/Other Book - Jane Holloway.kepub.epub".to_string(),
        ];
        let m = classify(&paths, "Hollow", "Someone Else");
        assert!(m.candidates.is_empty());
    }

    #[test]
    fn kepub_wins_over_epub_twin() {
        let paths = vec![
            "/mnt/onboard/Austen, Jane/Emma - Jane Austen.epub".to_string(),
            "/mnt/onboard/Austen, Jane/Emma - Jane Austen.kepub.epub".to_string(),
        ];
        let m = classify(&paths, "Emma", "Jane Austen");
        assert_eq!(m.candidates.len(), 1);
        assert!(m.candidates[0].identity.kepub);
        assert!(m.auto.as_deref().unwrap().ends_with(".kepub.epub"));
    }

    #[test]
    fn two_kepubs_same_identity_tie() {
        // Two volumes synced into different directories: a genuine tie.
        let paths = vec![
            "/mnt/onboard/Austen, Jane/Emma - Jane Austen.kepub.epub".to_string(),
            "/mnt/onboard/_backup/Austen, Jane/Emma - Jane Austen.kepub.epub".to_string(),
        ];
        let m = classify(&paths, "Emma", "Jane Austen");
        assert_eq!(m.candidates.len(), 2);
        assert!(m.auto.is_none());
        assert!(m.author_matched);
    }

    #[test]
    fn title_subtitle_and_articles_collapse() {
        // calibre title_sort form + a library subtitle must still meet.
        let paths = vec![
            "/mnt/onboard/Christie, Agatha/Hollow, The - Agatha Christie.kepub.epub".to_string(),
        ];
        let m = classify(&paths, "The Hollow: A Novel", "Agatha Christie");
        assert_eq!(m.candidates.len(), 1);
        assert_eq!(m.candidates[0].tier, Tier::Exact);
    }

    #[test]
    fn diacritics_and_initials() {
        let paths = vec![
            "/mnt/onboard/Carre, John le/Tinker Tailor Soldier Spy - John le Carre.kepub.epub"
                .to_string(),
        ];
        let m = classify(&paths, "Tinker Tailor Soldier Spy", "John le Carré");
        assert_eq!(m.candidates.len(), 1);
        assert_eq!(m.candidates[0].tier, Tier::Exact);
    }

    #[test]
    fn fuzzy_title_with_matching_author() {
        let paths = vec![
            "/mnt/onboard/Christie, Agatha/Hollows, The - Agatha Christie.kepub.epub".to_string(),
        ];
        let m = classify(&paths, "The Hollow", "Agatha Christie");
        assert_eq!(m.candidates.len(), 1);
        assert_eq!(m.candidates[0].tier, Tier::Fuzzy);
        assert!(m.auto.is_some());
    }

    #[test]
    fn different_author_is_offered_not_auto() {
        let paths = vec![
            "/mnt/onboard/Christie, Agatha/Hollow, The - Agatha Christie.kepub.epub".to_string(),
        ];
        let m = classify(&paths, "The Hollow", "Michael Connelly");
        assert_eq!(m.candidates.len(), 1);
        assert_eq!(m.candidates[0].tier, Tier::TitleOnly);
        assert!(m.auto.is_none());
        assert!(!m.author_matched);
    }

    #[test]
    fn parallel_subtitled_volumes_stay_distinct() {
        // Same author, same head before the colon: only the full title may
        // decide, or a series would collapse into one book.
        let paths = vec![
            "/mnt/onboard/Zahn, Timothy/Star Wars_ Heir To The Empire - Timothy Zahn.kepub.epub"
                .to_string(),
            "/mnt/onboard/Zahn, Timothy/Star Wars_ Dark Force Rising - Timothy Zahn.kepub.epub"
                .to_string(),
        ];
        let m = classify(&paths, "Star Wars: Dark Force Rising", "Timothy Zahn");
        assert_eq!(m.candidates.len(), 1);
        assert!(m.auto.as_deref().unwrap().contains("Dark Force Rising"));

        let m2 = classify(&paths, "Star Wars: The Last Command", "Timothy Zahn");
        assert!(m2.candidates.is_empty());
    }

    #[test]
    fn unparseable_paths_are_dropped() {
        let paths = vec![
            "/mnt/onboard/.kobo/whatever".to_string(),
            "/mnt/onboard/Christie, Agatha/._Hollow, The - Agatha Christie.kepub.epub".to_string(),
            "/mnt/onboard/Christie, Agatha/notes.txt".to_string(),
        ];
        let m = classify(&paths, "The Hollow", "Agatha Christie");
        assert!(m.candidates.is_empty());
    }
}
