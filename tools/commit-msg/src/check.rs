//! The message and identity rules of the shared `No AI attribution` policy
//! (ryanduguid/release-policy, .github/actions/no-ai-attribution/action.yml),
//! ported from the Python copy this hook carried before. The vendor list, the
//! credit patterns, the Unicode folding and the identity rules must agree
//! with that action, so the hook is never laxer than it.
//!
//! Two extra readings, both fail-closed and shared with the action:
//!
//! - Comment lines are scanned. `git commit -m` and `-F` use `whitespace`
//!   cleanup, which keeps commentary in the stored message, so a credit hidden
//!   behind a comment marker reaches history. This hook peels the markers the
//!   clone has configured; the action, which cannot know them, peels every
//!   marker Git may use. Patterns are anchored, so an editor template
//!   ("# On branch claude/...") still does not match.
//! - Folded trailer continuations are scanned twice: as the physical line they
//!   are, and again joined onto the trailer they continue, so a credit split
//!   across 2 lines is caught and the join cannot hide one that starts the
//!   indented line.
//!
//! Product references stay allowed: "publish the skills as a Claude Code
//! plugin" is a subject line, not a credit.
//!
//! Patterns are written in Python's regex syntax and translated by
//! [`translate`]: `\w`, `\W`, `\s` and `\b` take Python's `str` meaning
//! (`isalnum()` or `_`, `isspace()`), which differs from the regex crate's
//! Unicode classes.

use std::collections::HashSet;
use std::sync::LazyLock;

use fancy_regex::{Match, Regex};
use unicode_normalization::UnicodeNormalization;

use crate::{pytables, tables};

fn in_ranges(table: &[(u32, u32)], c: char) -> bool {
    let cp = c as u32;
    let i = table.partition_point(|&(_, hi)| hi < cp);
    table.get(i).is_some_and(|&(lo, _)| lo <= cp)
}

fn is_space(c: char) -> bool {
    in_ranges(pytables::SPACE, c)
}

fn is_printable(c: char) -> bool {
    in_ranges(pytables::PRINTABLE, c)
}

fn class_body(table: &[(u32, u32)]) -> String {
    let mut body = String::new();
    for &(lo, hi) in table {
        body.push_str(&format!("\\x{{{lo:X}}}-\\x{{{hi:X}}}"));
    }
    body
}

static WORD: LazyLock<String> = LazyLock::new(|| class_body(pytables::ALNUM) + "_");
static SPACE: LazyLock<String> = LazyLock::new(|| class_body(pytables::SPACE));

/// Rewrite Python's `\w`, `\W`, `\s` and `\b` as explicit classes, inside and
/// outside brackets. Everything else passes through unchanged. Each class
/// holding one is matched case-sensitively (`(?-i:...)`): the word and space
/// sets are closed under case already, and folding about 800 ranges in every
/// pattern would triple the guard's start-up time.
fn translate(pattern: &str) -> String {
    let (w, s) = (WORD.as_str(), SPACE.as_str());
    let mut out = String::new();
    let mut class: Option<(String, bool)> = None;
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        if let Some((body, rewritten)) = class.as_mut() {
            match c {
                '\\' => match chars.next().expect("pattern ends in a backslash") {
                    'w' => {
                        body.push_str(w);
                        *rewritten = true;
                    }
                    's' => {
                        body.push_str(s);
                        *rewritten = true;
                    }
                    'W' | 'b' => panic!("unsupported escape in a class: {pattern}"),
                    other => {
                        body.push('\\');
                        body.push(other);
                    }
                },
                ']' => {
                    if *rewritten {
                        out.push_str(&format!("(?-i:[{body}])"));
                    } else {
                        out.push_str(&format!("[{body}]"));
                    }
                    class = None;
                }
                _ => body.push(c),
            }
            continue;
        }
        match c {
            '\\' => match chars.next().expect("pattern ends in a backslash") {
                'w' => out.push_str(&format!("(?-i:[{w}])")),
                'W' => out.push_str(&format!("(?-i:[^{w}])")),
                's' => out.push_str(&format!("(?-i:[{s}])")),
                'b' => out.push_str(&format!("(?-i:(?<=[{w}])(?![{w}])|(?<![{w}])(?=[{w}]))")),
                other => {
                    out.push('\\');
                    out.push(other);
                }
            },
            '[' => {
                let mut body = String::new();
                // A leading `^` or `]` belongs to the class, not its end.
                if chars.peek() == Some(&'^') {
                    body.push(chars.next().expect("peeked"));
                }
                if chars.peek() == Some(&']') {
                    chars.next();
                    body.push_str("\\]");
                }
                class = Some((body, false));
            }
            _ => out.push(c),
        }
    }
    assert!(class.is_none(), "unclosed class: {pattern}");
    out
}

