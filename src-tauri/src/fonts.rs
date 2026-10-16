//! The installed font families offered in Settings → General → Appearance.
//!
//! Fonts are found with `fontdb`, which reads the font directories from the
//! fontconfig configuration (Linux) or scans the platform's font folders
//! (Windows, macOS). Inside Flatpak the host's fonts are mounted under
//! `/run/host`; fontconfig in the sandbox lists them as `remap-dir`s, which
//! fontdb doesn't read, so those directories are added here. If fontdb finds
//! nothing, `fc-list` is asked instead where it exists.
//!
//! On Linux the webview finds fonts through fontconfig, which leaves out
//! fonts its configuration rejects (`<selectfont><rejectfont>`: fonts
//! turned off in a font manager, or hidden by the distribution). fontdb
//! ignores those rules, so where `fc-list` runs, only the families
//! fontconfig also knows are listed.
//!
//! The list is built once per run, off the main thread.

use std::collections::HashSet;

use cumulas_core::model::is_valid_font_family;
use tokio::sync::OnceCell;

/// Where Flatpak mounts the host's system, local and user font directories.
#[cfg(target_os = "linux")]
const FLATPAK_HOST_FONT_DIRS: [&str; 3] = [
    "/run/host/fonts",
    "/run/host/local-fonts",
    "/run/host/user-fonts",
];

/// Everyday letters of the world's major scripts. A family is listed when
/// it draws every letter of at least one of these runs; fonts that only draw
/// symbols, emoji, math, private-use or a few stray letters (OpenSymbol has
/// the Greek alphabet for formulas, but no accented Greek) are left out.
const SCRIPT_SAMPLES: &[&str] = &[
    "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ",
    "абвгдежзийклмнопрстуфхцчшщъыьэюя",
    "αβγδεζηθικλμνξοπρστυφχψωάέήίόύώ",
    "աբգդեզէըթժիլխծկ",
    "אבגדהוזחטיכלמנסעפצקרשת",
    "ابتثجحخدذرزسشصضطظعغفقكلمنهوي",
    "ܐܒܓܕܗܘܙ",
    "ހށނރބ",
    "ߊߋߌߍߎ",
    "अआइईउकखगघचजटडतदनपबमयरलवसह",
    "অআইকখগঘচজটতদনপবমযরলসহ",
    "ਅਆਇਕਖਗਚਜਟਤਦਨਪਬਮਰਲਸਹ",
    "અઆઇકખગચજટતદનપબમયરલસહ",
    "ଅଆଇକଖଗଚଜଟତଦନପବମରଲସହ",
    "அஆஇகஙசஞடணதநபமயரலவ",
    "అఆఇకఖగచజటతదనపబమయరలసహ",
    "ಅಆಇಕಖಗಚಜಟತದನಪಬಮಯರಲಸಹ",
    "അആഇകഖഗചജടതദനപബമയരലസഹ",
    "අආඉකගචජටතදනපබමයරලසහ",
    "กขคงจฉชซญดตถทนบปผพฟมยรลวสหอ",
    "ກຂຄງຈຊຍດຕຖທນບປຜຝພຟມຢຣລວສຫອ",
    "ཀཁགངཅཆཇཉཏཐདནཔཕབམ",
    "ကခဂဃငစဆဇညတထဒနပဖဗဘမ",
    "აბგდევზთიკლმნოპ",
    "ሀለሐመሠረሰሸቀበተቸኀነኘአ",
    "ᎠᎡᎢᎣᎤᎥᎦᎧ",
    "ᐁᐃᐅᐊᐯᐱᐳᐸ",
    "កខគឃងចឆជឈញដឋឌឍណ",
    "ᠠᠡᠢᠣᠤᠥᠦᠧ",
    "ⴰⴱⴳⴷⴹⴻⴼⴽ",
    "一二三人大中日本国的是不了",
    "あいうえおかきくけこ",
    "アイウエオカキクケコ",
    "가나다라마바사아자차카타파하",
    "ꀀꀁꀂꀃꀄ",
    "ꔀꔁꔂꔃꔄ",
    "ꦄꦆꦈꦏꦒꦔ",
];

/// Whether the webview picks fonts through fontconfig (WebKitGTK does; the
/// Windows and macOS webviews use the platform's own font lookup).
const WEBVIEW_USES_FONTCONFIG: bool = cfg!(not(any(windows, target_os = "macos")));

static FAMILIES: OnceCell<Vec<String>> = OnceCell::const_new();

/// The installed font families: sorted, without duplicates, hidden system
/// faces or symbol-only fonts. Scanned on first use, then cached.
pub async fn installed_families() -> Vec<String> {
    FAMILIES
        .get_or_init(|| async {
            tauri::async_runtime::spawn_blocking(scan)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!("could not list fonts: {e}");
                    Vec::new()
                })
        })
        .await
        .clone()
}

