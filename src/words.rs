// The bad-word filter (Engine/Core/Text/word_filter.cpp): the list in bad_words.txt, matched
// after folding case and look-alike characters, so "Sh1t", "@$$" and "f u c k" count too.
use std::collections::HashSet;
use std::sync::OnceLock;

const BAD_WORDS: &str = include_str!("bad_words.txt");

// Letters fold to lower case and look-alikes to the letter they stand for; anything else
// separates words.
fn fold(c: u8) -> u8 {
    match c {
        b'a'..=b'z' => c,
        b'A'..=b'Z' => c - b'A' + b'a',
        b'0' => b'o',
        b'1' => b'i',
        b'3' => b'e',
        b'4' => b'a',
        b'5' => b's',
        b'7' => b't',
        b'8' => b'b',
        b'@' => b'a',
        b'$' => b's',
        b'+' => b't',
        _ => 0,
    }
}

// Longer list words that are common inside ordinary ones: these only count as a whole word.
const WHOLE_WORD_ONLY: &[&str] = &[
    "anus", "arse", "ayir", "bich", "breasts", "cawk", "cawks", "chuj", "cipa", "crap", "dego", "dike", "dupa", "ekto",
    "faen", "faig", "faigs", "fanny", "fart", "fitt", "flipping", "gays", "gayz", "gook", "hell", "hells", "hoar", "hoer",
    "hoor", "hore", "injun", "jiss", "kawk", "knob", "knobs", "knobz", "kraut", "kunt", "kunts", "kuntz", "kusi", "merd",
    "muie", "nastt", "nasty", "packi", "packie", "packy", "paki", "pakie", "paky", "paska", "perse", "picka", "pillu",
    "polac", "polak", "poop", "pric", "prik", "pron", "pula", "pule", "pusse", "puta", "puto", "rape", "raped", "rapes",
    "raping", "rapist", "rapists", "rautenberg", "schaffer", "screw", "screwing", "semen", "shiz", "smut", "teets", "teez",
    "tits", "titt", "turd", "woose",
];

// Ordinary words with a bad word inside; a match that one of these covers does not count.
const ALLOWED_WORDS: &[&str] = &[
    "scunthorpe", "dickens", "dickinson", "dickson", "dickies", "cocktail", "cockpit", "peacock", "cockroach", "cockatoo",
    "cockatiel", "cockney", "cockle", "cockerel", "cockburn", "cocky", "hancock", "hitchcock", "woodcock", "babcock",
    "shuttlecock", "ashkenazi", "shiitake", "shitake", "matsushita", "yamashita", "kinoshita", "morishita", "takeshita",
    "swank", "swanky", "woodpecker", "clitheroe", "snigger", "pussycat", "pissarro", "retardant",
];

struct Lists {
    whole: HashSet<String>,
    inside: HashSet<String>,
    shortest_inside: usize,
    longest_inside: usize,
}

fn lists() -> &'static Lists {
    static BUILT: OnceLock<Lists> = OnceLock::new();
    BUILT.get_or_init(|| {
        let mut l = Lists { whole: HashSet::new(), inside: HashSet::new(), shortest_inside: usize::MAX, longest_inside: 0 };
        for line in BAD_WORDS.split('\n') {
            let word: String = line.bytes().map(fold).filter(|&c| c != 0).map(char::from).collect();
            if word.is_empty() {
                continue;
            }
            l.whole.insert(word.clone());
            if word.len() > 3 && !WHOLE_WORD_ONLY.contains(&word.as_str()) {
                l.shortest_inside = l.shortest_inside.min(word.len());
                l.longest_inside = l.longest_inside.max(word.len());
                l.inside.insert(word);
            }
        }
        l
    })
}

// A word as folded letters, with where each letter sits in the text.
#[derive(Default)]
struct Word {
    letters: String,
    at: Vec<usize>,
}

fn words(text: &[u8]) -> Vec<Word> {
    let mut out: Vec<Word> = Vec::new();
    let mut current = Word::default();
    for (i, &c) in text.iter().enumerate() {
        let letter = fold(c);
        if letter != 0 {
            current.letters.push(char::from(letter));
            current.at.push(i);
        } else if !current.letters.is_empty() {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.letters.is_empty() {
        out.push(current);
    }
    // Three or more letters spelled out one at a time ("f u c k", "s.o.b.") also read as a word.
    let mut spelled = Vec::new();
    let mut i = 0;
    while i < out.len() {
        let mut end = i;
        while end < out.len() && out[end].letters.len() == 1 {
            end += 1;
        }
        if end - i >= 3 {
            let mut joined = Word::default();
            for word in &out[i..end] {
                joined.letters.push_str(&word.letters);
                joined.at.push(word.at[0]);
            }
            spelled.push(joined);
        }
        i = if end == i { i + 1 } else { end };
    }
    out.extend(spelled);
    out
}

fn covered(letters: &str, start: usize, length: usize) -> bool {
    for allowed in ALLOWED_WORDS {
        let mut from = 0;
        while let Some(found) = letters.get(from..).and_then(|rest| rest.find(allowed)) {
            let at = from + found;
            if at <= start && start + length <= at + allowed.len() {
                return true;
            }
            from = at + 1;
        }
    }
    false
}

// Whether `word` has a bad word in it; with `marked`, flags each letter that belongs to one.
fn matches(word: &Word, mut marked: Option<&mut Vec<bool>>) -> bool {
    let l = lists();
    let letters = word.letters.as_str();
    if l.whole.contains(letters) {
        if let Some(marked) = marked {
            marked.iter_mut().for_each(|m| *m = true);
        }
        return true;
    }
    let mut found = false;
    let mut start = 0;
    while start + l.shortest_inside <= letters.len() {
        let mut length = l.shortest_inside;
        while length <= l.longest_inside && start + length <= letters.len() {
            if l.inside.contains(&letters[start..start + length]) && !covered(letters, start, length) {
                match marked.as_deref_mut() {
                    None => return true,
                    Some(marked) => {
                        found = true;
                        marked[start..start + length].iter_mut().for_each(|m| *m = true);
                    }
                }
            }
            length += 1;
        }
        start += 1;
    }
    found
}

pub fn contains_bad_words(text: &str) -> bool {
    words(text.as_bytes()).iter().any(|word| matches(word, None))
}

// `text` with each letter of every bad word replaced by '*'; same length, other text untouched.
pub fn mask_bad_words(text: &str) -> String {
    let mut out = text.as_bytes().to_vec();
    for word in words(text.as_bytes()) {
        let mut marked = vec![false; word.letters.len()];
        if !matches(&word, Some(&mut marked)) {
            continue;
        }
        for (i, &m) in marked.iter().enumerate() {
            if m {
                out[word.at[i]] = b'*';
            }
        }
    }
    // Only ASCII letters and look-alikes are replaced, so the text stays valid UTF-8.
    String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}