fn rx(pattern: &str) -> Regex {
    Regex::new(&translate(pattern)).expect("valid pattern")
}

/// Case-insensitive, as Python's `re.I`.
fn rxi(pattern: &str) -> Regex {
    rx(&format!("(?i:{pattern})"))
}

/// Python's `fullmatch`.
fn rxi_full(pattern: &str) -> Regex {
    rx(&format!("\\A(?i:{pattern})\\z"))
}

/// Python's `re.match`: a match that starts at the beginning. The leftmost
/// match is the one starting there whenever one does.
fn at_start<'t>(re: &Regex, text: &'t str) -> Option<Match<'t>> {
    re.find(text).ok().flatten().filter(|m| m.start() == 0)
}

fn found(re: &Regex, text: &str) -> bool {
    re.is_match(text).unwrap_or(false)
}

fn matches<'t>(re: &Regex, text: &'t str) -> Vec<Match<'t>> {
    re.find_iter(text).filter_map(Result::ok).collect()
}

/// The last piece of Python's `re.split(pattern, text)`.
fn last_piece<'t>(re: &Regex, text: &'t str) -> &'t str {
    let end = matches(re, text).last().map_or(0, Match::end);
    &text[end..]
}

const VENDOR: &str = "(?:claude|anthropic|codex|openai|cursor|hermes|nousresearch|\
copilot|chatgpt|gemini|devin|codeium|windsurf|antigravity|aider|qodo|openhands|cascade)";

const DOMAIN: &str = r"(?:claude\.ai|claude\.com|anthropic\.com|openai\.com|chatgpt\.com|cursor\.com|cursor\.sh|codeium\.com|windsurf\.com|cognition\.ai|devin\.ai|nousresearch\.com|qodo\.ai|openhands\.dev|all-hands\.dev)";

static AI_NAME: LazyLock<String> = LazyLock::new(|| {
    format!(
        r"(?:anthropic|codex|openai|cursor|nousresearch|copilot|chatgpt|codeium|windsurf|antigravity|aider|qodo|openhands)|{VENDOR}(?:[-_ ](?:agent|code|bot)(?:\[bot\])?|\[bot\])|{VENDOR}(?:[-_ ][a-z0-9]+)+\[bot\]|claude (?:opus|sonnet|haiku|fable)(?: [0-9.]+)?|hermes review|review (?:bot|agent|fixes)|(?:openai[-_ ]codex|github[-_ ]copilot|qodo[-_ ]code[-_ ]review|gemini[-_ ]code[-_ ]assist)(?:\[bot\])?"
    )
});

const CREDIT_VERB: &str = "(?:generated|addressed|authored|written|created|produced|\
reviewed|fixed|resolved|implemented|assisted|handled)";

const BLOCKED_EMAILS: [&str; 3] = [
    "noreply@anthropic.com",
    "openhands@all-hands.dev",
    "cascade@windsurf.com",
];

static RX: LazyLock<Vec<(&str, Regex)>> = LazyLock::new(|| {
    let v = VENDOR;
    let c = CREDIT_VERB;
    [
        (
            "co-author trailer",
            format!(r"^\s*co-?authored-by:\s*.*{v}"),
        ),
        ("agent by-line trailer", format!(r"^\s*{c}[-_ ]by:\s*.*{v}")),
        (
            "session trailer",
            format!(r"^\s*{v}[-_ ]?(?:session|run|thread|trace|id):"),
        ),
        ("robot emoji credit", format!(r"^\s*_?\s*\x{{1F916}}.*{v}")),
        (
            "generated-with footer",
            format!(r"^\s*_?{c}\s+(?:with|by)\s+\[?[^\]\n]*{v}"),
        ),
        (
            "bare vendor url",
            format!(r"^\s*[_<(]*https?://(?:[\w.-]+\.)?{DOMAIN}\b"),
        ),
    ]
    .into_iter()
    .map(|(name, p)| (name, rxi(&p)))
    .collect()
});