fn scan() -> Vec<String> {
    let started = std::time::Instant::now();
    let mut families = scan_fontdb();
    if families.is_empty() {
        families = scan_fc_list();
    } else if WEBVIEW_USES_FONTCONFIG && let Some(known) = fontconfig_families() {
        families = known_to_fontconfig(families, &known);
    }
    tracing::debug!(
        "found {} font families in {:?}",
        families.len(),
        started.elapsed()
    );
    families
}

fn scan_fontdb() -> Vec<String> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    #[cfg(target_os = "linux")]
    for dir in FLATPAK_HOST_FONT_DIRS {
        let dir = std::path::Path::new(dir);
        let loaded = db.faces().any(|face| match &face.source {
            fontdb::Source::File(path) | fontdb::Source::SharedFile(path, _) => {
                path.starts_with(dir)
            }
            fontdb::Source::Binary(_) => false,
        });
        if dir.is_dir() && !loaded {
            db.load_fonts_dir(dir);
        }
    }

    // A family is listed once one of its faces draws letters.
    let mut decided = HashSet::new();
    let mut names = Vec::new();
    for face in db.faces() {
        let Some((family, _)) = face.families.first() else {
            continue;
        };
        let key = family.to_lowercase();
        if decided.contains(&key) {
            continue;
        }
        if !is_listable(family) {
            decided.insert(key);
            continue;
        }
        let draws_letters = db
            .with_face_data(face.id, |data, index| {
                ttf_parser::Face::parse(data, index)
                    .is_ok_and(|f| covers_letters(|c| f.glyph_index(c).is_some()))
            })
            .unwrap_or(false);
        if draws_letters {
            decided.insert(key);
            names.push(family.clone());
        }
    }
    tidy(names)
}

/// Asks fontconfig, for systems where fontdb finds no fonts.
fn scan_fc_list() -> Vec<String> {
    let output = std::process::Command::new("fc-list")
        .arg("--format=%{family[0]}\t%{lang}\n")
        .output();
    match output {
        Ok(out) if out.status.success() => parse_fc_list(&String::from_utf8_lossy(&out.stdout)),
        Ok(out) => {
            tracing::warn!("fc-list failed: {}", out.status);
            Vec::new()
        }
        Err(e) => {
            tracing::debug!("fc-list unavailable: {e}");
            Vec::new()
        }
    }
}

/// Parses `fc-list --format='%{family[0]}\t%{lang}\n'`. fontconfig gives a
/// font no languages when it has no letters (symbol fonts), or only the
/// `und-*` pseudo-languages (`und-zsye` for emoji).
fn parse_fc_list(output: &str) -> Vec<String> {
    let names = output.lines().filter_map(|line| {
        let (family, langs) = line.split_once('\t')?;
        let speaks = langs
            .split('|')
            .any(|lang| !lang.is_empty() && !lang.starts_with("und-"));
        speaks.then(|| family.to_string())
    });
    tidy(names)
}

/// Every family name fontconfig will match, lowercased: all the names of
/// each font it lists (a font can have localized names before its English
/// one), or None when `fc-list` is missing or lists nothing.
fn fontconfig_families() -> Option<HashSet<String>> {
    let output = std::process::Command::new("fc-list")
        .arg("--format=%{[]family{%{family}\n}}")
        .output();
    match output {
        Ok(out) if out.status.success() => {
            let known = parse_fc_families(&String::from_utf8_lossy(&out.stdout));
            (!known.is_empty()).then_some(known)
        }
        Ok(out) => {
            tracing::warn!("fc-list failed: {}", out.status);
            None
        }
        Err(e) => {
            tracing::debug!("fc-list unavailable: {e}");
            None
        }
    }
}

