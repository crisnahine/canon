//! Files that are two languages at once.
//!
//! An `.erb` template is Ruby inside HTML; a `.vue` component is TypeScript
//! inside markup. One grammar cannot parse either, which is why both were
//! declared unwired rather than half-wired, and why 339 `.erb` files in a real
//! Rails repository were invisible.
//!
//! tree-sitter's answer is [`Parser::set_included_ranges`]: parse the same
//! buffer with one grammar, but tell the parser to consider only certain byte
//! ranges of it. Everything outside those ranges is treated as though it were
//! not there, and the resulting tree carries the original byte offsets, so a
//! line number from the tree still points at the right line of the file.
//!
//! The ranges themselves come from scanning for the host language's delimiters
//! rather than from parsing the host language. That is a deliberate trade: a
//! real HTML parse would be more correct about pathological markup, and would
//! also mean carrying two more grammars to extract facts from the half of the
//! file canon has no conventions about.

use tree_sitter::{Point, Range};

use crate::Language;

/// The embedded language in a file, and where it lives.
pub(crate) struct Embedded {
    /// The language the extracted ranges are written in.
    pub(crate) language: Language,
    /// Byte ranges of that language within the file.
    pub(crate) ranges: Vec<Range>,
}

/// What is embedded in a file of this kind, if anything.
pub(crate) fn embedded_in(language: Language, source: &str) -> Option<Embedded> {
    match language {
        Language::Erb => {
            let ranges = delimited(source, &[("<%=", "%>"), ("<%", "%>")]);
            (!ranges.is_empty()).then_some(Embedded { language: Language::Ruby, ranges })
        }
        Language::Vue => {
            // The script block only. `<template>` is markup, and canon has no
            // conventions about markup; `<style>` is CSS, which is a different
            // engine entirely.
            let ranges = script_blocks(source);
            (!ranges.is_empty()).then_some(Embedded { language: script_language(source), ranges })
        }
        _ => None,
    }
}

/// Which language a Vue component's script block is written in.
///
/// `lang="ts"` is explicit; a component without it is JavaScript, which is
/// what the Vue documentation says and what the tooling assumes.
fn script_language(source: &str) -> Language {
    let Some(start) = source.find("<script") else { return Language::JavaScript };
    let rest = source.get(start..).unwrap_or_default();
    let tag = rest.split_once('>').map_or(rest, |(t, _)| t);
    if tag.contains("ts") { Language::TypeScript } else { Language::JavaScript }
}

/// Ranges between `<script ...>` and `</script>`.
fn script_blocks(source: &str) -> Vec<Range> {
    let mut ranges = Vec::new();
    let mut cursor = 0;
    while let Some(open) = source[cursor..].find("<script") {
        let tag_start = cursor + open;
        let Some(tag_end) = source[tag_start..].find('>') else { break };
        let body_start = tag_start + tag_end + 1;
        let Some(close) = source[body_start..].find("</script") else { break };
        let body_end = body_start + close;
        if body_end > body_start {
            ranges.push(range_of(source, body_start, body_end));
        }
        cursor = body_end;
    }
    ranges
}

/// Ranges between each pair of delimiters, longest opener first.
///
/// `<%=` has to be tried before `<%`, or every printing tag is read as a
/// plain one and the `=` becomes part of the Ruby, which parses as a syntax
/// error at the start of every expression in the file.
fn delimited(source: &str, pairs: &[(&str, &str)]) -> Vec<Range> {
    let mut ranges = Vec::new();
    let mut cursor = 0;
    while cursor < source.len() {
        let next = pairs
            .iter()
            .filter_map(|(open, close)| {
                source[cursor..].find(open).map(|at| (cursor + at, *open, *close))
            })
            .min_by_key(|(at, open, _)| (*at, std::cmp::Reverse(open.len())));

        let Some((at, open, close)) = next else { break };
        // At the same offset the longer opener wins, so `<%=` is not read as
        // `<%` followed by a stray `=`.
        let open = pairs
            .iter()
            .filter(|(o, _)| source[at..].starts_with(*o))
            .max_by_key(|(o, _)| o.len())
            .map_or(open, |(o, _)| *o);

        let body_start = at + open.len();
        let Some(end) = source[body_start..].find(close) else { break };
        let body_end = body_start + end;

        // `<%# a comment %>` and `<% end %>` are Ruby, and both parse, so they
        // are kept: a range that is only whitespace is not.
        if source[body_start..body_end].trim().is_empty() {
            cursor = body_end + close.len();
            continue;
        }
        ranges.push(range_of(source, body_start, body_end));
        cursor = body_end + close.len();
    }
    ranges
}