static FENCE: LazyLock<Regex> =
    LazyLock::new(|| rx(r"^(?P<indent> {0,3})(?P<fence>`{3,}|~{3,})(?P<info>.*)$"));

// A trailer key, so an indented line following one can be read as its
// continuation.
static TRAILER_START: LazyLock<Regex> = LazyLock::new(|| rx(r"^\s*[A-Za-z][A-Za-z0-9-]*:"));

/// The characters Git chooses between for `core.commentChar = auto`.
const AUTO_MARKERS: [&str; 10] = ["#", ";", "@", "!", "$", "%", "^", "&", "|", ":"];

/// Compare visible letters without hidden characters or combining marks.
///
/// Code points Unicode 17 leaves unassigned are dropped before decomposing:
/// Python 3.15's NFKD leaves them alone and its filter then drops them,
/// whereas the newer Unicode data in `unicode-normalization` may decompose one.
pub fn comparable(text: &str) -> String {
    text.chars()
        .filter(|&c| !in_ranges(tables::UNASSIGNED, c))
        .nfkd()
        .filter(|&c| (is_printable(c) || is_space(c)) && !in_ranges(tables::MARK, c))
        .map(|c| {
            if is_space(c) {
                ' '
            } else if in_ranges(tables::DASH_PUNCTUATION, c) {
                '-'
            } else {
                c
            }
        })
        .collect()
}

fn strip(s: &str) -> &str {
    s.trim_matches(is_space)
}

fn lstrip(s: &str) -> &str {
    s.trim_start_matches(is_space)
}

/// Every marker Git might treat as starting a comment line: each configured
/// value of core.commentString and core.commentChar (an unset or empty value
/// adds none; `auto` adds every character Git may pick), `#` when none is
/// configured, and the folded form of each marker that folds to something
/// else. A marker that folds to nothing is dropped: it would match at every
/// position. The rule throughout is to scan more lines, never fewer.
pub fn comment_markers(comment_string: &str, comment_char: &str) -> Vec<String> {
    let mut markers: Vec<String> = Vec::new();
    for configured in [comment_string, comment_char] {
        let configured = strip(configured);
        if configured.is_empty() {
            continue;
        }
        if configured.to_lowercase() == "auto" {
            markers.extend(AUTO_MARKERS.iter().map(|m| (*m).to_owned()));
        } else {
            markers.push(configured.to_owned());
        }
    }
    if markers.is_empty() {
        markers.push("#".to_owned());
    }
    for marker in markers.clone() {
        let folded = comparable(&marker);
        let folded = strip(&folded);
        if !folded.is_empty() && folded != marker {
            markers.push(folded.to_owned());
        }
    }
    let mut seen = HashSet::new();
    markers.retain(|m| seen.insert(m.clone()));
    markers
}

/// The line as written, then again with each comment marker peeled off,
/// repeated markers in turn, every matching marker explored.
fn readings(text: &str, markers: &[String]) -> Vec<String> {
    let mut found = vec![text.to_owned()];
    let mut seen: HashSet<String> = HashSet::from([text.to_owned()]);
    let mut i = 0;
    while i < found.len() {
        let rest = lstrip(&found[i]).to_owned();
        for marker in markers {
            if let Some(after) = rest.strip_prefix(marker.as_str()) {
                let candidate = lstrip(after).to_owned();
                if seen.insert(candidate.clone()) {
                    found.push(candidate);
                }
            }
        }
        i += 1;
    }
    found
}

// --- prose credit grammar, ported from the shared action -------------------
const SEP: &str = r"[^\w;|\n]+";
const PRODUCT: &str = r"(?:API|SDK|clients?|integrations?|models?|parsers?|prompts?)";
const DATA_NOUN: &str =
    r"(?:errors?|responses?|requests?|pagination|tokens?|records?|IDs?|JSON|XML|CSV|data)";
const AUTHOR_PREFIX: &str =
    r"^(?:['\x{2019}]s)?(?:\W|_)*(?:(?:as|is|was|are|were|who|the|a|an)\s+)*";