/// Parses `fc-list --format='%{[]family{%{family}\n}}'`: one family name
/// per line.
fn parse_fc_families(output: &str) -> HashSet<String> {
    output
        .lines()
        .filter(|name| !name.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// `families` without those fontconfig doesn't know: the webview can't
/// find them. Font matching ignores case.
fn known_to_fontconfig(mut families: Vec<String>, known: &HashSet<String>) -> Vec<String> {
    families.retain(|family| known.contains(&family.to_lowercase()));
    families
}

/// Whether a face mapping these characters can set text in some script.
fn covers_letters(maps: impl Fn(char) -> bool) -> bool {
    SCRIPT_SAMPLES
        .iter()
        .any(|sample| sample.chars().all(&maps))
}

/// Families shown to people: valid names that aren't hidden. Names starting
/// with "." are macOS's private system faces; "@" marks Windows' vertical
/// variants of CJK fonts.
fn is_listable(name: &str) -> bool {
    is_valid_font_family(name)
        && !name.starts_with('.')
        && !name.starts_with('@')
        && !name.eq_ignore_ascii_case("LastResort")
}

/// Listable names, without case-insensitive duplicates (font matching
/// ignores case), sorted case-insensitively.
fn tidy(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out: Vec<String> = names
        .into_iter()
        .filter(|name| is_listable(name) && seen.insert(name.to_lowercase()))
        .collect();
    out.sort_by_cached_key(|name| (name.to_lowercase(), name.clone()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn tidy_sorts_dedupes_and_drops_hidden_and_invalid_names() {
        let names = strings(&[
            "liberation serif",
            "DejaVu Sans",
            "Cantarell",
            "Liberation Serif",
            ".SF NS",
            "@MS Gothic",
            "LastResort",
            "",
            "Bad\nName",
            " Padded",
            "Noto Sans",
            "Noto Sans Arabic",
            "文泉驿正黑",
        ]);
        assert_eq!(
            tidy(names),
            strings(&[
                "Cantarell",
                "DejaVu Sans",
                "liberation serif",
                "Noto Sans",
                "Noto Sans Arabic",
                "文泉驿正黑",
            ])
        );
    }

    #[test]
    fn fonts_need_the_letters_of_some_script() {
        let covers = |ranges: &[(char, char)]| {
            covers_letters(|c| ranges.iter().any(|&(lo, hi)| (lo..=hi).contains(&c)))
        };
        // Latin; Arabic only; CJK and kana only; Devanagari only.
        assert!(covers(&[('A', 'Z'), ('a', 'z')]));
        assert!(covers(&[('\u{0600}', '\u{06FF}')]));
        assert!(covers(&[
            ('\u{3040}', '\u{30FF}'),
            ('\u{4E00}', '\u{9FFF}')
        ]));
        assert!(covers(&[('\u{0900}', '\u{097F}')]));
        // Lowercase Latin only, or Greek without accents (OpenSymbol): not enough.
        assert!(!covers(&[('a', 'z')]));
        assert!(!covers(&[
            ('\u{0391}', '\u{03A9}'),
            ('\u{03B1}', '\u{03C9}')
        ]));
        // Symbols, emoji, digits and private-use glyphs only.
        assert!(!covers(&[
            ('0', '9'),
            ('\u{2190}', '\u{21FF}'),
            ('\u{E000}', '\u{F8FF}'),
            ('\u{1F300}', '\u{1F64F}'),
        ]));
        assert!(!covers_letters(|_| false));
    }

    #[test]
    fn fc_list_output_keeps_fonts_with_languages() {
        let output = "DejaVu Sans\taa|ab|af|ar\n\
                      Noto Color Emoji\tund-zsye\n\
                      OpenSymbol\t\n\
                      Unifont-JP\taa|ja\n\
                      DejaVu Sans\taa|ab\n\
                      .Hidden\ten\n\
                      no tab here\n\
                      IPAPGothic\taa|en|ja\n";
        assert_eq!(
            parse_fc_list(output),
            strings(&["DejaVu Sans", "IPAPGothic", "Unifont-JP"])
        );
    }

    #[test]
    fn fontconfig_names_every_family_it_matches() {
        // Each name of each font on its own line, localized names included.
        let output = "Inter Display\n\
                      Inter Display Light\n\
                      文泉驛點陣正黑\n\
                      WenQuanYi Zen Hei Sharp\n\
                      \n\
                      DejaVu Sans\n\
                      dejavu sans\n\
                      \u{FEFF}Bom Serif\n";
        assert_eq!(
            parse_fc_families(output),
            HashSet::from(
                [
                    "inter display",
                    "inter display light",
                    "文泉驛點陣正黑",
                    "wenquanyi zen hei sharp",
                    "dejavu sans",
                    "\u{FEFF}bom serif",
                ]
                .map(String::from)
            )
        );
    }

    #[test]
    fn families_fontconfig_rejects_are_left_out() {
        // fontconfig lists every font but Carlito (a <rejectfont> rule).
        let known = parse_fc_families(
            "Caladea\nDejaVu Sans\n文泉驛點陣正黑\nWenQuanYi Zen Hei Sharp\n\u{FEFF}Bom Serif\n",
        );
        let families = strings(&[
            "Caladea",
            "Carlito",
            "dejavu sans",
            "WenQuanYi Zen Hei Sharp",
            "\u{FEFF}Bom Serif",
            "Bom Serif",
        ]);
        assert_eq!(
            known_to_fontconfig(families, &known),
            strings(&[
                "Caladea",
                "dejavu sans",
                "WenQuanYi Zen Hei Sharp",
                "\u{FEFF}Bom Serif",
            ])
        );
    }

    #[test]
    fn the_scan_returns_a_clean_list() {
        // Whatever this machine has installed, the list is tidy.
        let families = scan();
        assert_eq!(families, tidy(families.clone()));
        assert!(families.iter().all(|f| is_valid_font_family(f)));
    }
}