/// A byte range with the row and column the parser needs.
///
/// Computed against the whole file rather than the fragment, so a line number
/// from the resulting tree points at the right line of the original.
fn range_of(source: &str, start: usize, end: usize) -> Range {
    Range {
        start_byte: start,
        end_byte: end,
        start_point: point_at(source, start),
        end_point: point_at(source, end),
    }
}

fn point_at(source: &str, byte: usize) -> Point {
    let before = source.get(..byte).unwrap_or("");
    let row = before.matches('\n').count();
    let column = before.rsplit_once('\n').map_or(before.len(), |(_, last)| last.len());
    Point { row, column }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ruby_ranges(source: &str) -> Vec<String> {
        embedded_in(Language::Erb, source)
            .map(|e| {
                e.ranges
                    .iter()
                    .map(|r| source[r.start_byte..r.end_byte].trim().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn erb_yields_the_ruby_between_its_tags() {
        let source =
            "<div>\n  <%= user.name %>\n  <% if admin? %>\n    <p>hi</p>\n  <% end %>\n</div>\n";
        assert_eq!(ruby_ranges(source), vec!["user.name", "if admin?", "end"]);
    }

    #[test]
    fn a_printing_tag_is_not_read_as_a_plain_one() {
        // `<%=` must beat `<%` at the same offset, or the `=` becomes Ruby and
        // every expression in the file starts with a syntax error.
        assert_eq!(ruby_ranges("<%= 1 + 1 %>"), vec!["1 + 1"]);
    }

    #[test]
    fn an_empty_tag_contributes_nothing() {
        assert!(ruby_ranges("<%  %>").is_empty());
    }

    #[test]
    fn a_template_with_no_ruby_at_all_is_not_embedded() {
        assert!(embedded_in(Language::Erb, "<p>just markup</p>").is_none());
    }

    #[test]
    fn an_unterminated_tag_does_not_hang_or_panic() {
        for bad in ["<%", "<%=", "<% unterminated", "<%%>", "<%=<%=<%="] {
            let _ = ruby_ranges(bad);
        }
    }

    #[test]
    fn a_vue_script_block_is_typescript_when_it_says_so() {
        let source = "<template><div/></template>\n<script lang=\"ts\">\nexport class A { call() {} }\n</script>\n";
        let e = embedded_in(Language::Vue, source).expect("a script block");
        assert_eq!(e.language, Language::TypeScript);
        assert!(source[e.ranges[0].start_byte..e.ranges[0].end_byte].contains("export class A"));
    }

    #[test]
    fn a_vue_script_block_without_a_lang_is_javascript() {
        let source = "<script>\nexport const a = 1;\n</script>\n";
        assert_eq!(
            embedded_in(Language::Vue, source).expect("a block").language,
            Language::JavaScript
        );
    }

    #[test]
    fn a_vue_file_with_only_markup_is_not_embedded() {
        assert!(embedded_in(Language::Vue, "<template><div/></template>").is_none());
    }

    #[test]
    fn positions_point_at_the_original_file() {
        // A range carrying the wrong row would put every reported line number
        // in an embedded file on line one.
        let source = "<div>\n<% helper %>\n</div>\n";
        let e = embedded_in(Language::Erb, source).expect("a range");
        assert_eq!(e.ranges[0].start_point.row, 1, "the Ruby is on the second line");
    }

    #[test]
    fn a_language_with_nothing_embedded_says_so() {
        assert!(embedded_in(Language::Ruby, "class A; end").is_none());
    }
}