const AUTHOR: &str = r"(?:co[- ]?author(?:ed|s)?|author(?:ed|s)?|wrote)";
const HANDLING: &str = r"\b(?:add(?:s|ed|ing)?|updat(?:e|es|ed|ing)|fix(?:es|ed|ing)?|handl(?:e|es|ed|ing)|pars(?:e|es|ed|ing)|validat(?:e|es|ed|ing)|reject(?:s|ed|ing)?|block(?:s|ed|ing)?|detect(?:s|ed|ing)?|filter(?:s|ed|ing)?|remov(?:e|es|ed|ing)|ignor(?:e|es|ed|ing)|test(?:s|ed|ing)?|prevent(?:s|ed|ing)?|support(?:s|ed|ing)?|enabl(?:e|es|ed|ing))\b\s*[:\-]?\s*(?:(?!\s+-\s)[^.!?,:;|\n])*$";

static TOOL: LazyLock<String> = LazyLock::new(|| {
    format!(
        r"(?<!\w)(?:{ai}|(?:github(?:['\x{{2019}}]s)?{SEP})?{VENDOR})(?!\w)(?:{SEP}(?:code|agent|bot|assist|review|codex)(?!\w))*(?:[- ]+[0-9]+(?:\.[0-9]+)*)?(?!\w)",
        ai = *AI_NAME
    )
});
const DESCRIPTOR_WORDS: &str = "(?:the|ai|assistant|agent|bot|and|by|with|via|using)";
static DESCRIPTOR: LazyLock<String> = LazyLock::new(|| format!("(?:{DESCRIPTOR_WORDS}{SEP})*"));
static NAME_END: LazyLock<String> = LazyLock::new(|| {
    format!(r"(?!(?:['\x{{2019}}]s)?(?:\s+|-)(?:{PRODUCT}|pagination|deletion|support)\b)")
});

struct Prose {
    title_credit: Regex,
    reverse_credit: Regex,
    coauthors: Regex,
    credit_relation: Regex,
    technical: Regex,
    handling: Regex,
    handling_starts: Regex,
    output_phrase: Regex,
    author_statement: Regex,
    qualifier: Regex,
    product_words: Regex,
    byline: Regex,
    tool: Regex,
    tool_name: Regex,
    name_split: Regex,
    credit_prefix: Regex,
    policy_object: Regex,
    data_verb: Regex,
    data_object: Regex,
    feature: Regex,
    pronoun: Regex,
    product_suffix: Regex,
    clause_split: Regex,
    together: Regex,
}

static PROSE: LazyLock<Prose> = LazyLock::new(|| {
    let (tool, desc, name_end) = (TOOL.as_str(), DESCRIPTOR.as_str(), NAME_END.as_str());
    Prose {
        title_credit: rxi(&format!(
            r"(?<!\w){CREDIT_VERB}{SEP}(?:by|with|via|using){SEP}{desc}{tool}"
        )),
        reverse_credit: rxi(&format!(r"{tool}{SEP}{CREDIT_VERB}(?!\w)")),
        coauthors: rxi(&format!(r"\bco[-_ ]?authored{SEP}by{SEP}")),
        credit_relation: rxi(&format!(
            r"(?<!\w){CREDIT_VERB}{SEP}(?:by|with|via|using){SEP}"
        )),
        technical: rxi(&format!(
            r"\bimplemented{SEP}(?:with|via|using){SEP}{desc}(?:cursor[- ]+(?:based[- ]+)?pagination|cascade[- ]+deletion|{tool}(?:['\x{{2019}}]s)?(?:\s+|-){PRODUCT})\b|\b{DATA_NOUN}{SEP}generated{SEP}by{SEP}{desc}{tool}(?:['\x{{2019}}]s)?(?:\s+|-){PRODUCT}\b"
        )),
        handling: rxi(HANDLING),
        handling_starts: rxi(&format!("(?=({HANDLING}))")),
        output_phrase: rxi_full(&format!(
            r"{tool}\s*[-:]\s*{CREDIT_VERB}(?!\w)|{tool}{SEP}(?:code{SEP})?review{SEP}implemented"
        )),
        author_statement: rxi(&format!(r"{AUTHOR_PREFIX}{AUTHOR}\b")),
        qualifier: rxi(&format!(r"{AUTHOR_PREFIX}(?:{AUTHOR}|assistance|help)\b")),
        product_words: rxi(r"\b(?:integration|support|pars(?:e|es|ed|ing))\b"),
        byline: rxi(&format!(
            r"\b(?P<relation>by|with|via|using|and){SEP}{desc}{tool}{name_end}"
        )),
        tool: rxi(tool),
        tool_name: rxi(&format!("{tool}{name_end}")),
        name_split: rxi(r"\s+-\s+|[;|,\n]|\band\b"),
        credit_prefix: rxi(r"\b(?:support|assistance|help)\s+from\s*$"),
        policy_object: rxi(&format!(
            r"{SEP}(?:commits?|PRs?|pull requests?|credits?|attributions?|authorship|trailers?|identities|contributors?)\b"
        )),
        data_verb: rxi(r"\b(?:generated|created|produced)$"),
        data_object: rxi(&format!(r"{SEP}(?:API\s+)?{DATA_NOUN}\b")),
        feature: rxi(r"\breview\s+implemented$"),
        pronoun: rxi(r"\s+(?:this|the|these|those|it|them|our|my)\b"),
        product_suffix: rxi(&format!(
            r"(?:['\x{{2019}}]s)?(?:(?:\s+|-)(?:{PRODUCT}|pagination|deletion)\b)+"
        )),
        clause_split: rx(r"\.(?=\s|$)|[,;|\n]"),
        together: rxi(r"\b(?:together|alongside)\s*$"),
    }
});

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Technical,
    Reverse,
}

#[derive(Clone, Copy)]
struct Span {
    start: usize,
    end: usize,
    kind: Kind,
}

fn inside(objects: &[Span], m: &Match) -> bool {
    objects
        .iter()
        .any(|o| o.start <= m.start() && m.end() <= o.end)
}

/// The last group 1 of `(?=(handling))` over `text`: Python's
/// `list(handling_starts.finditer(text))[-1][1]`.
fn last_action(p: &Prose, text: &str) -> Option<String> {
    let mut pos = 0;
    let mut last = None;
    while pos <= text.len() {
        let Some(caps) = p
            .handling_starts
            .captures_from_pos(text, pos)
            .ok()
            .flatten()
        else {
            break;
        };
        let start = caps.get(0).map_or(pos, |m| m.start());
        last = caps.get(1).map(|m| m.as_str().to_owned());
        pos = start + text[start..].chars().next().map_or(1, char::len_utf8);
    }
    last
}

const POLICY_ACTIONS: [&str; 9] = [
    "reject", "block", "detect", "filter", "remov", "ignor", "test", "validat", "prevent",
];

/// Whether `text` carries prose authorship credit.
pub fn prose_credit(text: &str) -> bool {
    let p = &*PROSE;
    let text = comparable(text).replace('_', " ");
    let text = text.as_str();
    // Classify each match in the original text; removing words can join clauses.
    let mut objects: Vec<Span> = matches(&p.technical, text)
        .iter()
        .map(|m| Span {
            start: m.start(),
            end: m.end(),
            kind: Kind::Technical,
        })
        .collect();
    if matches(&p.title_credit, text)
        .iter()
        .any(|m| !inside(&objects, m))
    {
        return true;
    }
    let mut relations = matches(&p.coauthors, text);
    relations.extend(matches(&p.credit_relation, text));
    for m in relations {
        if inside(&objects, &m) {
            continue;
        }
        // Read names in place so punctuation inside a decoration cannot split one.
        let tail = &text[m.end()..];
        for name in matches(&p.tool_name, tail) {
            let local = lstrip(last_piece(&p.name_split, &tail[..name.start()]));
            let task = at_start(&p.handling, local).is_some();
            let credit_prefix = found(&p.credit_prefix, local);
            if task && !credit_prefix && at_start(&p.qualifier, &tail[name.end()..]).is_none() {
                continue;
            }
            return true;
        }
    }
    for m in matches(&p.reverse_credit, text) {
        let after = &text[m.end()..];
        let action = last_action(p, &text[..m.start()]);
        let policy_action = action.as_ref().is_some_and(|a| {
            let a = a.to_lowercase();
            POLICY_ACTIONS.iter().any(|prefix| a.starts_with(prefix))
        }) && at_start(&p.policy_object, after).is_some();
        let data_output =
            found(&p.data_verb, m.as_str()) && at_start(&p.data_object, after).is_some();
        let feature = found(&p.feature, m.as_str());
        if action.is_some()
            && (policy_action || (found(&p.output_phrase, m.as_str()) && (data_output || feature)))
            && at_start(&p.pronoun, after).is_none()
        {
            objects.push(Span {
                start: m.start(),
                end: m.end(),
                kind: Kind::Reverse,
            });
            continue;
        }
        return true;
    }
    for name in matches(&p.tool, text) {
        if objects
            .iter()
            .any(|o| o.kind == Kind::Reverse && o.start <= name.start() && name.end() <= o.end)
        {
            continue;
        }
        let mut suffix = &text[name.end()..];
        if let Some(product) = at_start(&p.product_suffix, suffix) {
            suffix = &suffix[product.end()..];
        }
        if at_start(&p.author_statement, suffix).is_some() {
            return true;
        }
    }
    for caps in p.byline.captures_iter(text).filter_map(Result::ok) {
        let m = caps.get(0).expect("group 0");
        if inside(&objects, &m) {
            continue;
        }
        let earlier: Vec<&Span> = objects.iter().filter(|o| o.end <= m.start()).collect();
        let start = earlier.iter().map(|o| o.end).max().unwrap_or(0);
        let prefix = &text[start..m.start()];
        let local = last_piece(&p.clause_split, prefix);
        let relation = caps
            .name("relation")
            .expect("relation")
            .as_str()
            .to_lowercase();
        // Explicit author wording takes priority over a product task.
        let explicit = relation == "by"
            || at_start(&p.qualifier, &text[m.end()..]).is_some()
            || found(&p.together, local);
        let same_clause = !earlier.is_empty() && !found(&p.clause_split, prefix);
        let mut context = same_clause || found(&p.credit_relation, local);
        if relation != "and" {
            context = context || !earlier.is_empty() || found(&p.handling, local);
        }
        if explicit || (context && !found(&p.product_words, local)) {
            return true;
        }
    }
    // Explicit credit grammar; ambiguous prose still needs review. Add a
    // pattern for any attribution wording found to slip past.
    false
}

fn chars_prefix(s: &str, n: usize) -> &str {
    s.char_indices().nth(n).map_or(s, |(i, _)| &s[..i])
}

/// Fence state over the physical lines, before any trailer folding: a fence
/// delimiter is skipped, as is every line inside an open block.
fn fenced(lines: &[&str]) -> Vec<bool> {
    let mut skip = vec![false; lines.len()];
    let mut open: Option<(char, usize)> = None;
    for (index, line) in lines.iter().enumerate() {
        let caps = FENCE.captures(line).ok().flatten();
        // CommonMark: a backtick fence's info string may not contain a backtick.
        let caps = caps.filter(|c| {
            !(open.is_none()
                && c.name("fence").map_or("", |m| m.as_str()).starts_with('`')
                && c.name("info").map_or("", |m| m.as_str()).contains('`'))
        });
        if let Some(c) = caps {
            let fence = c.name("fence").map_or("", |m| m.as_str());
            let (ch, len) = (fence.chars().next().expect("fence"), fence.len());
            match open {
                None => {
                    open = Some((ch, len));
                    skip[index] = true;
                    continue;
                }
                Some((oc, ol))
                    if ch == oc
                        && len >= ol
                        && strip(c.name("info").map_or("", |m| m.as_str())).is_empty() =>
                {
                    open = None;
                    skip[index] = true;
                    continue;
                }
                Some(_) => {}
            }
        }
        skip[index] = open.is_some();
    }
    skip
}

/// The report line for each line of `message` that carries credit.
pub fn message_failures(message: &str, markers: &[String]) -> Vec<String> {
    let lines: Vec<&str> = message.split('\n').collect();
    let skip = fenced(&lines);
    let mut failures = Vec::new();
    let mut folded: Option<(String, usize)> = None;
    for (index, line) in lines.iter().enumerate() {
        let number = index + 1;
        if skip[index] {
            folded = None;
            continue;
        }
        // Every physical line is scanned on its own. A continuation is scanned a
        // second time joined onto its trailer, so folding can only add a detection.
        let mut candidates = vec![(line.to_string(), number)];
        match folded.take() {
            Some((text, at)) if line.starts_with([' ', '\t']) => {
                let joined = format!("{text} {}", strip(line));
                candidates.push((joined.clone(), at));
                folded = Some((joined, at));
            }
            _ if at_start(&TRAILER_START, line).is_some() => {
                folded = Some((line.to_string(), number));
            }
            _ => {}
        }
        for (text, at) in candidates {
            // Markers are peeled from the line as written and again from the
            // folded line: either order can be the one that exposes a credit.
            let mut all = readings(&text, markers);
            all.extend(readings(&comparable(&text), markers));
            let hit = all.iter().find_map(|reading| {
                let reading = comparable(reading);
                let shown = chars_prefix(strip(&text), 120);
                RX.iter()
                    .find(|(_, re)| at_start(re, &reading).is_some())
                    .map(|(name, _)| format!("line {at}: {shown} ({name})"))
                    .or_else(|| {
                        prose_credit(&reading)
                            .then(|| format!("line {at}: {shown} (authorship credit)"))
                    })
            });
            if let Some(hit) = hit {
                failures.push(hit);
                break;
            }
        }
    }
    failures
}

static IDENT: LazyLock<Regex> =
    LazyLock::new(|| rx(r"\A([^<>\n]*) <([^<>\n]*)> -?[0-9]+ [+-][0-9]{4}\z"));
static AI_NAME_FULL: LazyLock<Regex> = LazyLock::new(|| rxi_full(&AI_NAME));
static ACCOUNT: LazyLock<Regex> =
    LazyLock::new(|| rxi_full(r"(?:[0-9]+\+)?([^@]+)@users\.noreply\.github\.com"));

/// The failure for an author or committer identity, if any. A non-printable
/// identity is refused before normalising, as the shared action does.
pub fn identity_failure(role: &str, ident: &str) -> Option<String> {
    let Some(caps) = IDENT.captures(ident).ok().flatten() else {
        return Some(format!("{role} identity is malformed: {ident}"));
    };
    let (name, email) = (
        caps.get(1).map_or("", |m| m.as_str()),
        caps.get(2).map_or("", |m| m.as_str()),
    );
    if name
        .chars()
        .chain(email.chars())
        .any(|c| !is_printable(c) || c == '\u{FFFD}')
    {
        return Some(format!("{role} identity carries a non-printable character"));
    }
    let name = comparable(name);
    let email = comparable(email);
    let (name, email) = (strip(&name), strip(&email));
    let account = ACCOUNT.captures(email).ok().flatten();
    let blocked = BLOCKED_EMAILS.contains(&email.to_lowercase().as_str());
    let ai = |s: &str| found(&AI_NAME_FULL, s);
    if ai(name) || blocked || account.is_some_and(|a| ai(a.get(1).map_or("", |m| m.as_str()))) {
        return Some(format!("AI {role} identity: {name} <{email}>"));
    }
    None
}

/// Python's text-mode read: invalid UTF-8 replaced, then universal newlines.
pub fn decode(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}

/// Every failure for a message and the 2 identities, in report order.
pub fn failures(
    message: &[u8],
    author: &str,
    committer: &str,
    comment_string: &str,
    comment_char: &str,
) -> Vec<String> {
    let markers = comment_markers(comment_string, comment_char);
    let mut failures = message_failures(&decode(message), &markers);
    failures.extend(identity_failure("author", author));
    failures.extend(identity_failure("committer", committer));
    failures
}

pub const REFUSAL: &str = "
Commit refused: this repository does not carry AI authorship credit.

Remove the reported trailer or footer from the message. If the report names an
author or committer identity, set a human identity for this repository:

    git config user.name  \"Your Name\"
    git config user.email \"you@example.com\"

For a commit that already exists, retry with:

    git commit --amend --reset-author

";

/// The text printed to stderr for a non-empty failure list.
pub fn report(failures: &[String]) -> String {
    let mut out = String::from("commit-msg: AI attribution found:\n\n");
    for failure in failures {
        out.push_str("  ");
        out.push_str(failure);
        out.push('\n');
    }
    out.push_str(REFUSAL);
    out
}
